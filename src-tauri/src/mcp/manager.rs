//! The running side: which servers are connected, and what they offer.
//!
//! # Why the state is process-global
//!
//! The tool list has to be readable from [`crate::ai::tools::registry_for_with`],
//! which is a plain function called while a request is being built -- it has no
//! runtime, no app handle and no database in scope. The workspace root solved the
//! same problem the same way (`tools/workspace.rs` holds it in a `OnceLock`), and
//! a second mechanism for the second global would be two ways to read one kind of
//! state.
//!
//! # Two locks, one job each
//!
//! Connections are async and live behind a `tokio` mutex; the snapshot the
//! registry reads, and the last result the Settings page reads, are plain data
//! behind `std` mutexes. They are separate because they are read at different
//! moments by different things: building a request must not be able to wait on a
//! server, and the settings page must not start one.
//!
//! # When it refreshes
//!
//! At startup, and after every change the Settings page makes. Nothing refreshes
//! on a timer: a server's tool list changes when someone installs a new version
//! of it, which this app cannot observe, and polling every server every minute to
//! learn nothing is a cost with no return.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::ai::tools::DynamicTool;
use crate::database::Database;

use super::client::{Connection, RemoteTool};
use super::{load, McpServer, McpServerView, McpToolSummary};

/// The tools the model may call, as of the last refresh.
fn snapshot_cell() -> &'static Mutex<Arc<Vec<DynamicTool>>> {
    static CELL: OnceLock<Mutex<Arc<Vec<DynamicTool>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Arc::new(Vec::new())))
}

/// What each server offered the last time it was reached, and how it failed.
///
/// Kept apart from the configuration so a page can be drawn without connecting to
/// anything: a row shows the server as configured, plus what it said when it was
/// last asked.
fn known_cell() -> &'static Mutex<HashMap<String, (Vec<McpToolSummary>, String)>> {
    static CELL: OnceLock<Mutex<HashMap<String, (Vec<McpToolSummary>, String)>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The live connections, by server id.
fn connections() -> &'static tokio::sync::Mutex<HashMap<String, Connection>> {
    static CELL: OnceLock<tokio::sync::Mutex<HashMap<String, Connection>>> = OnceLock::new();
    CELL.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// The current tool set, for building a request.
///
/// A clone of an `Arc`, so this costs nothing and cannot block: the registry may
/// call it on every turn.
pub fn snapshot() -> Arc<Vec<DynamicTool>> {
    snapshot_cell()
        .lock()
        .map(|cell| Arc::clone(&cell))
        .unwrap_or_else(|_| Arc::new(Vec::new()))
}

/// Every server as configured, with what it said last time it answered.
///
/// Reads no process and connects to nothing, so the Settings page can draw itself
/// the moment it opens. [`refresh`] is what goes and asks.
pub fn views(database: &Database) -> Vec<McpServerView> {
    let known = known_cell()
        .lock()
        .map(|cell| cell.clone())
        .unwrap_or_default();
    load(database)
        .into_iter()
        .map(|server| {
            let (tools, error) = known.get(&server.id).cloned().unwrap_or_default();
            McpServerView {
                server,
                tools,
                error,
            }
        })
        .collect()
}

/// Connects to every enabled server, rebuilds the tool set, and reports what
/// happened.
///
/// A server that cannot be reached is not fatal to the others: it comes back with
/// its error attached and no tools, so one broken entry in Settings does not cost
/// the user the servers that do work.
pub async fn refresh(database: &Database) -> Vec<McpServerView> {
    let servers = load(database);
    let mut live = connections().lock().await;

    // A server that was switched off or deleted keeps no process. Doing this
    // first means a disabled server is not merely unreachable but gone.
    live.retain(|id, _| {
        servers
            .iter()
            .any(|server| &server.id == id && server.enabled)
    });

    let mut views = Vec::new();
    let mut built = Vec::new();
    let mut known = HashMap::new();

    for server in &servers {
        let (remote, error) = if server.enabled {
            match reach(&mut live, server).await {
                Ok(remote) => (remote, String::new()),
                Err(error) => {
                    // The connection is dropped rather than kept: whatever is in
                    // the map answered nothing, and reusing it would repeat the
                    // same silence.
                    live.remove(&server.id);
                    (Vec::new(), error)
                }
            }
        } else {
            (Vec::new(), String::new())
        };

        let summaries = super::tools::summaries(server, &remote);
        built.extend(super::tools::build(server, &remote));
        known.insert(server.id.clone(), (summaries.clone(), error.clone()));
        views.push(McpServerView {
            server: server.clone(),
            tools: summaries,
            error,
        });
    }

    if let Ok(mut cell) = snapshot_cell().lock() {
        *cell = Arc::new(built);
    }
    if let Ok(mut cell) = known_cell().lock() {
        *cell = known;
    }
    views
}

/// Calls one remote tool, connecting again if the server is not up.
///
/// The connection is not guaranteed to still be in the map: a refresh drops the
/// ones that failed, and a server that died since the last refresh is only
/// discovered here.
pub async fn call(
    server_id: &str,
    remote_name: &str,
    arguments: Value,
    database: Option<&Database>,
) -> Result<String, String> {
    let Some(database) = database else {
        return Err("this call has no database, so no MCP server can be resolved".to_string());
    };
    let Some(server) = load(database)
        .into_iter()
        .find(|server| server.id == server_id && server.enabled)
    else {
        return Err(format!(
            "the MCP server behind `{remote_name}` is no longer configured or is switched off"
        ));
    };

    let mut live = connections().lock().await;
    if !live.contains_key(server_id) {
        live.insert(
            server_id.to_string(),
            Connection::start(&server)
                .await
                .map_err(|error| format!("could not reach the {} server: {error}", server.name))?,
        );
    }
    live.get_mut(server_id)
        .expect("just inserted")
        .call_tool(remote_name, arguments)
        .await
}

/// Drops a connection, so a server that was removed or switched off does not keep
/// running with no way to see it.
pub async fn forget(server_id: &str) {
    connections().lock().await.remove(server_id);
}

/// Reaches a server, starting it if this is the first time.
///
/// A failure on a connection that already existed is retried once with a fresh
/// process: a server that has been up since startup and stopped answering is far
/// more likely to have died than to be broken.
async fn reach(
    live: &mut HashMap<String, Connection>,
    server: &McpServer,
) -> Result<Vec<RemoteTool>, String> {
    let mut last = String::new();
    for attempt in 0..2 {
        if !live.contains_key(&server.id) {
            live.insert(server.id.clone(), Connection::start(server).await?);
        }
        match live
            .get_mut(&server.id)
            .expect("just inserted")
            .list_tools()
            .await
        {
            Ok(tools) => return Ok(tools),
            Err(error) => last = error,
        }
        live.remove(&server.id);
        if attempt == 0 {
            continue;
        }
    }
    Err(last)
}

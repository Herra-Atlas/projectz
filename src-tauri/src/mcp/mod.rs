//! Model Context Protocol servers.
//!
//! # What this is
//!
//! An MCP server is a program that offers tools over a small JSON-RPC protocol --
//! a filesystem, a database, a design tool, whatever the user points it at. This
//! module runs those programs, asks them what they can do, and hands the answers
//! to the agent as ordinary tools.
//!
//! # Where the configuration lives
//!
//! One settings row, `app.mcp_servers`, holding the whole list. The written shape
//! and the stored shape are the same object rather than a table with a migration
//! behind it, because a server is five fields and a new one should not cost a
//! schema version.
//!
//! # Where the tools come from
//!
//! Nothing here is known at compile time: the names and schemas arrive from the
//! server at runtime, so they cannot be a [`crate::ai::tools::ToolSpec`], whose
//! fields are `&'static str`. They are [`crate::ai::tools::DynamicTool`]s
//! instead, which carry owned text and a closure that knows which server and
//! which remote tool it speaks for. See [`manager`] for the connection lifetime
//! and [`tools`] for how a remote tool becomes a callable one.

pub mod client;
pub mod manager;
pub mod tools;

use serde::{Deserialize, Serialize};

use crate::database::Database;

/// The settings key holding every configured server.
const SERVERS_KEY: &str = "app.mcp_servers";

/// One configured MCP server.
///
/// `command` plus `args` is exactly how the user would start it in a terminal,
/// which is the only description of a server that does not need a schema of its
/// own: `npx -y @modelcontextprotocol/server-filesystem C:\work` and a compiled
/// binary are the same shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpServer {
    /// Stable id, generated once. Renaming a server does not orphan its row.
    pub id: String,
    /// Shown in Settings and used to name the tools the model sees.
    pub name: String,
    /// The program to run. Resolved on `PATH`, like any other command.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the child process, as a list so the form can
    /// keep the order the user typed.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// The credential for this server, or empty.
    ///
    /// One field doing two jobs, because a user has one secret either way. A bare
    /// value is handed to a command as `MCP_API_KEY`, which is what a server that
    /// needs one almost always reads; `NAME=value` sets that name instead, for a
    /// server that documents a different variable. Over HTTP the bare value is
    /// the bearer token. It lives in the same settings row as the rest of the
    /// server: this is the app's own local database, not a shared configuration.
    #[serde(default)]
    pub key: String,
    /// Whether the agent may use this server's tools at all.
    #[serde(default)]
    pub enabled: bool,
}

/// One tool a server advertised, named the way the model calls it.
#[derive(Clone, Debug, Serialize)]
pub struct McpToolSummary {
    /// The name in the request body, e.g. `mcp__files__read_file`.
    pub name: String,
    /// The server's own name for it, which is what `tools/call` takes.
    pub remote_name: String,
    pub description: String,
}

/// A server as the Settings page sees it: the configuration, what it offered
/// last time it answered, and why it did not answer.
#[derive(Clone, Debug, Serialize)]
pub struct McpServerView {
    #[serde(flatten)]
    pub server: McpServer,
    /// Empty while a server is switched off or has never been reached.
    pub tools: Vec<McpToolSummary>,
    /// The last failure, or empty. Shown on the row rather than only logged,
    /// because a server that does not start is the one thing a user has to fix.
    pub error: String,
}

/// Every configured server, in the order the user arranged them.
pub fn load(database: &Database) -> Vec<McpServer> {
    database
        .setting::<Vec<McpServer>>(SERVERS_KEY)
        .unwrap_or_default()
}

/// Writes the whole list back.
pub fn save(database: &Database, servers: &[McpServer]) -> Result<(), String> {
    database.set_setting(SERVERS_KEY, &servers.to_vec())
}

/// A fresh id for a server being added.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The first `http://` or `https://` address in a command line, if there is one.
///
/// A server that *is* a URL is the obvious case; the scan also catches one
/// configured with an address rather than as one -- `mcp-server-foo --url
/// https://foo.example/mcp` -- which is somewhere an icon can come from and
/// otherwise would not. Which scheme appears first in the line is what wins,
/// rather than `http` always being preferred.
pub fn url_in(command: &str) -> Option<&str> {
    let start = ["http://", "https://"]
        .iter()
        .filter_map(|prefix| command.find(prefix))
        .min()?;
    let rest = &command[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    // A path pasted inside prose arrives with whatever followed it.
    Some(rest[..end].trim_end_matches([',', ')', '"', '\'', '.']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_that_is_a_url_has_one() {
        assert_eq!(
            url_in("https://example.com/mcp"),
            Some("https://example.com/mcp")
        );
        assert_eq!(
            url_in("http://localhost:8931/mcp"),
            Some("http://localhost:8931/mcp")
        );
    }

    #[test]
    fn an_address_inside_the_arguments_is_found() {
        assert_eq!(
            url_in("npx -y server-foo --url https://foo.example/mcp --token t"),
            Some("https://foo.example/mcp")
        );
    }

    /// Whichever scheme comes first in the line, not whichever is more common.
    #[test]
    fn the_earliest_address_wins() {
        assert_eq!(
            url_in("--a https://first.example --b http://second.example"),
            Some("https://first.example")
        );
        assert_eq!(
            url_in("--a http://first.example --b https://second.example"),
            Some("http://first.example")
        );
    }

    /// A command line is pasted from anywhere, punctuation and all.
    #[test]
    fn trailing_punctuation_is_not_part_of_the_address() {
        assert_eq!(
            url_in("see https://example.com/mcp."),
            Some("https://example.com/mcp")
        );
        assert_eq!(
            url_in("(https://example.com/mcp)"),
            Some("https://example.com/mcp")
        );
    }

    #[test]
    fn a_program_has_no_address() {
        assert_eq!(
            url_in("npx -y @modelcontextprotocol/server-filesystem C:\\work"),
            None
        );
        assert_eq!(url_in(""), None);
    }
}

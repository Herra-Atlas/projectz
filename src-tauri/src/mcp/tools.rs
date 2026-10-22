//! Turning what a server advertises into a tool the model can call.
//!
//! # The name
//!
//! `mcp__<server>__<tool>`, slugged to what a function name may contain. The
//! prefix is doing three jobs at once:
//!
//! - It is how a call is routed back to a server without a lookup table, since
//!   the name a model uses is the whole address.
//! - It is what [`crate::ai::tools::access`] recognises when it decides what a
//!   narrowed run may touch.
//! - It tells a reader of a transcript where a tool came from, which no other
//!   tool's name does.
//!
//! # The effect
//!
//! Every MCP tool is a [`Effect::Write`], and that is a deliberate under-reading.
//! The protocol says nothing about whether a tool changes anything, and the two
//! mistakes are not the same size: calling a read tool a write costs the run its
//! cache, while calling a write tool a read tells the model a stale file is
//! current. The conservative direction is chosen in the dark and never guessed.

use std::sync::Arc;

use serde_json::json;

use crate::ai::tools::{DynamicTool, Effect};

use super::client::RemoteTool;
use super::{McpServer, McpToolSummary};

/// Longest a tool name may be for the providers that accept the smallest ones.
const MAX_NAME_CHARS: usize = 64;

/// What every MCP tool name starts with.
///
/// A constant rather than a literal in two places: [`crate::ai::tools::access`]
/// reads it to decide what a narrowed run may touch, so the prefix is a contract
/// between this module and the access set and not just a naming habit.
pub const TOOL_PREFIX: &str = "mcp__";

/// Builds the callable tools for one server.
///
/// A server that offers two tools whose names slug to the same string keeps the
/// first and drops the rest: two entries with one name would be one tool to the
/// model, and which of them a call reached would be luck.
pub fn build(server: &McpServer, remote: &[RemoteTool]) -> Vec<DynamicTool> {
    let mut seen: Vec<String> = Vec::new();
    let mut built = Vec::new();

    for tool in remote {
        let name = model_name(server, &tool.name);
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());

        let description = tool
            .description
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| format!("`{}` on the {} server", tool.name, server.name));
        let server_id = server.id.clone();
        let remote_name = tool.name.clone();

        built.push(DynamicTool {
            name,
            description: format!("[MCP · {}] {description}", server.name),
            // Passed through untouched. The schema is the server's contract with
            // the model, and rewriting it here would be this app guessing at an
            // interface it did not write.
            parameters: tool
                .input_schema
                .clone()
                .unwrap_or_else(|| json!({ "type": "object" })),
            effect: Effect::Write,
            execute: Arc::new(move |arguments, context| {
                let server_id = server_id.clone();
                let remote_name = remote_name.clone();
                Box::pin(async move {
                    super::manager::call(
                        &server_id,
                        &remote_name,
                        arguments,
                        context.database.as_deref(),
                    )
                    .await
                })
            }),
        });
    }
    built
}

/// What the Settings page lists for one server.
pub fn summaries(server: &McpServer, remote: &[RemoteTool]) -> Vec<McpToolSummary> {
    remote
        .iter()
        .map(|tool| McpToolSummary {
            name: model_name(server, &tool.name),
            remote_name: tool.name.clone(),
            description: tool.description.clone().unwrap_or_default(),
        })
        .collect()
}

/// The name the model calls, built from the server's own name and the tool's.
fn model_name(server: &McpServer, tool: &str) -> String {
    let name = format!("{TOOL_PREFIX}{}__{}", slug(&server.name), slug(tool));
    name.chars().take(MAX_NAME_CHARS).collect()
}

/// A name a function may hold: lowercase, alphanumerics and single underscores.
///
/// A server whose name is a sentence or a path is normal -- `Files (work)`,
/// `./servers/fs` -- and every character a function name cannot hold is folded
/// to one underscore rather than dropped, so two names that differ only there do
/// not collapse into one.
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> McpServer {
        McpServer {
            id: "id-1".into(),
            name: name.into(),
            command: "node".into(),
            args: vec![],
            env: vec![],
            key: String::new(),
            enabled: true,
        }
    }

    fn remote(name: &str) -> RemoteTool {
        RemoteTool {
            name: name.into(),
            description: Some("does a thing".into()),
            input_schema: Some(json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
            })),
        }
    }

    #[test]
    fn a_tool_is_named_for_its_server() {
        let built = build(&server("Files"), &[remote("read_file")]);
        assert_eq!(built[0].name, "mcp__files__read_file");
    }

    /// A server name is whatever the user typed, and the tool name has to survive
    /// it -- including when the two slug to something that is not a function name.
    #[test]
    fn a_name_a_function_cannot_hold_is_folded() {
        let built = build(&server("My Files (work)"), &[remote("read.file")]);
        assert_eq!(built[0].name, "mcp__my_files_work__read_file");
    }

    #[test]
    fn a_tool_with_no_schema_still_gets_a_permissive_one() {
        let built = build(
            &server("Files"),
            &[RemoteTool {
                name: "ping".into(),
                description: None,
                input_schema: None,
            }],
        );
        assert_eq!(built[0].parameters, json!({ "type": "object" }));
        // And it still says what it is, since the server did not.
        assert!(built[0].description.contains("ping"));
    }

    #[test]
    fn the_description_names_the_server_it_came_from() {
        let built = build(&server("Files"), &[remote("read_file")]);
        assert!(built[0].description.starts_with("[MCP · Files]"));
        assert!(built[0].description.contains("does a thing"));
    }

    /// Two remote tools that fold to one name would be one tool to the model, so
    /// the second is dropped rather than shadowing the first at random.
    #[test]
    fn tools_that_fold_to_one_name_keep_the_first() {
        let built = build(
            &server("Files"),
            &[remote("read.file"), remote("read/file")],
        );
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].name, "mcp__files__read_file");
    }

    /// Every provider that accepts tools at all accepts this length, and a server
    /// name is not bounded by anything a user can see when they type it.
    #[test]
    fn a_long_name_is_capped() {
        let long = "a".repeat(80);
        let built = build(&server(&long), &[remote(&long)]);
        assert_eq!(built[0].name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn summaries_carry_both_names() {
        let listed = summaries(&server("Files"), &[remote("read_file")]);
        assert_eq!(listed[0].name, "mcp__files__read_file");
        assert_eq!(listed[0].remote_name, "read_file");
    }
}

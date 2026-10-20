//! Reading one learning reply.
//!
//! The model answers with operations against the profile it was shown, never a
//! rewritten document. That is the difference between an edit and a re-invention:
//! an `update` names the row it changes, while a whole-document reply would let a
//! small model silently drop a fact nobody asked it to drop and would make two
//! runs with the same input produce two different profiles.
//!
//! # The reply is treated as untrusted
//!
//! A model wraps JSON in a fence, prefixes it with "Here is", or answers with
//! prose. All of that is normal. The parser therefore reads the first JSON value
//! rather than searching for the last brace, and a malformed operation is dropped
//! by the merge instead of failing the whole run -- one bad line should not cost
//! the facts that were fine.

use serde::Deserialize;
use serde_json::Value;

/// The reply as the model wrote it.
#[derive(Debug, Default)]
pub struct RawReply {
    /// Operations that could be read. Missing is the same as empty: a run may
    /// learn nothing.
    pub ops: Vec<RawOp>,
    /// One or two sentences for the run history. Not interpreted.
    pub report: String,
    /// Operations that could not be read. Counted rather than fatal: one bad
    /// line should not cost the facts beside it.
    pub ignored: usize,
}

/// One operation, before its section and text have been validated.
///
/// Fields default rather than being required so one missing key becomes one
/// ignored operation instead of an unreadable reply.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum RawOp {
    Add {
        #[serde(default)]
        section: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        confidence: Option<f32>,
    },
    Update {
        #[serde(default)]
        id: i64,
        #[serde(default)]
        text: String,
        #[serde(default)]
        confidence: Option<f32>,
    },
    Reinforce {
        #[serde(default)]
        id: i64,
        #[serde(default)]
        confidence: Option<f32>,
    },
    Remove {
        #[serde(default)]
        id: i64,
        /// Why the model removed it. Kept on the wire so the field is not rejected;
        /// the merge does not need it, and the run report carries the explanation.
        #[serde(default, rename = "reason")]
        _reason: Option<String>,
    },
}

/// Reads one reply.
///
/// A bare array is accepted as a list of operations, because that is a shape a
/// model produces when it reads too much of the operation schema and not enough
/// of the envelope. The report is then empty rather than the reply being thrown
/// away.
pub fn parse_reply(raw: &str) -> Result<RawReply, String> {
    let value =
        read_first_json(raw).ok_or_else(|| "the reply contained no JSON value".to_string())?;

    // A bare array is a list of operations. Some models read the schema and not
    // the envelope; the operations are still usable and the report is simply absent.
    let (ops_value, report) = match value {
        Value::Array(ops) => (Some(Value::Array(ops)), String::new()),
        Value::Object(object) => {
            // Neither key is not a decline: it is the model ignoring the format.
            // That is reported rather than read as "nothing to learn", which is
            // the one outcome that hides a completely unreadable answer.
            if !object.contains_key("ops") && !object.contains_key("report") {
                return Err("the reply named neither `ops` nor `report`".to_string());
            }
            let report = object
                .get("report")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            (object.get("ops").cloned(), report)
        }
        _ => return Err("the reply was not a JSON object".to_string()),
    };

    let mut reply = RawReply {
        report,
        ..RawReply::default()
    };
    let Some(ops_value) = ops_value else {
        return Ok(reply);
    };
    let ops = ops_value
        .as_array()
        .ok_or_else(|| "`ops` was not an array".to_string())?;
    for operation in ops {
        match serde_json::from_value::<RawOp>(operation.clone()) {
            Ok(operation) => reply.ops.push(operation),
            Err(_) => reply.ignored += 1,
        }
    }
    Ok(reply)
}

/// The first JSON value in a reply, ignoring whatever surrounds it.
///
/// **The first value, not the first brace to the last one.** A model that repeats
/// its object, or prints a closing tag and the object again, would otherwise have
/// two correct replies joined into text that is not JSON at all.
fn read_first_json(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find(|character: char| character == '{' || character == '[')?;
    serde_json::Deserializer::from_str(&trimmed[start..])
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_object_with_operations_and_a_report() {
        let reply = parse_reply(
            r#"{"ops":[{"op":"reinforce","id":3},{"op":"add","section":"work","text":"Prefers Rust (programming language) for backend services.","confidence":0.9}],"report":"Reinforced Rust."}"#,
        )
        .expect("reply");
        assert_eq!(reply.ops.len(), 2);
        assert_eq!(reply.report, "Reinforced Rust.");
    }

    /// A code fence and a sentence in front are normal model decoration, not a
    /// different format.
    #[test]
    fn reads_through_a_fence_and_a_preamble() {
        let reply = parse_reply(
            "Here is the result:\n```json\n{\"ops\":[],\"report\":\"Nothing new.\"}\n```",
        )
        .expect("reply");
        assert!(reply.ops.is_empty());
        assert_eq!(reply.report, "Nothing new.");
    }

    /// Repeating the object must not join two copies into unparseable text: the
    /// first value is the answer, and the rest is decoration.
    #[test]
    fn reads_the_first_value_when_the_model_repeats_itself() {
        let reply = parse_reply(
            "{\"ops\":[],\"report\":\"First.\"}\n</answer>\n{\"ops\":[],\"report\":\"Second.\"}",
        )
        .expect("reply");
        assert_eq!(reply.report, "First.");
    }

    #[test]
    fn accepts_a_bare_operation_array() {
        let reply = parse_reply(r#"[{"op":"remove","id":1}]"#).expect("reply");
        assert_eq!(reply.ops.len(), 1);
        assert!(reply.report.is_empty());
    }

    /// One unknown operation is that operation's problem, not the run's.
    #[test]
    fn one_bad_operation_does_not_discard_the_others() {
        let reply = parse_reply(
            r#"{"ops":[{"op":"add","section":"work","text":"Uses Tauri (app framework).","confidence":0.8},{"op":"nonsense"}],"report":"One fact."}"#,
        )
        .expect("reply");
        assert_eq!(reply.ops.len(), 1);
        assert_eq!(reply.ignored, 1);
        assert_eq!(reply.report, "One fact.");
    }

    #[test]
    fn a_reply_with_neither_key_is_an_error_rather_than_a_decline() {
        assert!(parse_reply(r#"{"notes":"I could not decide"}"#).is_err());
        assert!(parse_reply("no json here").is_err());
    }
}

//! `browser_exec` -- one tool that drives a real browser.
//!
//! # Why one tool and not twelve
//!
//! A browser action is a tiny thing: click here, type this, read that. Twelve
//! tools for them means twelve schemas in every request, and this app caches on
//! the exact bytes of the request body -- so the cost is paid on every turn of
//! every conversation, whether or not a browser is ever opened. One tool is one
//! schema, and the actions live in the script where they compose: a script can
//! read a table, then click a row, then read the result, in a single call.
//!
//! The script is JavaScript because the browser already has a JavaScript engine
//! and the protocol already has "run this in the page". Any other language here
//! would be an interpreter to ship.
//!
//! # Write, not read
//!
//! [`Effect::Write`], like every MCP tool and for the same reason: this can
//! submit a form, place an order, or send a message, and there is no way to tell
//! from the outside which of those it did. Calling a read a write costs a run its
//! cache; calling a write a read tells the model a stale page is current. The
//! conservative direction is taken in the dark.
//!
//! That also means its result is never cached and taking one invalidates the
//! reads before it, which is right: a page that has just been clicked is not the
//! page that was read five steps ago.

use serde_json::Value;

use super::vision;
use super::{Effect, ToolContext, ToolSpec};

/// What the model is told about this tool.
///
/// It carries the whole API, because it is the only place the API is described:
/// the helpers exist in the page's prelude and nothing else tells the model they
/// are there. Kept to what is needed to work and no further -- every line here is
/// in every request of every agent run.
const DESCRIPTION: &str = "\
Drive a real Chrome browser: open pages, read them, and click and type as a person does.

`code` is the body of an async function that runs in the page. `window`, `document` and \
`fetch` are available, `await` works, and a `return`ed value is reported back to you.

Every result ends with the page as numbered text. Those numbers are what the helpers below \
take, and they stay valid until the page navigates.

A navigation ends the script: `goto(...)`, or a click that follows a link, stops it there \
and returns the page you landed on. Moving is normally one call per page.

Finding things — a page is mostly prose, so search it before reading it:
  pz.findText(text | /regex/) -> the lines around each match, like searching a file
  pz.text(selector?) -> the visible text, or one part of the page
  pz.snapshot(selector?) -> the tree again, of that part if you name one
  pz.find(text | /regex/, {in: selector}) -> [{index, label}, ...] then act on a label's index
  pz.attr(i, name) pz.value(i) pz.html(i?) pz.query(selector) pz.url() pz.title()

Acting:
  pz.click(i) pz.dblclick(i) pz.hover(i) pz.focus(i)
  pz.type(i, text, {submit})   pz.press('Enter'|'Ctrl+A')   pz.select(i, option)   pz.check(i, on)
  pz.scroll(down) pz.scrollTo(i)   pz.wait(ms)   pz.waitFor(selectorOrFn, ms)

Moving — these end the script:
  pz.goto(url) pz.back() pz.forward() pz.reload() pz.newTab(url)

On a long page, read the whole tree only as a last resort: search for the value, or name the \
part of the page you mean. Both are a fraction of the size, and the tree of a real page is \
mostly layout.

Example:
  const hits = await pz.findText('invoice total');
  return hits;";

/// A picture of the page, for a model that cannot see it.
///
/// Asked the same way `read_file` asks about an image: the text on the page and
/// what it looks like, because a screenshot of a form is mostly its labels.
const PAGE_QUESTION: &str = "Describe this web page as it appears, for a program that cannot see \
it. Include every piece of text that is visible, what the controls are and what they are set to, \
and anything a viewer would notice: an error, a dialog, a cookie banner, a state that looks wrong.";

/// The tool.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "browser_exec",
        description: DESCRIPTION,
        parameters: r#"{
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "JavaScript to run in the page, as the body of an async function."
                },
                "tab": {
                    "type": "integer",
                    "description": "Which tab to work in, from the `tabs:` line of an earlier result. Defaults to the one already open."
                },
                "screenshot": {
                    "type": "boolean",
                    "description": "Also let the vision model look at the page. Use it when the page's appearance matters and the tree does not explain it. Defaults to false."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }"#,
        effect: Effect::Write,
        // A script is not a command line, and the approval prompt shows this
        // tool's arguments rather than a command it will run.
        command_argument: None,
        execute: |arguments, context| Box::pin(exec(arguments, context)),
    }
}

/// Runs one script, and looks at the page if asked.
async fn exec(arguments: Value, context: ToolContext) -> Result<String, String> {
    let code = arguments
        .get("code")
        .and_then(Value::as_str)
        .ok_or("browser_exec needs a string `code`")?
        .to_string();
    let tab = arguments
        .get("tab")
        .and_then(Value::as_u64)
        .map(|index| index as usize);
    let want_screenshot = arguments
        .get("screenshot")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let outcome = crate::browser::run(&code, tab, want_screenshot, &context.cancelled).await?;

    let mut report = outcome.report;
    if let Some(picture) = outcome.screenshot.as_deref() {
        report.push_str("\n\n");
        report.push_str(&look_at(&context, picture).await);
    }

    // A script that stopped short is reported as a failure so the panel marks the
    // row and the model is told plainly -- but the report goes with it, because
    // the page it failed on is the thing that says what to do next.
    if outcome.failed {
        Err(report)
    } else {
        Ok(report)
    }
}

/// Shows a picture of the page to the vision model the user chose.
///
/// A missing vision model is a sentence rather than a failure: everything else
/// the call found is still worth having, and a page a model could not look at is
/// not a page it could not read.
async fn look_at(context: &ToolContext, picture: &[u8]) -> String {
    let Some((endpoint, model)) = context
        .run
        .as_ref()
        .and_then(|run| run.vision_model.clone())
    else {
        return "[a picture of the page was taken, but no vision model is set, so there is nobody \
                to look at it. Ask the user to choose one in Settings.]"
            .to_string();
    };
    match vision::describe_image(&endpoint, &model, picture, "image/png", PAGE_QUESTION).await {
        Ok(description) => format!("[the page, as {model} sees it]\n\n{description}"),
        Err(error) => format!("[the picture could not be described: {error}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_tool_is_named_and_classified() {
        let spec = spec();
        assert_eq!(spec.name, "browser_exec");
        // A browser can submit a form or place an order, so its result is never
        // cached and taking one invalidates the reads before it.
        assert_eq!(spec.effect, Effect::Write);
        assert_eq!(spec.command_argument, None);
    }

    /// The schema has to parse: a malformed one advertises no arguments at all
    /// and fails much later, as a confusing tool-call error.
    #[test]
    fn the_arguments_are_a_well_formed_schema() {
        let schema: Value = serde_json::from_str(spec().parameters).expect("valid JSON");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["code"]));
        assert!(schema["properties"]["code"].is_object());
        assert!(schema["properties"]["tab"].is_object());
        assert!(schema["properties"]["screenshot"].is_object());
        // Unknown arguments are rejected rather than ignored, which is what the
        // registry's parser needs this to be false for.
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    /// A missing script is a plain refusal, not a browser started for a call that
    /// could never have run.
    ///
    /// Driven on the runtime the app already depends on rather than with an async
    /// test attribute, which is how the rest of this suite reaches an async tool.
    #[test]
    fn a_call_with_no_script_is_refused_by_name() {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let error = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(crate::browser::run("   ", None, false, &cancelled))
            .expect_err("refused");
        assert!(error.contains("script"), "{error}");
    }

    /// The description is the whole API reference. If it stops naming the
    /// helpers, the model is asked to write a script against an API it was never
    /// told about -- which is a tool that fails in a way no error message
    /// explains.
    #[test]
    fn the_description_still_teaches_the_api() {
        for helper in [
            "goto", "click", "type", "press", "select", "snapshot", "find", "waitFor",
        ] {
            assert!(DESCRIPTION.contains(helper), "`{helper}` is not described");
        }
        // The one behaviour a model cannot infer, and the one that will otherwise
        // make every navigating step look like a failure.
        assert!(
            DESCRIPTION.contains("navigation ends the script"),
            "the navigation rule is not stated"
        );
        // And the prefix, since the helpers are meant to read as `pz.click` too.
        assert!(DESCRIPTION.contains("pz"), "the `pz` name is not stated");
    }
}

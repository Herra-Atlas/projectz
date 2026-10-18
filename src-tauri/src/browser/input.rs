//! Real mouse and keyboard input, through the browser's own pipeline.
//!
//! # Why not a DOM event
//!
//! `element.click()` from a script produces a `MouseEvent` with
//! `isTrusted: false`. A page is entitled to ignore it, and sites that care --
//! anything with a payment step, a canvas, or pointer-event handling -- routinely
//! do. What a person's click has that a script's does not is a trip through the
//! browser's input stack, and the only way to get one is
//! `Input.dispatchMouseEvent`, which is why acting on a page is the driver's job
//! and not the page's.
//!
//! # Coordinates
//!
//! Every position here is in the top page's viewport, in CSS pixels. The prelude
//! computes it from the element's own box and corrects for any enclosing frame,
//! because that is where the element is and this is where the click lands -- see
//! `point()` in `prelude.js`. Nothing here resolves a selector: by the time a
//! call arrives, the page has already said exactly where.

use serde_json::{json, Value};

use super::cdp::Connection;

/// The modifier bits the protocol uses. Not our numbering -- these are the values
/// `modifiers` is documented to take, and they are also the bits the browser
/// reports back.
const ALT: u32 = 1;
const CTRL: u32 = 2;
const META: u32 = 4;
const SHIFT: u32 = 8;

/// Clicks at a point, `count` times, as a person would.
///
/// The move first is deliberate: a page that reveals a control on hover, or that
/// tracks the pointer, has to see it arrive. Omitting the move is the difference
/// between clicking a button and clicking where a button is about to be.
pub async fn click(
    conn: &Connection,
    session: &str,
    x: f64,
    y: f64,
    count: usize,
) -> Result<Value, String> {
    mouse(conn, session, "mouseMoved", x, y, "none", 0, 0).await?;
    for press in 1..=count.max(1) {
        mouse(conn, session, "mousePressed", x, y, "left", 1, press).await?;
        mouse(conn, session, "mouseReleased", x, y, "left", 0, press).await?;
    }
    Ok(json!(true))
}

/// Moves the pointer over a point, without pressing anything.
pub async fn hover(conn: &Connection, session: &str, x: f64, y: f64) -> Result<Value, String> {
    mouse(conn, session, "mouseMoved", x, y, "none", 0, 0).await?;
    Ok(json!(true))
}

/// Scrolls by a delta, at a point.
///
/// A wheel event rather than `scrollTo`, because the page is entitled to treat
/// them differently: an infinite feed that loads on a wheel usually loads on
/// nothing else, and a scroller that snaps does not snap for a jump.
pub async fn wheel(
    conn: &Connection,
    session: &str,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
) -> Result<Value, String> {
    conn.call(
        "Input.dispatchMouseEvent",
        json!({
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": dx,
            "deltaY": dy,
            "modifiers": 0,
        }),
        Some(session),
    )
    .await?;
    Ok(json!(true))
}

/// Inserts text into whatever has focus.
///
/// Not a run of key events. A key event per character would have to invent the
/// layout that maps each one to a character, and would still be wrong for a
/// keyboard that is not the assumed one; `insertText` is the protocol's own "type
/// this into the field", it is what a real click focuses, and it is what a
/// framework's input handler sees.
pub async fn insert_text(conn: &Connection, session: &str, text: &str) -> Result<Value, String> {
    conn.call("Input.insertText", json!({ "text": text }), Some(session))
        .await?;
    Ok(json!(true))
}

/// Presses a key, or a combination like `Ctrl+A`.
///
/// Sent as a down and an up, because a page can tell the difference: pressing
/// Enter is a change and releasing it is not, and a form that submits on keydown
/// would submit once for a "press" that glues them together.
pub async fn key(conn: &Connection, session: &str, combination: &str) -> Result<Value, String> {
    let (modifiers, name) = split(combination)?;
    let shift = modifiers & SHIFT != 0;
    let mut pressed = stroke(&name, shift)
        .ok_or_else(|| format!("`{name}` is not a key this driver knows how to press"))?;

    // A shortcut must not also type its letter: `Ctrl+A` is select-all, and a
    // character event alongside it would insert an "a" into the field it selected.
    if modifiers & !SHIFT != 0 {
        pressed.text = None;
    }

    let mut down = json!({
        "type": if pressed.text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": pressed.key,
        "code": pressed.code,
        "windowsVirtualKeyCode": pressed.virtual_key,
        "nativeVirtualKeyCode": pressed.virtual_key,
        "modifiers": modifiers,
    });
    if let Some(text) = pressed.text {
        down["text"] = Value::String(text);
    }
    conn.call("Input.dispatchKeyEvent", down.clone(), Some(session))
        .await?;

    let mut up = down;
    up["type"] = Value::String("keyUp".into());
    // The character was delivered on the way down; repeating it on the way up
    // types it twice.
    if let Some(fields) = up.as_object_mut() {
        fields.remove("text");
    }
    conn.call("Input.dispatchKeyEvent", up, Some(session))
        .await?;
    Ok(json!(true))
}

/// One mouse event.
#[allow(clippy::too_many_arguments)]
async fn mouse(
    conn: &Connection,
    session: &str,
    kind: &str,
    x: f64,
    y: f64,
    button: &str,
    buttons: u32,
    click_count: usize,
) -> Result<(), String> {
    conn.call(
        "Input.dispatchMouseEvent",
        json!({
            "type": kind,
            "x": x,
            "y": y,
            "button": button,
            "buttons": buttons,
            "clickCount": click_count,
            "modifiers": 0,
        }),
        Some(session),
    )
    .await?;
    Ok(())
}

/// One key, as the protocol wants it described.
struct Stroke {
    /// The character this key produces, as the DOM names it: `"Enter"`, `"a"`, `" "`.
    key: String,
    /// The physical position on the keyboard: `"KeyA"`, `"Enter"`. Distinct from
    /// `key` on purpose -- a US layout and a French one share positions and not
    /// characters, and the protocol carries both so a site can use either.
    code: String,
    /// The Windows virtual key code, which is what a keydown handler reads.
    virtual_key: u32,
    /// What the key would insert, or `None` for a key that inserts nothing.
    text: Option<String>,
}

/// The keys that have a name rather than a character.
///
/// Aliases are listed because these are typed by a model reasoning about what it
/// wants to do, not by a keyboard: `"up"` and `"ArrowUp"` are the same intention,
/// and refusing one of them would be a driver being pedantic about a synonym.
const NAMED: &[(&str, &str, &str, u32, Option<char>)] = &[
    ("enter", "Enter", "Enter", 13, None),
    ("return", "Enter", "Enter", 13, None),
    ("tab", "Tab", "Tab", 9, None),
    ("escape", "Escape", "Escape", 27, None),
    ("esc", "Escape", "Escape", 27, None),
    ("backspace", "Backspace", "Backspace", 8, None),
    ("delete", "Delete", "Delete", 46, None),
    ("del", "Delete", "Delete", 46, None),
    ("insert", "Insert", "Insert", 45, None),
    ("home", "Home", "Home", 36, None),
    ("end", "End", "End", 35, None),
    ("pageup", "PageUp", "PageUp", 33, None),
    ("pagedown", "PageDown", "PageDown", 34, None),
    ("arrowup", "ArrowUp", "ArrowUp", 38, None),
    ("arrowdown", "ArrowDown", "ArrowDown", 40, None),
    ("arrowleft", "ArrowLeft", "ArrowLeft", 37, None),
    ("arrowright", "ArrowRight", "ArrowRight", 39, None),
    ("up", "ArrowUp", "ArrowUp", 38, None),
    ("down", "ArrowDown", "ArrowDown", 40, None),
    ("left", "ArrowLeft", "ArrowLeft", 37, None),
    ("right", "ArrowRight", "ArrowRight", 39, None),
    ("space", " ", "Space", 32, Some(' ')),
    ("spacebar", " ", "Space", 32, Some(' ')),
];

/// A named key, or a single character, described the way the protocol wants it.
fn stroke(name: &str, shift: bool) -> Option<Stroke> {
    if let Some((_, key, code, virtual_key, character)) = NAMED
        .iter()
        .find(|(alias, ..)| alias.eq_ignore_ascii_case(name))
        .copied()
    {
        return Some(Stroke {
            key: key.to_string(),
            code: code.to_string(),
            virtual_key,
            text: character.map(|character| character.to_string()),
        });
    }

    // One character, or nothing this driver can press.
    let mut characters = name.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }

    if character.is_ascii_alphabetic() {
        let upper = character.to_ascii_uppercase();
        // A shifted letter is capital, and that capital is both what the key
        // reports and what it types.
        let key = if shift {
            upper.to_string()
        } else {
            character.to_ascii_lowercase().to_string()
        };
        return Some(Stroke {
            code: format!("Key{upper}"),
            virtual_key: upper as u32,
            text: Some(key.clone()),
            key,
        });
    }
    if character.is_ascii_digit() {
        return Some(Stroke {
            key: character.to_string(),
            code: format!("Digit{character}"),
            virtual_key: character as u32,
            text: Some(character.to_string()),
        });
    }

    let (position, virtual_key) = punctuation(character)?;
    let key = if shift {
        shifted(character).to_string()
    } else {
        character.to_string()
    };
    Some(Stroke {
        key: key.clone(),
        code: position.to_string(),
        virtual_key,
        text: Some(key),
    })
}

/// The physical position and virtual key code of a punctuation character.
fn punctuation(character: char) -> Option<(&'static str, u32)> {
    Some(match character {
        '-' => ("Minus", 189),
        '=' => ("Equal", 187),
        ',' => ("Comma", 188),
        '.' => ("Period", 190),
        '/' => ("Slash", 191),
        ';' => ("Semicolon", 186),
        '\'' => ("Quote", 222),
        '`' => ("Backquote", 192),
        '[' => ("BracketLeft", 219),
        ']' => ("BracketRight", 221),
        '\\' => ("Backslash", 220),
        _ => return None,
    })
}

/// What a punctuation key types while shift is held.
fn shifted(character: char) -> char {
    match character {
        '-' => '_',
        '=' => '+',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        ';' => ':',
        '\'' => '"',
        '`' => '~',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        other => other,
    }
}

/// Splits `"Ctrl+Shift+A"` into its modifier bits and the key itself.
fn split(combination: &str) -> Result<(u32, String), String> {
    let mut parts: Vec<&str> = combination.split('+').map(str::trim).collect();
    let name = parts
        .pop()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("`{combination}` does not name a key"))?
        .to_string();

    let mut modifiers = 0;
    for part in parts {
        modifiers |= match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => CTRL,
            "alt" | "option" => ALT,
            "shift" => SHIFT,
            "meta" | "cmd" | "command" | "super" | "win" => META,
            // The name as it was written, rather than the folded form the match
            // is on: an error that echoes back a word the caller did not type
            // reads like it is about something else.
            _ => return Err(format!("`{part}` is not a modifier this driver knows")),
        };
    }
    Ok((modifiers, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_key_is_recognised_by_name_and_by_alias() {
        for name in ["Enter", "enter", "Return"] {
            let pressed = stroke(name, false).expect("a known key");
            assert_eq!(pressed.key, "Enter");
            assert_eq!(pressed.code, "Enter");
            assert_eq!(pressed.virtual_key, 13);
            assert_eq!(pressed.text, None, "Enter types nothing by itself");
        }
        assert_eq!(stroke("up", false).expect("alias").key, "ArrowUp");
        assert_eq!(stroke("Esc", false).expect("alias").key, "Escape");
    }

    #[test]
    fn a_letter_carries_its_position_and_its_shifted_form() {
        let lower = stroke("a", false).expect("a letter");
        assert_eq!(lower.key, "a");
        assert_eq!(lower.code, "KeyA");
        assert_eq!(lower.virtual_key, 65);
        assert_eq!(lower.text.as_deref(), Some("a"));

        let upper = stroke("a", true).expect("shifted");
        assert_eq!(upper.key, "A");
        assert_eq!(upper.text.as_deref(), Some("A"));
        // The *position* does not change with shift; only what it types does.
        assert_eq!(upper.code, "KeyA");
    }

    #[test]
    fn a_digit_and_a_punctuation_character_have_positions_too() {
        assert_eq!(stroke("7", false).expect("digit").code, "Digit7");
        let minus = stroke("-", false).expect("punctuation");
        assert_eq!(minus.code, "Minus");
        assert_eq!(minus.text.as_deref(), Some("-"));
        assert_eq!(
            stroke("-", true)
                .expect("shifted punctuation")
                .text
                .as_deref(),
            Some("_")
        );
    }

    /// A driver that guesses at a key it does not know types the wrong thing,
    /// which is worse than saying so.
    #[test]
    fn an_unknown_key_is_refused_rather_than_guessed() {
        assert!(stroke("F13", false).is_none());
        assert!(stroke("banana", false).is_none());
        assert!(stroke("", false).is_none());
    }

    #[test]
    fn a_combination_is_split_into_modifiers_and_a_key() {
        let (modifiers, name) = split("Ctrl+Shift+A").expect("a combination");
        assert_eq!(modifiers, CTRL | SHIFT);
        assert_eq!(name, "A");

        let (modifiers, name) = split("Meta+ArrowLeft").expect("a combination");
        assert_eq!(modifiers, META);
        assert_eq!(name, "ArrowLeft");

        // A bare key has no modifiers, which is the common case.
        let (modifiers, name) = split("Enter").expect("a key");
        assert_eq!(modifiers, 0);
        assert_eq!(name, "Enter");
    }

    #[test]
    fn a_modifier_this_driver_does_not_know_is_named_in_the_error() {
        let error = split("Hyper+A").expect_err("unknown modifier");
        assert!(error.contains("Hyper"), "{error}");
        assert!(split("+").is_err());
    }

    /// Each modifier has to map to the protocol's own bit, or a shortcut is sent
    /// as a plain key and does something else entirely.
    #[test]
    fn the_modifier_bits_are_the_ones_the_protocol_uses() {
        assert_eq!(ALT, 1);
        assert_eq!(CTRL, 2);
        assert_eq!(META, 4);
        assert_eq!(SHIFT, 8);
    }
}

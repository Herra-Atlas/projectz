//! The `PATH` a child process is given, read when it is spawned.
//!
//! # Why the inherited block is not enough
//!
//! A process inherits its environment once, at creation. When the app is started
//! from Explorer, that copy of `PATH` is as old as Explorer -- so a tool installed
//! afterwards (`winget install ffmpeg`, a new Node, a new `uv`) is on the machine
//! and invisible to every terminal the app starts until the app is restarted. The
//! registry is the source of truth for the user and machine values; reading it at
//! spawn time is what makes a per-command shell see what the machine sees.
//!
//! # What is merged, and in what order
//!
//! Machine first, then user, then any entry the app itself inherited that neither
//! registry value carried -- a dev shell or a portable install must not disappear
//! because it was never written to the registry. Entries are compared
//! case-insensitively (Windows paths are), the first spelling wins, and the result
//! is bounded: an environment block over the limit makes `CreateProcess` fail
//! outright, which would be worse than a stale `PATH`.
//!
//! # Why a cache
//!
//! The read is a few registry calls -- tens of microseconds -- but it happens
//! before every console spawn, including one per spoken sentence. Three seconds is
//! long enough to collapse a burst and short enough that an install mid-session is
//! visible on the next command.

use std::collections::HashSet;
use std::ffi::OsString;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

const USER_KEY: &str = "Environment";
const MACHINE_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const PATH_VALUE: &str = "Path";

/// How long a read is trusted before the registry is asked again.
const CACHE_TTL: Duration = Duration::from_secs(3);

/// A `PATH` longer than this is not passed on: the environment block is a
/// fixed-size allocation, and an absurd one would make every spawn fail.
const MAX_PATH_BYTES: usize = 32 * 1024;

static CACHE: OnceLock<std::sync::Mutex<Option<(Instant, Option<(OsString, OsString)>)>>> =
    OnceLock::new();

/// Applies the current `PATH` to a standard command.
pub fn apply_std(command: &mut std::process::Command) {
    if let Some((key, value)) = prepared() {
        command.env(key, value);
    }
}

/// Applies the current `PATH` to a Tokio command.
pub fn apply_async(command: &mut tokio::process::Command) {
    if let Some((key, value)) = prepared() {
        command.env(key, value);
    }
}

/// The merged value and the spelling the child's environment already uses for it.
fn prepared() -> Option<(OsString, OsString)> {
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, value)) = guard.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return value.clone();
            }
        }
    }
    let value = fresh();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), value.clone()));
    }
    value
}

fn fresh() -> Option<(OsString, OsString)> {
    let machine = machine_path().map(|raw| expand(&raw, environment));
    let user = user_path().map(|raw| expand(&raw, environment));
    let inherited = std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned());
    let merged = merge(machine.as_deref(), user.as_deref(), inherited.as_deref())?;
    Some((inherited_key(), OsString::from(merged)))
}

fn user_path() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(USER_KEY)
        .ok()?
        .get_value(PATH_VALUE)
        .ok()
}

fn machine_path() -> Option<String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(MACHINE_KEY)
        .ok()?
        .get_value(PATH_VALUE)
        .ok()
}

/// The key the app already has for `PATH`, so the override replaces it rather
/// than adding a second, differently-cased entry.
fn inherited_key() -> OsString {
    std::env::vars_os()
        .map(|(key, _)| key)
        .find(|key| key.to_string_lossy().eq_ignore_ascii_case("path"))
        .unwrap_or_else(|| OsString::from("Path"))
}

fn environment(name: &str) -> Option<String> {
    std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
}

/// Expands `%NAME%` once, leaving an unknown or empty name exactly as written.
fn expand(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match lookup(name) {
            Some(value) if !name.is_empty() => out.push_str(&value),
            _ => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Machine, then user, then inherited entries nobody named, without duplicates.
fn merge(machine: Option<&str>, user: Option<&str>, inherited: Option<&str>) -> Option<String> {
    let mut seen = HashSet::new();
    let mut entries: Vec<&str> = Vec::new();
    for part in machine
        .into_iter()
        .chain(user)
        .chain(inherited)
        .flat_map(|value| value.split(';'))
    {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !seen.insert(part.to_lowercase()) {
            continue;
        }
        entries.push(part);
    }
    if entries.is_empty() {
        return None;
    }
    let joined = entries.join(";");
    (joined.len() <= MAX_PATH_BYTES).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_variable_is_expanded() {
        let expanded = expand(r"%SystemRoot%\System32;%SystemRoot%", |name| {
            (name == "SystemRoot").then(|| r"C:\Windows".to_string())
        });
        assert_eq!(expanded, r"C:\Windows\System32;C:\Windows");
    }

    #[test]
    fn an_unknown_variable_is_left_as_written() {
        assert_eq!(
            expand(r"C:\Tools;%NOPE%\bin", |_| None),
            r"C:\Tools;%NOPE%\bin"
        );
        assert_eq!(expand("%%", |_| Some("x".to_string())), "%%");
    }

    #[test]
    fn machine_then_user_then_inherited_is_deduplicated() {
        assert_eq!(
            merge(
                Some(r"C:\Windows;C:\Tools"),
                Some(r"c:\tools;C:\Users\me\.local\bin"),
                Some(r"C:\Windows;C:\Portable"),
            )
            .as_deref(),
            Some(r"C:\Windows;C:\Tools;C:\Users\me\.local\bin;C:\Portable")
        );
    }

    #[test]
    fn an_empty_path_is_not_passed_on() {
        assert_eq!(merge(Some("; ;"), Some(""), None), None);
        assert_eq!(merge(None, None, None), None);
    }

    /// An over-long block would make `CreateProcess` fail, which is worse than a
    /// stale `PATH`: the old value is kept instead.
    #[test]
    fn an_absurd_path_is_refused_rather_than_truncated() {
        let huge = (0..MAX_PATH_BYTES)
            .map(|index| format!(r"C:\tools\entry-{index}"))
            .collect::<Vec<_>>()
            .join(";");
        assert_eq!(merge(Some(&huge), None, None), None);
    }
}

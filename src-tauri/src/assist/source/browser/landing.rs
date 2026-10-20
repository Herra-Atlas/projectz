//! The page the app serves, so setting the extension up is one URL.
//!
//! # Why the app serves a page at all
//!
//! Because the alternative is the worst part of every tool like this: find the folder,
//! guess that Chrome wants it unzipped, find the token somewhere in the app, copy it,
//! paste it into a popup that may not be where you think, and then find out whether it
//! worked. Every one of those steps is a place to stop.
//!
//! Here, the same port the extension talks to answers a plain `GET` with a page that:
//!
//! - **hands over the extension itself**, zipped, built from the folder on disk at the
//!   moment it is asked for -- so there is no store, no release and no version to keep
//!   in step. What is handed out is what is on this machine;
//! - **says whether it worked**, by polling `/status` and drawing one dot, so the answer
//!   to "did it pair?" is visible rather than guessed at;
//! - **carries the token in the page**, where the extension's content script reads it
//!   and sends it to its own background worker. No copying, no pasting, and nothing for
//!   the user to get wrong.
//!
//! # Why there is no CORS header
//!
//! Deliberately, and it is the security posture rather than an omission. Any page can
//! *ask* for `http://127.0.0.1:43128/status`; without an `Access-Control-Allow-Origin`
//! header the browser refuses to let it *read* the answer, so the token inside it stays
//! on this machine. The extension's content script is exempt because it runs in the
//! page's own origin. Adding that header "so it works with a fetch" would hand the
//! token to every site the user visits.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::assist::Assist;

/// The page, kept as HTML rather than as a Rust string so it can be read and edited as
/// what it is -- the same reason the browser driver keeps its JavaScript in `.js` files.
const PAGE: &str = include_str!("landing.html");

/// One response, already complete enough to write to a socket.
pub struct Http {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Extra headers, without the trailing blank line.
    pub extra: &'static str,
}

impl Http {
    fn html(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.into(),
            extra: "",
        }
    }

    fn json(body: String) -> Self {
        Self {
            status: 200,
            content_type: "application/json; charset=utf-8",
            body: body.into_bytes(),
            extra: "",
        }
    }

    fn not_found(what: &str) -> Self {
        Self {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: format!("{what}\n").into_bytes(),
            extra: "",
        }
    }

    /// The whole response as bytes: status line, headers, body.
    ///
    /// `Connection: close` on every response because exactly one request is served per
    /// connection and then the socket is shut. That is one line instead of a keep-alive
    /// state machine, and a browser reading a page and polling it once every few seconds
    /// cannot tell the difference.
    pub fn into_bytes(self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            404 => "Not Found",
            _ => "Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Connection: close\r\n",
            self.status,
            self.content_type,
            self.body.len()
        );
        if !self.extra.is_empty() {
            head.push_str(self.extra);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// Routes one path. Unknown paths are refused by name rather than served the page, so a
/// typo in the extension's configuration says so instead of looking like a bug.
pub fn route(path: &str, assist: &Assist) -> Http {
    // A query string is not part of the path, and the page is often opened with a
    // fragment by the extension when it hands the token over.
    let path = path.split(['?', '#']).next().unwrap_or("/");
    match path {
        "/" | "/index.html" => Http::html(PAGE),
        "/status" => Http::json(status_json(assist)),
        "/extension.zip" => match zipped_extension(assist) {
            Some(bytes) => Http {
                status: 200,
                content_type: "application/zip",
                body: bytes,
                // The filename is what the browser saves it as when the link is
                // followed directly rather than through the page's `download` attribute.
                extra: "Content-Disposition: attachment; filename=\"projectz-assist.zip\"",
            },
            None => Http::not_found(
                "The extension folder was not found next to the app. Load it unpacked from \
                 the project's `extension` directory instead.",
            ),
        },
        other => Http::not_found(&format!("No such page: {other}")),
    }
}

/// What the page polls to find out whether it worked.
///
/// Carries the token, which is why it is served without a CORS header: the page itself
/// is the only thing allowed to read it, and the extension's content script reads it from
/// the page's own DOM.
pub fn status_json(assist: &Assist) -> String {
    let status = assist.bridge_status();
    serde_json::json!({
        "enabled": assist.is_enabled(),
        "paired": !status.clients.is_empty(),
        "clients": status.clients,
        "port": status.port,
        "token": assist.token(),
        // Whether the app can hand over a copy. False is normal for an installed build
        // that did not ship the folder, and the page says so rather than offering a
        // download that 404s.
        "download": assist.extension_dir().is_some(),
    })
    .to_string()
}

/// The extension folder, zipped, exactly as it is on disk.
///
/// Built per request rather than cached, because it is asked for once by one person and a
/// cache would be a second copy of the extension that can be out of step with the folder
/// it came from.
fn zipped_extension(assist: &Assist) -> Option<Vec<u8>> {
    let root = assist.extension_dir()?;
    zip_directory(&root).ok()
}

/// Zips a directory, with paths relative to it and forward slashes.
fn zip_directory(root: &Path) -> Result<Vec<u8>, String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(root, root, &mut files)?;
    // Sorted so two downloads of an unchanged folder are byte-identical, which is what
    // makes "did anything change?" answerable with a checksum.
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, path) in &files {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            zip.start_file(name.as_str(), options)
                .map_err(|error| error.to_string())?;
            zip.write_all(&bytes).map_err(|error| error.to_string())?;
        }
        zip.finish().map_err(|error| error.to_string())?;
    }
    Ok(buffer.into_inner())
}

/// Every file under a directory, named relative to it.
///
/// `.git`, `node_modules` and `tools` are skipped: they are not part of the extension, and a
/// download that carried them would be a folder nobody can load -- Chrome reads the manifest
/// and the files it names, and everything else is just weight.
fn collect(root: &Path, directory: &Path, into: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("could not read the extension folder: {error}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" || name == "node_modules" || name == "tools" {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, into)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            into.push((relative.to_string_lossy().replace('\\', "/"), path.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paths the extension and the page use, and what each one answers with.
    #[test]
    fn the_routes_are_the_ones_the_page_asks_for() {
        let database = std::sync::Arc::new(crate::database::Database::open_in_memory());
        let assist = Assist::new(database, None);
        assert_eq!(route("/", &assist).content_type, "text/html; charset=utf-8");
        assert_eq!(
            route("/status", &assist).content_type,
            "application/json; charset=utf-8"
        );
        assert_eq!(route("/nonsense", &assist).status, 404);
    }

    /// A query string or a fragment is not part of the path: the extension opens the page
    /// with one when it hands the token over, and a page that 404s on its own install
    /// link is a page nobody can use.
    #[test]
    fn a_query_string_does_not_change_the_route() {
        let database = std::sync::Arc::new(crate::database::Database::open_in_memory());
        let assist = Assist::new(database, None);
        assert_eq!(route("/?pair=1", &assist).status, 200);
        assert_eq!(route("/status?x=1", &assist).status, 200);
        assert_eq!(route("/#top", &assist).status, 200);
    }

    /// The status carries the token and never claims a download it cannot serve.
    #[test]
    fn the_status_reports_what_the_page_needs() {
        let database = std::sync::Arc::new(crate::database::Database::open_in_memory());
        let assist = Assist::new(database, None);
        let body = status_json(&assist);
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("status must be JSON");
        assert!(parsed["token"]
            .as_str()
            .is_some_and(|token| token.len() >= 32));
        assert_eq!(parsed["enabled"], false);
        assert_eq!(parsed["paired"], false);
        assert_eq!(
            parsed["download"], false,
            "no folder is configured in a test"
        );
    }

    /// **No CORS header, on purpose.** Any page can ask for the status; without this
    /// header the browser refuses to let it read the token inside. Adding one is the
    /// single change that would hand the token to every site the user visits.
    #[test]
    fn no_response_lets_another_origin_read_it() {
        let database = std::sync::Arc::new(crate::database::Database::open_in_memory());
        let assist = Assist::new(database, None);
        let response = String::from_utf8(route("/status", &assist).into_bytes()).expect("utf-8");
        assert!(!response
            .to_lowercase()
            .contains("access-control-allow-origin"));
    }

    /// Every response is complete: a status line, a length that matches the body, and a
    /// blank line before it. A wrong length is a page that hangs until it times out.
    #[test]
    fn a_response_carries_a_length_that_matches_its_body() {
        let response = Http::html("<p>hi</p>").into_bytes();
        let text = String::from_utf8(response.clone()).expect("utf-8");
        let (head, body) = text.split_once("\r\n\r\n").expect("a head and a body");
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains(&format!("Content-Length: {}", body.len())));
        assert!(head.contains("Cache-Control: no-store"));
        assert!(head.contains("Connection: close"));
    }

    /// A folder is zipped with the paths relative to it and forward slashes, which is what
    /// Chrome needs to load it unpacked.
    #[test]
    fn a_folder_zips_with_relative_forward_slashed_paths() {
        let root = std::env::temp_dir().join(format!("projectz-ext-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("icons")).expect("temp dir");
        std::fs::write(root.join("manifest.json"), b"{}").expect("write");
        std::fs::write(root.join("icons").join("icon.png"), b"png").expect("write");
        // A folder Chrome must never be handed: it is not part of the extension.
        std::fs::create_dir_all(root.join(".git")).expect("temp dir");
        std::fs::write(root.join(".git").join("HEAD"), b"ref").expect("write");
        // Nor is the icon generator, which is a build-time script.
        std::fs::create_dir_all(root.join("tools")).expect("temp dir");
        std::fs::write(root.join("tools").join("make-icons.ps1"), b"#").expect("write");

        let bytes = zip_directory(&root).expect("zip");
        let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("read the zip");
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&"icons/icon.png".to_string()));
        assert!(
            !names.iter().any(|name| name.contains(".git")),
            "a .git folder must not be handed over: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with("tools/")),
            "the build script must not be handed over: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

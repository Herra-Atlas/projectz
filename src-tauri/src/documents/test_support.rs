//! Helpers the reader tests share.
//!
//! Both document readers are tested against a zip built in the test, which is what
//! makes them testable at all: a fixture file in the repository would be a binary
//! nobody can read, and a `.docx` is only ever a zip of XML.

#![cfg(test)]

use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes a zip with the given text entries.
///
/// `Stored` rather than `Deflated`: the archive is a fixture, and leaving the
/// parts uncompressed keeps a failing test's output readable.
pub(crate) fn write_zip(path: &Path, entries: &[(&str, &str)]) {
    let file = std::fs::File::create(path).expect("create");
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in entries {
        zip.start_file(*name, options).expect("start");
        zip.write_all(body.as_bytes()).expect("write");
    }
    zip.finish().expect("finish");
}

/// A unique temp path for one test.
///
/// A path rather than a folder: the readers take a file, and the caller removes it.
pub(crate) fn temp(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "projectz-doc-{}-{unique}-{name}",
        std::process::id()
    ))
}

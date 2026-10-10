//! Helpers the writer tests share.
//!
//! Reading a part back out of an archive and making a temp folder are needed by
//! every writer's tests, and three copies of the same six lines would be three
//! places to fix the day the test setup changes.

#![cfg(test)]

use std::path::PathBuf;

/// Reads one part back out of an archive.
///
/// A test asserts on the XML the writer *produced*, not on the string it built,
/// so the deflate step and the entry names are covered too.
pub(crate) fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip");
    let mut entry = archive.by_name(name).expect("entry");
    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text).expect("read");
    text
}

/// Every entry name in an archive.
pub(crate) fn names(bytes: &[u8]) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip");
    archive.file_names().map(str::to_string).collect()
}

/// A fresh temp folder for one test.
pub(crate) fn temp_root() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("projectz-export-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A PNG signature and IHDR chunk.
///
/// The writers read only the header, so a fixture can stop there -- whether the
/// pixels behind it are valid is the image's problem, not the writer's.
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes
}

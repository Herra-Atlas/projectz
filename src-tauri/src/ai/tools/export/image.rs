//! Reading a picture's size, so a document can place it at a sensible scale.
//!
//! Word needs an explicit extent in EMU for every picture it draws, and the only
//! source of a picture's proportions is the picture. The alternative -- asking
//! the model for a width and height -- would be a number it has to guess, and a
//! guessed aspect ratio is the one mistake that makes a document look wrong at a
//! glance.
//!
//! Only the format headers are read, not the image: PNG, GIF, JPEG and BMP all
//! state their dimensions in a fixed place near the start, so this is a few
//! bytes of arithmetic rather than a decoder. WebP is deliberately absent -- its
//! size lives in one of three different chunk layouts, and rather than guess, the
//! writer refuses the file by name.
//!
//! Media types are not decided here: [`crate::documents::image_media_type`]
//! already maps extensions to the types the app accepts, and a second table
//! would be a second thing to keep in step.

use std::path::Path;

/// A picture's pixel size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// EMU (English Metric Units) per pixel at the 96 DPI a screen image is assumed
/// to be authored at. Word measures everything in EMU: 914400 to the inch.
const EMU_PER_PIXEL: f64 = 914400.0 / 96.0;

/// The text width of a US Letter page with one-inch margins, in EMU.
///
/// What a full-width picture is scaled to. A picture narrower than this keeps its
/// own size -- enlarging it would only make a small image blurry and loud.
pub(crate) const TEXT_WIDTH_EMU: i64 = 6 * 914_400 + 457_200;

/// A pixel count as EMU.
pub(crate) fn emu(pixels: u32) -> i64 {
    (pixels as f64 * EMU_PER_PIXEL).round() as i64
}

/// The dimensions of an image, or `None` when the format is not one of the four
/// this reads.
pub(crate) fn dimensions(bytes: &[u8]) -> Option<Dimensions> {
    png(bytes)
        .or_else(|| gif(bytes))
        .or_else(|| jpeg(bytes))
        .or_else(|| bmp(bytes))
}

/// The extension a media part should be stored under, from the media type.
pub(crate) fn extension_for(path: &Path) -> Option<&'static str> {
    match crate::documents::image_media_type(path)? {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpeg"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// PNG: an 8-byte signature, then the IHDR chunk whose first two fields are the
/// width and the height, both big-endian.
fn png(bytes: &[u8]) -> Option<Dimensions> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some(Dimensions {
        width: u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        height: u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    })
}

/// GIF: the size is two little-endian `u16`s right after the version tag.
fn gif(bytes: &[u8]) -> Option<Dimensions> {
    if bytes.len() < 10 || (&bytes[..6] != b"GIF87a" && &bytes[..6] != b"GIF89a") {
        return None;
    }
    Some(Dimensions {
        width: u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,
        height: u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32,
    })
}

/// JPEG: walk the segment markers to the first start-of-frame, which carries the
/// height and width as big-endian `u16`s after a one-byte precision and a
/// two-byte length.
fn jpeg(bytes: &[u8]) -> Option<Dimensions> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        // Padding between segments is a run of `0xFF`, not a marker.
        if marker == 0xFF {
            index += 1;
            continue;
        }
        // Start-of-frame markers. The arithmetic-coded and differential variants
        // are in this set too; the four excluded values are not frames.
        let is_frame = matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        );
        let length = u16::from_be_bytes(bytes[index + 2..index + 4].try_into().ok()?) as usize;
        if is_frame {
            return Some(Dimensions {
                height: u16::from_be_bytes(bytes[index + 5..index + 7].try_into().ok()?) as u32,
                width: u16::from_be_bytes(bytes[index + 7..index + 9].try_into().ok()?) as u32,
            });
        }
        // A segment shorter than its own header would loop forever.
        if length < 2 {
            return None;
        }
        index += 2 + length;
    }
    None
}

/// BMP: a 14-byte file header, then a DIB header whose width and height are
/// little-endian `i32`s. The height is signed -- a negative one means the rows
/// are stored top-down -- so its magnitude is what matters.
fn bmp(bytes: &[u8]) -> Option<Dimensions> {
    if bytes.len() < 26 || &bytes[..2] != b"BM" {
        return None;
    }
    let width = i32::from_le_bytes(bytes[18..22].try_into().ok()?);
    let height = i32::from_le_bytes(bytes[22..26].try_into().ok()?);
    if width <= 0 {
        return None;
    }
    Some(Dimensions {
        width: width as u32,
        height: height.unsigned_abs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG signature and IHDR chunk are all the reader looks at, so a fixture
    /// can stop there -- the CRC is Word's problem, not the writer's.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn a_png_reports_its_size() {
        assert_eq!(
            dimensions(&png_header(640, 480)),
            Some(Dimensions {
                width: 640,
                height: 480
            })
        );
    }

    #[test]
    fn a_gif_reports_its_size_little_endian() {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0]);
        assert_eq!(
            dimensions(&bytes),
            Some(Dimensions {
                width: 16,
                height: 32
            })
        );
    }

    /// The frame marker is not the first segment, so the walk has to step over
    /// the ones before it rather than reading the first `0xFF` it finds.
    #[test]
    fn a_jpeg_walks_past_its_other_segments() {
        let mut bytes = vec![0xFF, 0xD8];
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&120u16.to_be_bytes());
        bytes.extend_from_slice(&160u16.to_be_bytes());
        bytes.extend_from_slice(&[0; 8]);
        assert_eq!(
            dimensions(&bytes),
            Some(Dimensions {
                width: 160,
                height: 120
            })
        );
    }

    #[test]
    fn something_that_is_not_an_image_reports_nothing() {
        assert_eq!(dimensions(b"just text"), None);
        assert_eq!(dimensions(&[]), None);
    }

    #[test]
    fn pixels_become_emu_at_screen_density() {
        assert_eq!(emu(96), 914_400);
        assert_eq!(emu(192), 1_828_800);
    }

    #[test]
    fn a_webp_is_not_one_of_the_four() {
        assert_eq!(extension_for(Path::new("a.webp")), None);
    }
}

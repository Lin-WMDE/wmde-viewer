// SPDX-License-Identifier: GPL-3.0-only

// WMDE: camera RAW files, shown through the preview the camera wrote into them, the way
// wmde-photos showed them. No demosaic: turning sensor data into a photograph is a darkroom's
// job, and the preview carries the camera's own white balance and curve - what the photographer
// saw on the back of the camera.
//
// Where the preview sits differs with every make: a SubIFD, a maker note, a block of the maker's
// own container. So the file is scanned for JPEG starts and each is asked for its size; there is
// no table of makes to go stale. Lossless JPEG is passed over: it is how CR2 and DNG hold the
// sensor data itself, not a picture.

use exif::{Exif, In, Tag};
use image::{ImageFormat, metadata::Orientation};
use std::{cmp::Reverse, collections::HashMap, io::Cursor, path::Path};
use zune_image::codecs::{bmp::zune_core::bytestream::ZCursor, jpeg::JpegDecoder};

/// The RAW formats opened, by extension, with their shared-mime-info types.
///
/// They move together with `MimeType=` in `res/fun.wmde.Viewer.desktop` and the metainfo, which
/// `tests/raw.rs` checks. Canon CR3 is left out: its preview sits in an ISO BMFF box of its own,
/// and wmde-photos did not claim it either.
pub const RAW_TYPES: [(&str, &str); 13] = [
    ("dng", "image/x-adobe-dng"),
    ("cr2", "image/x-canon-cr2"),
    ("crw", "image/x-canon-crw"),
    ("nef", "image/x-nikon-nef"),
    ("nrw", "image/x-nikon-nrw"),
    ("arw", "image/x-sony-arw"),
    ("orf", "image/x-olympus-orf"),
    ("rw2", "image/x-panasonic-rw2"),
    ("pef", "image/x-pentax-pef"),
    ("srw", "image/x-samsung-srw"),
    ("raf", "image/x-fuji-raf"),
    ("mrw", "image/x-minolta-mrw"),
    ("x3f", "image/x-sigma-x3f"),
];

/// The media type of a RAW extension, whatever its case; `None` for anything else.
#[must_use]
pub fn raw_mime_type(extension: &str) -> Option<&'static str> {
    RAW_TYPES
        .iter()
        .find(|(raw, _)| raw.eq_ignore_ascii_case(extension))
        .map(|(_, mime)| *mime)
}

/// Whether `path` carries a RAW extension.
#[must_use]
pub fn is_raw_name(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(raw_mime_type)
        .is_some()
}

/// Whether the file is read as a RAW: a RAW name over content that is a TIFF container or no
/// format of its own. A JPEG or a PNG under a RAW name opens as what it is; `head` is the start
/// of the file.
#[must_use]
pub(crate) fn reads_as_raw(path: &Path, head: &[u8]) -> bool {
    is_raw_name(path) && matches!(image::guess_format(head), Ok(ImageFormat::Tiff) | Err(_))
}

/// A JPEG inside a RAW file. `data` runs from its start to the end of the file; a decoder stops
/// at the end of the picture.
pub(crate) struct Preview<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// The JPEGs inside a RAW file that are pictures, the largest first.
pub(crate) fn previews(bytes: &[u8]) -> Vec<Preview<'_>> {
    /// A cap on the starts tried, so a pathological file costs milliseconds and not minutes. It
    /// is high because a DNG may keep its sensor data in hundreds of tiles, each a start.
    const CANDIDATES: usize = 4096;

    let mut previews: Vec<Preview> = memchr::memmem::find_iter(bytes, &[0xFF, 0xD8, 0xFF])
        .take(CANDIDATES)
        .filter_map(|start| {
            let data = &bytes[start..];
            let (width, height) = declared_size(data)?;
            let width = u32::try_from(width).ok()?;
            let height = u32::try_from(height).ok()?;
            (width > 0 && height > 0).then_some(Preview {
                data,
                width,
                height,
            })
        })
        .collect();
    // The main image of a lossy DNG is baseline JPEG tiles, many of one size; a camera writes
    // each preview once.
    let mut sizes: HashMap<(u32, u32), usize> = HashMap::new();
    for preview in &previews {
        *sizes.entry((preview.width, preview.height)).or_default() += 1;
    }
    previews.retain(|preview| sizes[&(preview.width, preview.height)] < 3);
    previews.sort_by_key(|preview| Reverse(u64::from(preview.width) * u64::from(preview.height)));
    previews
}

/// The size the JPEG at the start of `data` declares; `None` for lossless JPEG and for what is
/// no JPEG at all.
fn declared_size(data: &[u8]) -> Option<(usize, usize)> {
    // A decompressor of its own for each start: one that has read a header carries on from
    // there, and takes the next start for a second SOI marker.
    if let Ok(header) = turbojpeg::read_header(data) {
        return (!header.is_lossless).then_some((header.width, header.height));
    }
    // libjpeg-turbo fails a header on a mere warning, stray bytes between two segments for one.
    // zune-jpeg reads through those, as the decoder such a preview then goes to does, and takes
    // no lossless JPEG either.
    let mut decoder = JpegDecoder::new(ZCursor::new(data));
    decoder.decode_headers().ok()?;
    decoder.dimensions()
}

/// Which way the photograph is viewed: as the RAW file's own TIFF structure says, else as the
/// largest preview that carries EXIF says (Fujifilm's RAF has no TIFF structure to read). Not
/// as the preview shown says: a small one is often the bare thumbnail inside a larger one.
pub(crate) fn orientation(bytes: &[u8], previews: &[Preview]) -> Option<Orientation> {
    let tag = |exif: Exif| {
        exif.get_field(Tag::Orientation, In::PRIMARY)?
            .value
            .get_uint(0)
    };
    own_exif(bytes)
        .and_then(tag)
        .or_else(|| {
            previews
                .iter()
                .find_map(|preview| preview_exif(preview.data).and_then(tag))
        })
        .and_then(|value| Orientation::from_exif(u8::try_from(value).ok()?))
}

/// The EXIF of a RAW file for the details drawer: its own TIFF structure's when that holds the
/// shot's fields, else its preview's, where Fujifilm and Panasonic keep them.
pub(crate) fn exif(bytes: &[u8]) -> Option<Exif> {
    let own = own_exif(bytes);
    if own.as_ref().is_some_and(describes_the_shot) {
        return own;
    }
    let preview = previews(bytes)
        .iter()
        .find_map(|preview| preview_exif(preview.data));
    if preview.as_ref().is_some_and(describes_the_shot) {
        preview
    } else {
        own.or(preview)
    }
}

fn describes_the_shot(exif: &Exif) -> bool {
    [Tag::DateTimeOriginal, Tag::ExposureTime, Tag::FNumber]
        .into_iter()
        .any(|tag| exif.get_field(tag, In::PRIMARY).is_some())
}

/// The EXIF of the file's own TIFF structure. Olympus and Panasonic write TIFF under a magic
/// number of their own, which the EXIF reader refuses, so it is put back to TIFF's.
fn own_exif(bytes: &[u8]) -> Option<Exif> {
    let magic = match bytes.first_chunk::<4>()? {
        b"II*\0" | b"IIRO" | b"IIRS" | b"IIU\0" => b"II*\0",
        b"MM\0*" | b"MMOR" => b"MM\0*",
        _ => return None,
    };
    let mut tiff = bytes.to_vec();
    tiff[..4].copy_from_slice(magic);
    lenient(exif::Reader::new().continue_on_error(true).read_raw(tiff))
}

/// The EXIF a preview carries itself.
fn preview_exif(preview: &[u8]) -> Option<Exif> {
    lenient(
        exif::Reader::new()
            .continue_on_error(true)
            .read_from_container(&mut Cursor::new(preview)),
    )
}

/// What was read, even where a maker's own entries broke the rest.
fn lenient(read: Result<Exif, exif::Error>) -> Option<Exif> {
    read.or_else(|err| {
        err.distill_partial_result(|errors| {
            tracing::debug!("{} EXIF entries of a RAW file are unreadable", errors.len());
        })
    })
    .ok()
}

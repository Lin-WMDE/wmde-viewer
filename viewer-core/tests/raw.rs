// SPDX-License-Identifier: GPL-3.0-only

// WMDE: camera RAW files, shown through the preview inside them, and compressed SVG. The RAW
// fixtures are built here rather than kept as camera files: a TIFF structure with the
// orientation and the camera, followed by what a scan meets inside a real one - a small
// preview, a false JPEG start, a lossless JPEG such as CR2 and DNG hold the sensor data in,
// and the large preview.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};
use viewer_core::{
    LoadError, image_mime_type, is_drawing, is_supported_image, load_image, load_thumbnail,
    loader::load_for_wallpaper, raw::RAW_TYPES, read_details, wallpaper::is_native,
};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const YELLOW: [u8; 3] = [255, 255, 0];

/// `width` by `height`, the left half `left` and the right half `right`, as the sensor saw it.
fn halves(width: u32, height: u32, left: [u8; 3], right: [u8; 3]) -> Vec<u8> {
    (0..height)
        .flat_map(|_| (0..width).flat_map(move |x| if x < width / 2 { left } else { right }))
        .collect()
}

/// A JPEG without chroma subsampling, so the halves keep their colours to the edge.
fn jpeg(width: u32, height: u32, left: [u8; 3], right: [u8; 3], lossless: bool) -> Vec<u8> {
    let pixels = halves(width, height, left, right);
    let mut compressor = Compressor::new().unwrap();
    compressor.set_subsamp(Subsamp::None).unwrap();
    compressor.set_quality(100).unwrap();
    compressor.set_lossless(lossless).unwrap();
    compressor
        .compress_to_vec(Image {
            pixels: pixels.as_slice(),
            width: width as usize,
            pitch: 3 * width as usize,
            height: height as usize,
            format: PixelFormat::RGB,
        })
        .unwrap()
}

/// A little-endian TIFF structure under `magic`: IFD0 with the make, the model and the
/// orientation.
fn tiff(magic: [u8; 4], orientation: u16) -> Vec<u8> {
    // The header, then IFD0 at 8: a count, three entries and the next offset end at 50, where
    // the two strings follow.
    let mut out = magic.to_vec();
    out.extend(8_u32.to_le_bytes());
    out.extend(3_u16.to_le_bytes());
    for (tag, kind, count, value) in [
        (0x010F_u16, 2_u16, 5_u32, 50_u32),
        (0x0110, 2, 5, 55),
        (0x0112, 3, 1, u32::from(orientation)),
    ] {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
    out.extend(0_u32.to_le_bytes());
    out.extend(b"WMDE\0Test\0");
    out
}

/// `jpeg` with an EXIF segment holding `tiff`, right after its start.
fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    let length = u16::try_from(2 + 6 + tiff.len()).unwrap();
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(length.to_be_bytes());
    out.extend(b"Exif\0\0");
    out.extend(tiff);
    out.extend(&jpeg[2..]);
    out
}

/// `jpeg` with a stray byte after its first segment, which libjpeg-turbo reports as a warning
/// and fails the header on.
fn with_stray_byte(jpeg: &[u8]) -> Vec<u8> {
    let end = 4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
    [&jpeg[..end], &[0], &jpeg[end..]].concat()
}

/// What a scan meets after the TIFF structure: a preview 8 by 4, green and yellow; a false
/// JPEG start; a lossless JPEG larger than anything else; the preview 64 by 32, red and blue.
fn inside() -> Vec<u8> {
    let mut out = jpeg(8, 4, GREEN, YELLOW, false);
    out.extend(b"\xFF\xD8\xFF\x00 not a picture");
    out.extend(jpeg(256, 256, YELLOW, GREEN, true));
    out.extend(jpeg(64, 32, RED, BLUE, false));
    out
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn assert_upright(
    loaded: &viewer_core::LoadedImage,
    width: u32,
    height: u32,
    top: [u8; 3],
    bottom: [u8; 3],
) {
    assert_eq!((loaded.width, loaded.height), (width, height), "size");
    let rgba = loaded.image.to_rgba8();
    assert_eq!(rgba.dimensions(), (width, height), "the stored image too");
    let near = |at: (u32, u32), want: [u8; 3]| {
        let px = rgba.get_pixel(at.0, at.1).0;
        assert!(
            px.iter()
                .zip(want)
                .all(|(&got, want)| got.abs_diff(want) < 48),
            "pixel {at:?} is {px:?}, expected about {want:?}"
        );
    };
    near((width / 2, height / 4), top);
    near((width / 2, height * 3 / 4), bottom);
}

#[tokio::test]
async fn a_raw_file_shows_its_largest_preview_turned_by_its_own_tiff() {
    let dir = tempfile::tempdir().unwrap();
    let nef = write(
        dir.path(),
        "DSC_0001.NEF",
        &[tiff(*b"II*\0", 6), inside()].concat(),
    );
    let loaded = load_image(nef.clone()).await.expect("load");
    assert_upright(&loaded, 32, 64, RED, BLUE);
    let wallpaper = load_for_wallpaper(&nef, 3840).expect("wallpaper");
    assert_eq!((wallpaper.width(), wallpaper.height()), (32, 64));
}

#[tokio::test]
async fn a_preview_with_a_stray_byte_is_still_the_one_shown() {
    let dir = tempfile::tempdir().unwrap();
    let broken = with_stray_byte(&jpeg(64, 32, RED, BLUE, false));
    assert!(
        turbojpeg::read_header(&broken).is_err(),
        "the fixture warns"
    );
    let bytes = [tiff(*b"II*\0", 6), jpeg(8, 4, GREEN, YELLOW, false), broken].concat();
    let pef = write(dir.path(), "IMGP0001.PEF", &bytes);
    let loaded = load_image(pef).await.expect("load");
    assert_upright(&loaded, 32, 64, RED, BLUE);
}

#[tokio::test]
async fn a_raw_thumbnail_is_the_smallest_preview_that_covers_it() {
    let dir = tempfile::tempdir().unwrap();
    let nef = write(
        dir.path(),
        "DSC_0001.nef",
        &[tiff(*b"II*\0", 6), inside()].concat(),
    );
    let small = load_thumbnail(nef.clone(), 8).await.expect("small");
    assert_upright(&small, 4, 8, GREEN, YELLOW);
    // 32 is more than the small preview holds: the large one, DCT-scaled by half.
    let large = load_thumbnail(nef, 32).await.expect("large");
    assert_upright(&large, 16, 32, RED, BLUE);
}

#[tokio::test]
async fn olympus_and_panasonic_files_are_turned_by_their_own_tiff() {
    let dir = tempfile::tempdir().unwrap();
    for (name, magic) in [("P1.ORF", *b"IIRO"), ("P2.RW2", *b"IIU\0")] {
        let path = write(dir.path(), name, &[tiff(magic, 6), inside()].concat());
        let loaded = load_image(path.clone()).await.expect(name);
        assert_upright(&loaded, 32, 64, RED, BLUE);
        let details = read_details(&path);
        assert_eq!(details.format, Some("RAW"), "{name}");
        let camera = details.exif.and_then(|exif| exif.camera);
        assert!(
            camera.is_some_and(|camera| camera.contains("Test")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_fujifilm_file_is_turned_by_the_exif_of_its_preview() {
    let dir = tempfile::tempdir().unwrap();
    let preview = with_exif(&jpeg(64, 32, RED, BLUE, false), &tiff(*b"II*\0", 6));
    let bytes = [b"FUJIFILMCCD-RAW 0201".as_slice(), &[0; 100], &preview].concat();
    let raf = write(dir.path(), "DSCF0001.RAF", &bytes);
    let loaded = load_image(raf.clone()).await.expect("load");
    assert_upright(&loaded, 32, 64, RED, BLUE);
    let camera = read_details(&raf).exif.and_then(|exif| exif.camera);
    assert!(camera.is_some_and(|camera| camera.contains("Test")));
}

#[tokio::test]
async fn a_bare_thumbnail_inside_a_preview_is_turned_as_the_preview_says() {
    // The EXIF of the RAF preview holds the orientation and, after it, a thumbnail of its own
    // with no EXIF: a thumbnail size it covers must not come out on its side.
    let dir = tempfile::tempdir().unwrap();
    let app1 = [tiff(*b"II*\0", 6), jpeg(8, 4, GREEN, YELLOW, false)].concat();
    let preview = with_exif(&jpeg(64, 32, RED, BLUE, false), &app1);
    let bytes = [b"FUJIFILMCCD-RAW 0201".as_slice(), &[0; 100], &preview].concat();
    let raf = write(dir.path(), "DSCF0002.RAF", &bytes);
    let thumb = load_thumbnail(raf, 8).await.expect("thumb");
    assert_upright(&thumb, 4, 8, GREEN, YELLOW);
}

#[tokio::test]
async fn the_tiles_of_a_lossy_dng_are_no_preview() {
    let dir = tempfile::tempdir().unwrap();
    let tiles: Vec<u8> = (0..6)
        .flat_map(|_| jpeg(16, 16, YELLOW, GREEN, false))
        .collect();
    let with_preview = [
        tiff(*b"II*\0", 6),
        tiles.clone(),
        jpeg(64, 32, RED, BLUE, false),
    ]
    .concat();
    let dng = write(dir.path(), "lossy.dng", &with_preview);
    // 16 is covered by a tile and not by the small preview's 8: the large preview it is.
    let thumb = load_thumbnail(dng, 16).await.expect("thumb");
    assert_upright(&thumb, 8, 16, RED, BLUE);
    let without = write(
        dir.path(),
        "tiles-only.dng",
        &[tiff(*b"II*\0", 1), tiles].concat(),
    );
    assert!(matches!(
        load_image(without).await,
        Err(LoadError::NoPreview)
    ));
}

#[tokio::test]
async fn a_raw_file_without_a_preview_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = [tiff(*b"II*\0", 1), jpeg(64, 64, RED, BLUE, true)].concat();
    let dng = write(dir.path(), "IMG_0001.dng", &bytes);
    assert!(matches!(
        load_image(dng.clone()).await,
        Err(LoadError::NoPreview)
    ));
    assert!(matches!(
        load_thumbnail(dng, 32).await,
        Err(LoadError::NoPreview)
    ));
}

#[tokio::test]
async fn a_jpeg_under_a_raw_name_opens_as_what_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let nef = write(dir.path(), "export.nef", &jpeg(64, 32, RED, BLUE, false));
    assert_eq!(load_image(nef.clone()).await.expect("load").width, 64);
    assert_eq!(read_details(&nef).format, Some("JPEG"));
    // wmde-bg reads by content: a JPEG is set as it is, a RAW goes through a PNG of ours.
    assert!(is_native(&nef));
    let raw = write(
        dir.path(),
        "a.nef",
        &[tiff(*b"II*\0", 6), inside()].concat(),
    );
    assert!(!is_native(&raw));
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xEDB8_8320
            };
        }
    }
    !crc
}

/// `data` in a gzip member of one stored deflate block: compressed SVG without an encoder.
fn gzip(data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len()).unwrap();
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3, 1];
    out.extend(length.to_le_bytes());
    out.extend((!length).to_le_bytes());
    out.extend(data);
    out.extend(crc32(data).to_le_bytes());
    out.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

#[tokio::test]
async fn a_compressed_drawing_opens_like_a_plain_one() {
    let dir = tempfile::tempdir().unwrap();
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;
    let svgz = write(dir.path(), "drawing.SVGZ", &gzip(svg));
    assert!(is_drawing(&svgz));
    assert!(is_supported_image(&svgz));

    let loaded = load_image(svgz.clone()).await.expect("load");
    assert_eq!(
        loaded.width,
        2 * loaded.height,
        "drawn at its own proportions"
    );
    let middle = loaded
        .image
        .to_rgba8()
        .get_pixel(loaded.width / 2, loaded.height / 2)
        .0;
    assert_eq!(middle, [255, 0, 0, 255]);
    let thumb = load_thumbnail(svgz.clone(), 64).await.expect("thumb");
    assert_eq!((thumb.width, thumb.height), (64, 32));

    let details = read_details(&svgz);
    assert_eq!(details.format, Some("SVG"));
    assert_eq!(details.drawing_size, Some((40, 20)));
}

/// Upstream's `MimeType=`, which the fork keeps as it is.
const UPSTREAM: [&str; 18] = [
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/bmp",
    "image/tiff",
    "image/x-icon",
    "image/avif",
    "image/heif",
    "image/heic",
    "image/jxl",
    "image/svg+xml",
    "image/x-portable-pixmap",
    "image/x-portable-graymap",
    "image/x-portable-bitmap",
    "image/x-portable-anymap",
    "image/x-qoi",
    "image/vnd.radiance",
];

/// Every type the viewer declares: upstream's, compressed SVG and the RAW ones.
fn expected() -> BTreeSet<&'static str> {
    UPSTREAM
        .into_iter()
        .chain(std::iter::once("image/svg+xml-compressed"))
        .chain(RAW_TYPES.iter().map(|(_, mime)| *mime))
        .collect()
}

fn resource(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../res")
            .join(name),
    )
    .unwrap()
}

#[test]
fn every_type_opened_is_declared_and_copied_under_its_own_name() {
    let entry = resource("fun.wmde.Viewer.desktop");
    let mime_types = entry
        .lines()
        .find_map(|line| line.strip_prefix("MimeType="))
        .unwrap();
    let declared: BTreeSet<&str> = mime_types.split(';').filter(|m| !m.is_empty()).collect();
    assert_eq!(declared, expected(), "MimeType=");
    let metainfo = resource("fun.wmde.Viewer.metainfo.xml");
    let provided: BTreeSet<&str> = metainfo
        .split("<mediatype>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</mediatype>").map(|(mime, _)| mime))
        .collect();
    assert_eq!(provided, expected(), "the metainfo");

    assert_eq!(image_mime_type("svgz"), Some("image/svg+xml-compressed"));
    for (extension, mime) in RAW_TYPES {
        assert_eq!(image_mime_type(extension), Some(mime));
        assert!(is_supported_image(Path::new(&format!(
            "a.{}",
            extension.to_uppercase()
        ))));
    }
    assert!(!is_supported_image(Path::new("IMG_0001.cr3")));
    // wmde-photos took `.jpe` into its album too.
    assert!(is_supported_image(Path::new("scan.JPE")));
    assert_eq!(image_mime_type("jpe"), Some("image/jpeg"));
}

#[test]
fn the_lossless_fixture_is_lossless() {
    // Otherwise the test that such a JPEG is passed over would prove nothing.
    let header = turbojpeg::read_header(&jpeg(16, 16, RED, BLUE, true)).unwrap();
    assert!(header.is_lossless);
}

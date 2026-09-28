// SPDX-License-Identifier: GPL-3.0-only

// WMDE: a picture larger than memory is refused from its header, before a pixel is read, and
// the texture of a large picture is made without a full RGBA copy of it.

use cosmic::widget::image::Handle;
use image::{DynamicImage, ImageBuffer, Rgb};
use std::path::{Path, PathBuf};
use viewer_core::{LoadError, load_image, load_thumbnail};

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

/// A PNG whose header announces a million pixels square at 16 bits RGBA, 8 TB decoded, and
/// carries no pixels: whatever tries to decode it allocates first.
fn enormous_png(dir: &Path) -> PathBuf {
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(&body);
        chunk.extend_from_slice(&crc32(&body).to_be_bytes());
        chunk
    };
    let mut header = 1_000_000_u32.to_be_bytes().to_vec();
    header.extend_from_slice(&1_000_000_u32.to_be_bytes());
    header.extend_from_slice(&[16, 6, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &header));
    bytes.extend(chunk(b"IDAT", &[]));
    bytes.extend(chunk(b"IEND", &[]));
    let path = dir.join("enormous.png");
    std::fs::write(&path, bytes).unwrap();
    path
}

#[tokio::test]
async fn a_picture_larger_than_memory_is_refused_from_its_header() {
    let dir = tempfile::tempdir().unwrap();
    let png = enormous_png(dir.path());
    match load_image(png.clone()).await {
        Err(LoadError::TooLarge { needed, available }) => {
            assert!(needed >= 8_000_000_000_000, "needs {needed}");
            assert!(available < needed);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
    assert!(matches!(
        load_thumbnail(png, 128).await,
        Err(LoadError::TooLarge { .. })
    ));
}

#[tokio::test]
async fn the_texture_of_a_large_picture_is_capped_and_in_rgba() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.png");
    DynamicImage::ImageRgb16(ImageBuffer::from_pixel(4100, 10, Rgb([65535_u16, 0, 0])))
        .save(&path)
        .unwrap();
    let loaded = load_image(path).await.expect("load");
    assert_eq!(
        (loaded.width, loaded.height),
        (4100, 10),
        "the picture is whole"
    );
    assert_eq!(loaded.image.color(), image::ColorType::Rgb16);
    let Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = &loaded.handle
    else {
        panic!("the texture is RGBA pixels");
    };
    assert_eq!((*width, *height), (2048, 5));
    assert_eq!(pixels.len(), 2048 * 5 * 4);
    assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
}

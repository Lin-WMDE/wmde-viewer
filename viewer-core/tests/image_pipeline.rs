// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;
use viewer_core::{load_image, load_thumbnail};

fn test_images_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../test_images"))
}

#[tokio::test]
async fn load_jpeg_has_valid_dimensions() {
    let path = test_images_dir().join("1315754.jpg");
    let loaded = load_image(path.clone()).await.expect("should load JPEG");
    assert!(loaded.width > 0);
    assert!(loaded.height > 0);
    assert_eq!(loaded.path, path);
}

#[tokio::test]
async fn load_png_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.png");
    let loaded = load_image(path.clone()).await.expect("should load PNG");
    assert!(loaded.width > 0);
    assert!(loaded.height > 0);
    assert_eq!(loaded.path, path);
}

#[tokio::test]
async fn load_webp_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.webp");
    let loaded = load_image(path.clone()).await.expect("should load WebP");
    assert!(loaded.width > 0);
    assert!(loaded.height > 0);
}

#[tokio::test]
async fn load_nonexistent_file_returns_error() {
    let path = test_images_dir().join("no_such_file_at_all.jpg");
    assert!(load_image(path).await.is_err());
}

#[tokio::test]
async fn load_non_image_file_returns_error() {
    let tmp = std::env::temp_dir().join("cosmic-viewer-test-not-image.txt");
    std::fs::write(&tmp, b"this is not an image").unwrap();
    let result = load_image(tmp.clone()).await;
    assert!(result.is_err());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn thumbnail_smaller_than_original() {
    let path = test_images_dir().join("1315754.jpg");
    let full = load_image(path.clone()).await.expect("full load");
    let thumb = load_thumbnail(path, 128).await.expect("thumb load");

    let full_pixels = u64::from(full.width) * u64::from(full.height);
    let thumb_pixels = u64::from(thumb.width) * u64::from(thumb.height);
    assert!(thumb_pixels < full_pixels);
    assert!(thumb.width <= 128);
    assert!(thumb.height <= 128);
}

#[tokio::test]
async fn load_same_image_twice_deterministic() {
    let path = test_images_dir().join("1315754.jpg");
    let a = load_image(path.clone()).await.expect("first load");
    let b = load_image(path).await.expect("second load");
    assert_eq!(a.width, b.width);
    assert_eq!(a.height, b.height);
}

#[tokio::test]
async fn load_bmp_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.bmp");
    let loaded = load_image(path).await.expect("should load BMP");
    assert!(loaded.width > 0 && loaded.height > 0);
}

#[tokio::test]
async fn load_gif_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.gif");
    let loaded = load_image(path).await.expect("should load GIF");
    assert!(loaded.width > 0 && loaded.height > 0);
}

#[tokio::test]
async fn load_tiff_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.tiff");
    let loaded = load_image(path).await.expect("should load TIFF");
    assert!(loaded.width > 0 && loaded.height > 0);
}

#[tokio::test]
async fn load_ico_has_valid_dimensions() {
    let path = test_images_dir().join("test_format.ico");
    let loaded = load_image(path).await.expect("should load ICO");
    assert!(loaded.width > 0 && loaded.height > 0);
}

#[tokio::test]
async fn thumbnail_at_multiple_sizes() {
    let path = test_images_dir().join("1315754.jpg");
    for max in [64, 128, 256] {
        let thumb = load_thumbnail(path.clone(), max)
            .await
            .unwrap_or_else(|e| panic!("thumb at {max} failed: {e}"));
        assert!(thumb.width <= max && thumb.height <= max);
        assert!(thumb.width > 0 && thumb.height > 0);
    }
}

#[tokio::test]
async fn thumbnail_preserves_aspect_ratio() {
    let path = test_images_dir().join("1315754.jpg");
    let full = load_image(path.clone()).await.expect("full");
    let thumb = load_thumbnail(path, 256).await.expect("thumb");

    let full_ratio = f64::from(full.width) / f64::from(full.height);
    let thumb_ratio = f64::from(thumb.width) / f64::from(thumb.height);
    assert!(
        (full_ratio - thumb_ratio).abs() < 0.05,
        "aspect ratio diverged: full={full_ratio:.3}, thumb={thumb_ratio:.3}",
    );
}

// The orientation_6 fixtures store two halves side by side as the sensor saw them, with EXIF
// Orientation 6 ("rotate 90 degrees clockwise to view"): seen the right way up, the left half
// is on top. The embedded EXIF thumbnail of orientation_6_exif_thumb.jpg is green over yellow
// instead of red over blue, so a thumbnail test also proves which of the two it was given.
// test_images/README.md says how they are made.

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const YELLOW: [u8; 3] = [255, 255, 0];

fn assert_upright(
    loaded: &viewer_core::LoadedImage,
    width: u32,
    height: u32,
    top: [u8; 3],
    bottom: [u8; 3],
) {
    assert_eq!(
        (loaded.width, loaded.height),
        (width, height),
        "size after orientation"
    );
    let rgba = loaded.image.to_rgba8();
    assert_eq!(
        rgba.dimensions(),
        (width, height),
        "the stored image is turned too"
    );
    // JPEG is lossy: compare within a margin, at the middle of each half.
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
async fn jpeg_is_turned_by_its_exif_orientation() {
    let loaded = load_image(test_images_dir().join("orientation_6.jpg"))
        .await
        .expect("load");
    assert_upright(&loaded, 32, 64, RED, BLUE);
}

#[tokio::test]
async fn png_is_turned_by_its_exif_chunk() {
    let loaded = load_image(test_images_dir().join("orientation_6.png"))
        .await
        .expect("load");
    assert_upright(&loaded, 32, 64, RED, BLUE);
}

#[tokio::test]
async fn scaled_jpeg_thumbnail_is_turned() {
    // 32 makes calculate_jpeg_scale pick 1/2 for the 64 px fixture: 32x16 decoded, 16x32 seen.
    // A target above the image's own size would fall through to 1/8 there, which is a separate
    // matter from orientation.
    let path = test_images_dir().join("orientation_6.jpg");
    let thumb = load_thumbnail(path, 32).await.expect("thumb");
    assert_upright(&thumb, 16, 32, RED, BLUE);
}

#[tokio::test]
async fn embedded_exif_thumbnail_is_turned() {
    let path = test_images_dir().join("orientation_6_exif_thumb.jpg");
    let thumb = load_thumbnail(path.clone(), 128).await.expect("thumb");
    assert_upright(&thumb, 16, 32, GREEN, YELLOW);
    let full = load_image(path).await.expect("full");
    assert_upright(&full, 128, 256, RED, BLUE);
}

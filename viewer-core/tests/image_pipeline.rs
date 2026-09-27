// SPDX-License-Identifier: GPL-3.0-only

use image::ImageEncoder;
use std::path::{Path, PathBuf};
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

// WMDE: AVIF is read by libheif. The `image` crate here carries only the AVIF encoder, which
// is what makes the fixture: red over blue, 32 by 64, no transform.
fn write_avif(dir: &Path) -> PathBuf {
    let mut img = image::RgbImage::from_pixel(32, 64, image::Rgb(BLUE));
    for y in 0..32 {
        for x in 0..32 {
            img.put_pixel(x, y, image::Rgb(RED));
        }
    }
    let path = dir.join("red_over_blue.avif");
    let file = std::fs::File::create(&path).unwrap();
    image::codecs::avif::AvifEncoder::new_with_speed_quality(file, 10, 100)
        .write_image(img.as_raw(), 32, 64, image::ExtendedColorType::Rgb8)
        .unwrap();
    path
}

#[tokio::test]
async fn avif_is_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_image(write_avif(dir.path()))
        .await
        .expect("AVIF should decode");
    assert_upright(&loaded, 32, 64, RED, BLUE);
}

#[tokio::test]
async fn avif_thumbnail_is_scaled() {
    let dir = tempfile::tempdir().unwrap();
    let thumb = load_thumbnail(write_avif(dir.path()), 32)
        .await
        .expect("AVIF thumbnail");
    assert_upright(&thumb, 16, 32, RED, BLUE);
}

// WMDE: a file handed over under a name no decoder knows is decoded by its content.
#[tokio::test]
async fn an_unknown_extension_opens_by_its_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.download");
    std::fs::copy(test_images_dir().join("orientation_6.jpg"), &path).unwrap();
    let loaded = load_image(path.clone())
        .await
        .expect("JPEG under .download");
    assert_upright(&loaded, 32, 64, RED, BLUE);
    let thumb = load_thumbnail(path, 32).await.expect("its thumbnail");
    assert_upright(&thumb, 16, 32, RED, BLUE);
}

#[tokio::test]
async fn a_name_without_an_extension_opens_by_its_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screenshot");
    std::fs::copy(test_images_dir().join("orientation_6.png"), &path).unwrap();
    let loaded = load_image(path.clone())
        .await
        .expect("PNG without an extension");
    assert_upright(&loaded, 32, 64, RED, BLUE);
    let thumb = load_thumbnail(path, 32).await.expect("its thumbnail");
    assert_upright(&thumb, 16, 32, RED, BLUE);
}

// WMDE: HEIF and AVIF are recognised by their container, whatever the name.
#[tokio::test]
async fn avif_under_another_name_or_none_is_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let avif = write_avif(dir.path());
    for name in ["served_as.jpg", "no_extension"] {
        let path = dir.path().join(name);
        std::fs::copy(&avif, &path).unwrap();
        let loaded = load_image(path.clone()).await.expect(name);
        assert_upright(&loaded, 32, 64, RED, BLUE);
        let thumb = load_thumbnail(path, 32).await.expect(name);
        assert_upright(&thumb, 16, 32, RED, BLUE);
    }
}

#[tokio::test]
async fn a_jpeg_named_avif_opens_as_a_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_really.avif");
    std::fs::copy(test_images_dir().join("orientation_6.jpg"), &path).unwrap();
    let loaded = load_image(path).await.expect("JPEG under .avif");
    assert_upright(&loaded, 32, 64, RED, BLUE);
}

// WMDE: TIFF is the container of most camera RAW formats, and their first image is a small
// preview; under a RAW name it is refused rather than shown as the photograph.
#[tokio::test]
async fn tiff_content_under_a_raw_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let tiff = dir.path().join("scan.tiff");
    image::RgbImage::from_pixel(8, 6, image::Rgb(RED))
        .save(&tiff)
        .unwrap();
    let raw = dir.path().join("DSC_0001.nef");
    std::fs::copy(&tiff, &raw).unwrap();
    assert!(load_image(raw.clone()).await.is_err());
    assert!(load_thumbnail(raw, 32).await.is_err());

    let bare = dir.path().join("scan");
    std::fs::copy(&tiff, &bare).unwrap();
    assert_eq!(
        load_image(bare).await.expect("TIFF without a name").width,
        8
    );
}

// WMDE: a picture is read in its own depth and channel layout, so saving it again keeps them.
fn write_png(dir: &Path, name: &str, pixels: &[u8], color: image::ExtendedColorType) -> PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(pixels, 2, 1, color)
        .unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

#[tokio::test]
async fn png_keeps_its_depth_and_channels() {
    let dir = tempfile::tempdir().unwrap();
    // Two 16-bit RGB pixels: values 8-bit would round away.
    let deep: Vec<u8> = [1_u16, 258, 65535, 30000, 7, 12345]
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect();
    let path = write_png(
        dir.path(),
        "deep.png",
        &deep,
        image::ExtendedColorType::Rgb16,
    );
    let loaded = load_image(path).await.expect("16-bit PNG");
    let image::DynamicImage::ImageRgb16(pixels) = &loaded.image else {
        panic!("16-bit RGB PNG read as {:?}", loaded.image.color());
    };
    assert_eq!(pixels.get_pixel(0, 0).0, [1, 258, 65535]);
    assert_eq!(pixels.get_pixel(1, 0).0, [30000, 7, 12345]);

    let grey = write_png(
        dir.path(),
        "grey.png",
        &[10, 200],
        image::ExtendedColorType::L8,
    );
    let loaded = load_image(grey).await.expect("grey PNG");
    assert_eq!(loaded.image.color(), image::ColorType::L8);

    let rgb = write_png(
        dir.path(),
        "rgb.png",
        &[1, 2, 3, 4, 5, 6],
        image::ExtendedColorType::Rgb8,
    );
    let loaded = load_image(rgb).await.expect("RGB PNG");
    assert_eq!(loaded.image.color(), image::ColorType::Rgb8);
}

#[tokio::test]
async fn jpeg_keeps_grey_and_holds_no_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let grey = dir.path().join("grey.jpg");
    image::GrayImage::from_pixel(16, 16, image::Luma([90]))
        .save(&grey)
        .unwrap();
    let loaded = load_image(grey).await.expect("grey JPEG");
    assert_eq!(loaded.image.color(), image::ColorType::L8);

    let colour = dir.path().join("colour.jpg");
    image::RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 30]))
        .save(&colour)
        .unwrap();
    let loaded = load_image(colour).await.expect("colour JPEG");
    assert_eq!(loaded.image.color(), image::ColorType::Rgb8);
}

#[tokio::test]
async fn avif_without_alpha_is_read_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_image(write_avif(dir.path())).await.expect("AVIF");
    assert_eq!(loaded.image.color(), image::ColorType::Rgb8);
}

// WMDE: a PNM whose maxval is neither 255 nor 65535 is scaled to the full range, not shown
// almost black.
#[tokio::test]
async fn pnm_samples_are_scaled_by_maxval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ten_bit.pgm");
    let mut bytes = b"P5\n2 1\n1023\n".to_vec();
    bytes.extend_from_slice(&1023_u16.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    std::fs::write(&path, bytes).unwrap();
    let loaded = load_image(path).await.expect("10-bit PGM");
    let image::DynamicImage::ImageLuma16(pixels) = &loaded.image else {
        panic!("10-bit PGM read as {:?}", loaded.image.color());
    };
    assert_eq!(pixels.get_pixel(0, 0).0, [65535]);
    assert_eq!(pixels.get_pixel(1, 0).0, [0]);
}

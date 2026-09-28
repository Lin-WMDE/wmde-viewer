// SPDX-License-Identifier: GPL-3.0-only

// WMDE: save_image replaces a file only with a complete picture.

use image::{DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use viewer_core::{keeps_depth, save_image};

const ORIGINAL: &[u8] = b"the original bytes, which a failed save must leave alone";

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_refused_encode_leaves_the_original_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picture.ico");
    fs::write(&path, ORIGINAL).unwrap();
    // The ICO encoder refuses a side over 256 pixels, and `DynamicImage::save` has truncated
    // the file by the time it asks.
    let image = DynamicImage::ImageRgb16(ImageBuffer::from_pixel(300, 8, Rgb([60000u16, 0, 0])));
    assert!(save_image(&image, &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), ORIGINAL);
    assert_eq!(
        names(dir.path()),
        ["picture.ico"],
        "no temporary file is left"
    );
}

#[test]
fn an_extension_without_an_encoder_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picture.heic");
    fs::write(&path, ORIGINAL).unwrap();
    assert!(save_image(&DynamicImage::new_rgba8(8, 8), &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), ORIGINAL);
    assert_eq!(names(dir.path()), ["picture.heic"]);
}

// WMDE: a camera RAW file is shown through its preview, and saving that over the file would
// throw the sensor data away. No encoder takes a RAW name, so the save is refused.
#[test]
fn a_camera_raw_file_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    for (extension, _) in viewer_core::raw::RAW_TYPES {
        let path = dir.path().join(format!("photo.{extension}"));
        fs::write(&path, ORIGINAL).unwrap();
        assert!(
            save_image(&DynamicImage::new_rgb8(8, 8), &path).is_err(),
            "{extension}"
        );
        assert_eq!(fs::read(&path).unwrap(), ORIGINAL, "{extension}");
        fs::remove_file(&path).unwrap();
    }
    assert!(names(dir.path()).is_empty(), "no temporary file is left");
}

#[test]
fn a_saved_picture_replaces_the_file_and_keeps_its_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picture.png");
    fs::write(&path, ORIGINAL).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 2, Rgba([10, 20, 30, 255])));
    save_image(&image, &path).expect("save");
    let back = image::open(&path).unwrap().to_rgba8();
    assert_eq!(back.dimensions(), (4, 2));
    assert_eq!(back.get_pixel(0, 0).0, [10, 20, 30, 255]);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(names(dir.path()), ["picture.png"]);
}

#[test]
fn a_new_file_gets_the_mode_any_new_file_would() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("reference");
    fs::File::create(&reference).unwrap();
    let path = dir.path().join("new.png");
    save_image(&DynamicImage::new_rgba8(2, 2), &path).expect("save");
    assert_eq!(mode(&path), mode(&reference));
}

#[test]
fn a_symbolic_link_stays_a_link_and_its_target_gets_the_picture() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.png");
    fs::write(&target, ORIGINAL).unwrap();
    let link = dir.path().join("link.png");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    save_image(&DynamicImage::new_rgba8(3, 3), &link).expect("save");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(image::open(&target).unwrap().width(), 3);
}

#[test]
fn an_annotated_picture_goes_into_a_jpeg() {
    // Annotations leave the picture RGBA8, and JPEG carries no alpha.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([200, 0, 0, 255])));
    save_image(&image, &path).expect("save");
    assert_eq!(image::open(&path).unwrap().width(), 8);
}

/// Root, or anything with `CAP_DAC_OVERRIDE`, writes through missing permissions, so the
/// permission tests have nothing to show there.
fn permissions_are_enforced(dir: &Path) -> bool {
    let probe = dir.join("permission-probe");
    fs::write(&probe, b"").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o444)).unwrap();
    let enforced = fs::OpenOptions::new().write(true).open(&probe).is_err();
    fs::remove_file(&probe).unwrap();
    enforced
}

#[test]
fn a_file_without_write_permission_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    if !permissions_are_enforced(dir.path()) {
        eprintln!("skipped: permissions are not enforced for this user");
        return;
    }
    let path = dir.path().join("protected.png");
    fs::write(&path, ORIGINAL).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(save_image(&DynamicImage::new_rgba8(2, 2), &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), ORIGINAL);
    assert_eq!(names(dir.path()), ["protected.png"]);
}

#[test]
fn a_file_with_a_second_hard_link_is_written_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picture.png");
    let other = dir.path().join("other-name.png");
    fs::write(&path, ORIGINAL).unwrap();
    fs::hard_link(&path, &other).unwrap();
    save_image(&DynamicImage::new_rgba8(5, 4), &path).expect("save");
    assert_eq!(image::open(&other).unwrap().width(), 5, "both names see it");
    assert_eq!(names(dir.path()), ["other-name.png", "picture.png"]);
}

#[test]
fn a_writable_file_in_a_closed_directory_is_written_in_place() {
    let dir = tempfile::tempdir().unwrap();
    if !permissions_are_enforced(dir.path()) {
        eprintln!("skipped: permissions are not enforced for this user");
        return;
    }
    let closed = dir.path().join("closed");
    fs::create_dir(&closed).unwrap();
    let path = closed.join("picture.png");
    fs::write(&path, ORIGINAL).unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o555)).unwrap();
    let saved = save_image(&DynamicImage::new_rgba8(6, 2), &path);
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();
    saved.expect("save");
    assert_eq!(image::open(&path).unwrap().width(), 6);
    assert_eq!(names(&closed), ["picture.png"]);
}

#[test]
fn a_link_to_a_file_not_yet_there_creates_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("exported.png");
    let link = dir.path().join("link.png");
    std::os::unix::fs::symlink("exported.png", &link).unwrap();
    save_image(&DynamicImage::new_rgba8(3, 3), &link).expect("save");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(image::open(&target).unwrap().width(), 3);
}

// WMDE: a save over the picture's own file refuses a format that would lower its depth.
#[test]
fn a_format_keeps_the_depth_of_a_picture_or_does_not() {
    let deep = DynamicImage::new_rgb16(2, 2);
    let flat = DynamicImage::new_rgb8(2, 2);
    assert!(keeps_depth(&deep, Path::new("a.png")));
    assert!(keeps_depth(&deep, Path::new("a.tiff")));
    assert!(!keeps_depth(&deep, Path::new("a.avif")));
    assert!(!keeps_depth(
        &DynamicImage::new_luma16(2, 2),
        Path::new("a.jpg")
    ));
    assert!(keeps_depth(&flat, Path::new("a.avif")));
    assert!(keeps_depth(&flat, Path::new("a.jpg")));
}

#[test]
fn every_encoder_that_holds_the_depth_counts_and_one_that_is_missing_is_not_blamed() {
    let deep = DynamicImage::new_rgb16(2, 2);
    let float = DynamicImage::new_rgb32f(2, 2);
    assert!(keeps_depth(&deep, Path::new("a.pgm")));
    assert!(keeps_depth(
        &DynamicImage::new_rgba16(2, 2),
        Path::new("a.ff")
    ));
    assert!(keeps_depth(&float, Path::new("a.hdr")));
    assert!(keeps_depth(&float, Path::new("a.exr")));
    assert!(keeps_depth(&float, Path::new("a.tiff")));
    assert!(!keeps_depth(&float, Path::new("a.png")));
    // No encoder: save_image refuses those with its own reason.
    assert!(keeps_depth(&deep, Path::new("a.heic")));
    assert!(keeps_depth(&float, Path::new("a.jxl")));
}

// WMDE: a picture reaches the encoder in a type it takes: the loader keeps grey, 16-bit and
// float pictures as they are, and GIF, TIFF and PNG refuse some of those.
#[test]
fn a_picture_is_brought_to_a_type_the_encoder_takes() {
    let dir = tempfile::tempdir().unwrap();

    let grey = dir.path().join("grey.gif");
    save_image(&DynamicImage::new_luma8(3, 2), &grey).expect("grey as GIF");
    assert_eq!(image::open(&grey).unwrap().width(), 3);

    let grey_alpha = dir.path().join("grey_alpha.tiff");
    save_image(&DynamicImage::new_luma_a8(3, 2), &grey_alpha).expect("grey with alpha as TIFF");
    assert!(image::open(&grey_alpha).unwrap().color().has_alpha());

    let float = dir.path().join("float.png");
    save_image(&DynamicImage::new_rgb32f(3, 2), &float).expect("float as PNG");
    assert_eq!(
        image::open(&float).unwrap().color(),
        image::ColorType::Rgb16
    );
}

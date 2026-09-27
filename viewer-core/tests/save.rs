// SPDX-License-Identifier: GPL-3.0-only

// WMDE: save_image replaces a file only with a complete picture.

use image::{DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use viewer_core::save_image;

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
    let path = dir.path().join("picture.gif");
    fs::write(&path, ORIGINAL).unwrap();
    // The GIF encoder refuses 16-bit pixels, and `DynamicImage::save` has truncated the file
    // by the time it asks.
    let image = DynamicImage::ImageRgb16(ImageBuffer::from_pixel(8, 8, Rgb([60000u16, 0, 0])));
    assert!(save_image(&image, &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), ORIGINAL);
    assert_eq!(
        names(dir.path()),
        ["picture.gif"],
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

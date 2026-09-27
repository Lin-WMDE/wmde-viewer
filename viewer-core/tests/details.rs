// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the details drawer's facts, read from pictures written here with the EXIF they need.

use exif::{Field, In, Rational, Tag, Value};
use image::{ImageEncoder, RgbImage};
use std::path::{Path, PathBuf};
use viewer_core::details::{Exposure, Flash, FlashMode, Photograph, Taken};
use viewer_core::read_details;

fn ascii(tag: Tag, text: &str) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![text.as_bytes().to_vec()]),
    }
}

fn rationals(tag: Tag, values: &[(u32, u32)]) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value: Value::Rational(
            values
                .iter()
                .map(|&(num, denom)| Rational { num, denom })
                .collect(),
        ),
    }
}

fn short(tag: Tag, value: u16) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value: Value::Short(vec![value]),
    }
}

fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .write_image(
            RgbImage::from_pixel(8, 8, image::Rgb([200, 40, 40])).as_raw(),
            8,
            8,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}

/// A JPEG carrying `fields` in an APP1 segment right after its start marker.
fn jpeg_with_exif(dir: &Path, name: &str, fields: &[Field]) -> PathBuf {
    let mut writer = exif::experimental::Writer::new();
    for field in fields {
        writer.push_field(field);
    }
    let mut tiff = std::io::Cursor::new(Vec::new());
    writer.write(&mut tiff, false).unwrap();
    let tiff = tiff.into_inner();

    let jpeg = jpeg();
    let length = u16::try_from(2 + 6 + tiff.len()).unwrap();
    let mut bytes = jpeg[..2].to_vec();
    bytes.extend_from_slice(&[0xFF, 0xE1]);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(b"Exif\0\0");
    bytes.extend_from_slice(&tiff);
    bytes.extend_from_slice(&jpeg[2..]);

    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[0; 4 * 3], 2, 2, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn photograph(dir: &Path, fields: &[Field]) -> Photograph {
    read_details(&jpeg_with_exif(dir, "shot.jpg", fields))
        .exif
        .expect("the picture carries EXIF")
}

#[test]
fn a_photograph_says_when_with_what_and_where() {
    let dir = tempfile::tempdir().unwrap();
    let shot = photograph(
        dir.path(),
        &[
            ascii(Tag::Make, "NIKON CORPORATION"),
            ascii(Tag::Model, "NIKON D750"),
            ascii(Tag::LensModel, "50.0 mm f/1.8"),
            ascii(Tag::DateTimeOriginal, "2023:05:01 12:34:56"),
            rationals(Tag::ExposureTime, &[(1, 250)]),
            rationals(Tag::FNumber, &[(28, 10)]),
            short(Tag::PhotographicSensitivity, 400),
            rationals(Tag::FocalLength, &[(50, 1)]),
            short(Tag::Flash, 0x10),
            ascii(Tag::GPSLatitudeRef, "N"),
            rationals(Tag::GPSLatitude, &[(50, 1), (27, 1), (0, 1)]),
            ascii(Tag::GPSLongitudeRef, "E"),
            rationals(Tag::GPSLongitude, &[(30, 1), (31, 1), (12, 1)]),
            ascii(Tag::Software, "Ver.1.10"),
            ascii(Tag::Artist, "Taras"),
            ascii(Tag::ImageDescription, "Kyiv"),
            ascii(Tag::Copyright, "CC BY 4.0"),
        ],
    );

    assert_eq!(
        shot.taken,
        Some(Taken::Clock {
            year: 2023,
            month: 5,
            day: 1,
            hour: 12,
            minute: 34,
            second: 56,
        })
    );
    assert_eq!(shot.camera.as_deref(), Some("NIKON D750"));
    assert_eq!(shot.lens.as_deref(), Some("50.0 mm f/1.8"));
    assert_eq!(shot.exposure, Some(Exposure::Fraction(250.0)));
    assert_eq!(shot.aperture, Some(2.8));
    assert_eq!(shot.iso, Some(400));
    assert_eq!(shot.focal_length, Some(50.0));
    assert_eq!(
        shot.flash,
        Some(Flash {
            fired: Some(false),
            mode: Some(FlashMode::Suppressed),
            red_eye_reduction: false,
        })
    );
    let latitude = shot.latitude.unwrap();
    assert!((latitude - 50.45).abs() < 1e-9, "{latitude}");
    let longitude = shot.longitude.unwrap();
    assert!((longitude - 30.52).abs() < 1e-9, "{longitude}");
    assert_eq!(shot.software.as_deref(), Some("Ver.1.10"));
    assert_eq!(shot.author.as_deref(), Some("Taras"));
    assert_eq!(shot.description.as_deref(), Some("Kyiv"));
    assert_eq!(shot.copyright.as_deref(), Some("CC BY 4.0"));
}

#[test]
fn file_facts_come_with_the_photograph() {
    let dir = tempfile::tempdir().unwrap();
    let path = jpeg_with_exif(
        dir.path(),
        "shot.jpg",
        &[rationals(Tag::XResolution, &[(300, 1)])],
    );
    let details = read_details(&path);
    assert_eq!(details.size, Some(std::fs::metadata(&path).unwrap().len()));
    assert!(details.modified.is_some());
    assert_eq!(details.format, Some("JPEG"));
    assert_eq!(details.dpi, Some(300));
    assert_eq!(details.exif, Some(Photograph::default()));
}

#[test]
fn a_picture_without_exif_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let details = read_details(&png(dir.path(), "plain.png"));
    assert_eq!(details.exif, None);
    assert_eq!(details.format, Some("PNG"));
}

#[test]
fn a_maker_is_named_once() {
    let dir = tempfile::tempdir().unwrap();
    let camera = |make: &str, model: &str| {
        photograph(
            dir.path(),
            &[ascii(Tag::Make, make), ascii(Tag::Model, model)],
        )
        .camera
    };
    assert_eq!(
        camera("Canon", "Canon EOS 5D").as_deref(),
        Some("Canon EOS 5D")
    );
    assert_eq!(camera("SONY", "ILCE-7M3").as_deref(), Some("SONY ILCE-7M3"));
    let only_make = photograph(dir.path(), &[ascii(Tag::Make, "Apple")]).camera;
    assert_eq!(only_make.as_deref(), Some("Apple"));
}

#[test]
fn exposures_read_as_a_camera_shows_them() {
    let dir = tempfile::tempdir().unwrap();
    let exposure = |num, denom| {
        photograph(dir.path(), &[rationals(Tag::ExposureTime, &[(num, denom)])]).exposure
    };
    assert_eq!(exposure(10, 300), Some(Exposure::Fraction(30.0)));
    assert_eq!(exposure(1, 2), Some(Exposure::Fraction(2.0)));
    // An APEX-rounded 1/15, and third stops written as tenths.
    assert_eq!(exposure(667, 10000), Some(Exposure::Fraction(15.0)));
    assert_eq!(exposure(10, 25), Some(Exposure::Fraction(2.5)));
    assert_eq!(exposure(10, 16), Some(Exposure::Fraction(1.6)));
    assert_eq!(exposure(10, 13), Some(Exposure::Fraction(1.3)));
    // Times that are no 1/n stay in seconds rather than turning into the next stop.
    assert_eq!(exposure(6, 10), Some(Exposure::Seconds(0.6)));
    assert_eq!(exposure(3, 10), Some(Exposure::Seconds(0.3)));
    assert_eq!(exposure(2, 1), Some(Exposure::Seconds(2.0)));
    assert_eq!(exposure(0, 1), None);
    assert_eq!(exposure(1, 0), None);
}

#[test]
fn the_flash_fired_did_not_or_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let flash = |value| photograph(dir.path(), &[short(Tag::Flash, value)]).flash;
    let flash_of = |fired, mode, red_eye_reduction| {
        Some(Flash {
            fired,
            mode,
            red_eye_reduction,
        })
    };
    assert_eq!(
        flash(0x19),
        flash_of(Some(true), Some(FlashMode::Auto), false)
    );
    assert_eq!(
        flash(0x09),
        flash_of(Some(true), Some(FlashMode::Forced), false)
    );
    assert_eq!(
        flash(0x10),
        flash_of(Some(false), Some(FlashMode::Suppressed), false)
    );
    assert_eq!(
        flash(0x59),
        flash_of(Some(true), Some(FlashMode::Auto), true)
    );
    assert_eq!(flash(0x00), flash_of(Some(false), None, false));
    assert_eq!(flash(0x20), flash_of(None, None, false));
}

#[test]
fn south_and_west_are_negative_and_a_coordinate_needs_its_hemisphere() {
    let dir = tempfile::tempdir().unwrap();
    let shot = photograph(
        dir.path(),
        &[
            ascii(Tag::GPSLatitudeRef, "S"),
            rationals(Tag::GPSLatitude, &[(33, 1), (51, 1), (54, 1)]),
            rationals(Tag::GPSLongitude, &[(151, 1), (12, 1), (36, 1)]),
        ],
    );
    let latitude = shot.latitude.unwrap();
    assert!((latitude + 33.865).abs() < 1e-9, "{latitude}");
    assert_eq!(shot.longitude, None);

    let west = photograph(
        dir.path(),
        &[
            ascii(Tag::GPSLongitudeRef, "W"),
            rationals(Tag::GPSLongitude, &[(0, 1), (7, 1), (39, 1)]),
        ],
    );
    let longitude = west.longitude.unwrap();
    assert!((longitude + 0.1275).abs() < 1e-9, "{longitude}");
}

#[test]
fn blank_text_is_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let shot = photograph(
        dir.path(),
        &[
            ascii(Tag::ImageDescription, "          "),
            ascii(Tag::DateTimeOriginal, "    :  :     :  :  "),
        ],
    );
    assert_eq!(shot.description, None);
    assert_eq!(shot.taken, None);

    let unset = photograph(
        dir.path(),
        &[ascii(Tag::DateTimeOriginal, "0000:00:00 00:00:00")],
    );
    assert_eq!(unset.taken, None);
}

#[test]
fn a_date_in_another_layout_is_shown_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let shot = photograph(
        dir.path(),
        &[ascii(Tag::DateTimeOriginal, "2023-05-01 12:34:56")],
    );
    assert_eq!(
        shot.taken,
        Some(Taken::Text("2023-05-01 12:34:56".to_owned()))
    );
}

#[test]
fn the_format_is_read_from_the_content() {
    let dir = tempfile::tempdir().unwrap();
    let details = read_details(&png(dir.path(), "named.jpg"));
    assert_eq!(details.format, Some("PNG"));

    let avif = dir.path().join("named.png");
    let file = std::fs::File::create(&avif).unwrap();
    image::codecs::avif::AvifEncoder::new_with_speed_quality(file, 10, 100)
        .write_image(
            &[255, 0, 0, 255].repeat(16),
            4,
            4,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    assert_eq!(read_details(&avif).format, Some("AVIF"));

    let svg = dir.path().join("drawing.svg");
    std::fs::write(
        &svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"16\"/>",
    )
    .unwrap();
    let details = read_details(&svg);
    assert_eq!(details.format, Some("SVG"));
    // The drawing's own size, not the texture it is rendered to.
    assert_eq!(details.drawing_size, Some((24, 16)));
}

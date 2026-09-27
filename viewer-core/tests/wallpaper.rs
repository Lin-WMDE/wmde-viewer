// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the background settings set_wallpaper writes, in a configuration of its own. One test
// function: the directories come from the environment, which every test in a binary shares.

use cosmic::cosmic_config::ConfigGet;
use image::RgbImage;
use std::{
    ffi::OsStr,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};
use viewer_core::{set_wallpaper, wallpaper::is_native};
use wmde_bg_config::{Config, Context, Entry, ScalingMode, Source};

fn write(path: &Path, format: image::ImageFormat) {
    RgbImage::from_pixel(6, 4, image::Rgb([10, 120, 200]))
        .save_with_format(path, format)
        .unwrap();
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

/// A PNG whose header announces 30000 x 30000 at 16 bits RGBA, 7.2 GB decoded; no pixels.
fn huge_png(path: &Path) {
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(&body);
        chunk.extend_from_slice(&crc32(&body).to_be_bytes());
        chunk
    };
    let mut header = 30000_u32.to_be_bytes().to_vec();
    header.extend_from_slice(&30000_u32.to_be_bytes());
    header.extend_from_slice(&[16, 6, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &header));
    bytes.extend(chunk(b"IDAT", &[]));
    bytes.extend(chunk(b"IEND", &[]));
    std::fs::write(path, bytes).unwrap();
}

fn source_of(context: &Context, key: &str) -> PathBuf {
    match context.entry(key).unwrap().source {
        Source::Path(path) => path,
        source @ Source::Color(_) => panic!("{key} holds {source:?}"),
    }
}

fn written(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|files| {
            files
                .filter_map(Result::ok)
                .map(|file| file.path())
                .filter(|file| file.extension().is_some_and(|ext| ext == "png"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

// reason: one scenario in order; the settings it builds up are the environment's, not a test's.
#[allow(clippy::too_many_lines)]
#[test]
fn backgrounds_point_at_a_file_wmde_bg_reads_and_keep_the_rest_of_their_entries() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", home.path().join("config"));
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
        std::env::set_var("XDG_DATA_DIRS", home.path().join("system"));
    }
    let wallpapers = home.path().join("data/wmde/viewer/wallpaper");
    let switch = home
        .path()
        .join("config/wmde/fun.wmde.Background/v1/same-on-all");
    let pictures = home.path().join("pictures");
    std::fs::create_dir_all(&pictures).unwrap();

    // What wmde-bg reads goes by content, whatever the name says, and within its limit.
    let png = pictures.join("sea.png");
    write(&png, image::ImageFormat::Png);
    let gif = pictures.join("forest.gif");
    write(&gif, image::ImageFormat::Gif);
    let bmp = pictures.join("forest.bmp");
    write(&bmp, image::ImageFormat::Bmp);
    let png_named_gif = pictures.join("really-png.gif");
    write(&png_named_gif, image::ImageFormat::Png);
    let huge = pictures.join("huge.png");
    huge_png(&huge);
    assert!(is_native(&png));
    assert!(is_native(&png_named_gif));
    assert!(!is_native(&gif));
    assert!(!is_native(&bmp));
    assert!(!is_native(&huge));

    // One picture on all displays, which also have entries of their own, fitted differently.
    let context = wmde_bg_config::context().unwrap();
    let mut config = Config::load(&context).unwrap();
    for (output, scaling_mode, rotation_frequency) in [
        ("all", ScalingMode::Zoom, 42),
        ("A", ScalingMode::Stretch, 7),
        ("B", ScalingMode::Stretch, 7),
    ] {
        let mut entry = Entry::new(output.to_owned(), Source::Path(PathBuf::from("/old.jpg")));
        entry.scaling_mode = scaling_mode;
        entry.rotation_frequency = rotation_frequency;
        config.set_entry(&context, entry).unwrap();
    }
    context.set_same_on_all(true).unwrap();
    let fitting = |key: &str| {
        let entry = context.entry(key).unwrap();
        (entry.scaling_mode, entry.rotation_frequency)
    };

    // All displays: `all` and each display's entry, only the source changed, the switch left
    // as it was, and the path given through `..` written as the file it names.
    let shown = set_wallpaper(&pictures.join("../pictures/sea.png"), None).unwrap();
    assert_eq!(shown, png.canonicalize().unwrap());
    for key in ["all", "output.A", "output.B"] {
        assert_eq!(source_of(&context, key), shown, "{key}");
    }
    assert_eq!(fitting("all"), (ScalingMode::Zoom, 42));
    assert_eq!(fitting("output.A"), (ScalingMode::Stretch, 7));
    assert!(context.same_on_all());
    let mut outputs = context.backgrounds();
    outputs.sort();
    assert_eq!(outputs, ["A", "B"]);
    assert!(written(&wallpapers).is_empty());

    // The same file again changes no entry; the switch is written again, so wmde-bg rereads.
    let before = std::fs::metadata(&switch).unwrap().ino();
    set_wallpaper(&png, None).unwrap();
    assert_ne!(std::fs::metadata(&switch).unwrap().ino(), before);

    // One display while the switch is on: it showed `all` and keeps that fitting, not its own
    // old entry's; the other display keeps showing `all`; a GIF becomes a PNG.
    let forest = set_wallpaper(&gif, Some("A")).unwrap();
    assert_eq!(forest.parent(), Some(wallpapers.as_path()));
    assert_eq!(
        image::ImageReader::open(&forest)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .format(),
        Some(image::ImageFormat::Png)
    );
    assert_eq!(image::image_dimensions(&forest).unwrap(), (6, 4));
    assert_eq!(source_of(&context, "output.A"), forest);
    assert_eq!(fitting("output.A"), (ScalingMode::Zoom, 42));
    assert_eq!(source_of(&context, "output.B"), shown);
    assert_eq!(fitting("output.B"), (ScalingMode::Zoom, 42));
    assert_eq!(source_of(&context, "all"), shown);
    assert!(!context.same_on_all());

    // The same picture again is not written again; another one does not remove the first
    // while display A still shows it.
    let modified = std::fs::metadata(&forest).unwrap().modified().unwrap();
    assert_eq!(set_wallpaper(&gif, Some("A")).unwrap(), forest);
    assert_eq!(
        std::fs::metadata(&forest).unwrap().modified().unwrap(),
        modified
    );
    let other = set_wallpaper(&bmp, Some("B")).unwrap();
    assert_ne!(other, forest);
    assert_eq!(written(&wallpapers), {
        let mut both = vec![forest, other];
        both.sort();
        both
    });

    // Once nothing points at them, the written files go; a save in progress stays.
    let part = wallpapers.join(".wmde-viewer-test.part");
    std::fs::write(&part, b"").unwrap();
    set_wallpaper(&png, None).unwrap();
    assert!(written(&wallpapers).is_empty());
    assert!(part.exists());

    // The settings offer the pictures wmde-bg reads as they are, not the written PNGs.
    let settings = cosmic::cosmic_config::Config::new("fun.wmde.Settings.Wallpaper", 1).unwrap();
    let offered: Vec<PathBuf> = settings.get("custom-images").unwrap();
    assert_eq!(offered, [shown]);

    // A name the settings cannot store as text, and one of 60 four-byte characters, are set
    // through a PNG whose name fits.
    let latin1 = pictures.join(OsStr::from_bytes(b"caf\xe9.png"));
    write(&latin1, image::ImageFormat::Png);
    let from_latin1 = set_wallpaper(&latin1, None).unwrap();
    assert_eq!(from_latin1.parent(), Some(wallpapers.as_path()));
    assert!(from_latin1.to_str().is_some());
    let trees = pictures.join(format!("{}.gif", "\u{1F332}".repeat(60)));
    write(&trees, image::ImageFormat::Gif);
    let from_trees = set_wallpaper(&trees, None).unwrap();
    assert!(from_trees.file_name().unwrap().len() <= 255);
}

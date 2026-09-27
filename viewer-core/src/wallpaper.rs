// SPDX-License-Identifier: GPL-3.0-only

// WMDE: making a picture the desktop background, the way wmde-photos did. Two things are not
// obvious, and both come from reading wmde-bg.
//
// The format. wmde-bg opens the file by its content with the `image` crate built for JPEG,
// PNG, WebP, HDR and AVIF, plus the jxl-oxide hook for JPEG XL, and refuses a picture above
// 1 GiB decoded. A file it cannot read gave a black desktop and no message, so anything it may
// not read is decoded here and written as a PNG of our own. That includes AVIF, whose
// container rotation its decoder ignores and libheif here applies.
//
// Where to write. `same-on-all` decides whether the daemon reads the `all` entry or the
// per-output ones. A picture for all displays is written to both, and the switch stays the
// person's; a picture for one display turns the switch off, which is what was asked. Everything
// in an entry except the source is kept: how a background is fitted is chosen in the settings.

use crate::{
    LoadError, SaveError,
    details::content_format,
    loader::{load_for_wallpaper, register_jpeg_xl},
    save_image,
};
use cosmic::cosmic_config::{self, ConfigGet, ConfigSet};
use image::{ImageDecoder, ImageReader};
use std::{
    collections::HashSet,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};
use thiserror::Error;
use wmde_bg_config::{Config, Context, Entry, Source};

/// Formats wmde-bg is given as they are, by content: the `image` features in
/// `forks/wmde-bg/Cargo.toml` and the JPEG XL hook in its `main.rs`, less AVIF (see above).
/// This process reads them with the same decoders, so `is_native` checks what wmde-bg will
/// meet. The lists move together.
const NATIVE: [&str; 5] = ["JPEG", "PNG", "WebP", "HDR", "JPEG XL"];

/// wmde-bg's `max_alloc` for a decoded picture, in `decode` of its `src/wallpaper.rs`.
const DAEMON_MAX_ALLOC: u64 = 1024 * 1024 * 1024;

/// The longest side of a written wallpaper. Above a 4K screen there is nothing left to see.
const WRITTEN_MAX: u32 = 3840;

const SETTINGS: &str = "fun.wmde.Settings.Wallpaper";
const CUSTOM_IMAGES: &str = "custom-images";

#[derive(Debug, Error)]
pub enum WallpaperError {
    #[error("{0}")]
    Load(#[from] LoadError),
    #[error("{0}")]
    Save(#[from] SaveError),
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Config(#[from] cosmic_config::Error),
    #[error("no data directory")]
    NoDataDir,
}

/// Whether wmde-bg shows the file at `path` as it is: a format it reads, a header its decoder
/// takes (an arithmetic-coded JPEG it does not), and a picture within its allocation limit.
#[must_use]
pub fn is_native(path: &Path) -> bool {
    if !content_format(path).is_some_and(|format| NATIVE.contains(&format)) {
        return false;
    }
    register_jpeg_xl();
    let Ok(mut reader) = ImageReader::open(path).and_then(ImageReader::with_guessed_format) else {
        return false;
    };
    reader.no_limits();
    reader
        .into_decoder()
        .is_ok_and(|decoder| decoder.total_bytes() <= DAEMON_MAX_ALLOC)
}

/// The number of the latest wallpaper request, and the lock one request holds while it runs.
static LATEST: AtomicU64 = AtomicU64::new(0);
static RUNNING: Mutex<()> = Mutex::new(());

/// Take a number for a wallpaper request, to hand to [`set_wallpaper_in_turn`].
pub fn next_request() -> u64 {
    LATEST.fetch_add(1, Ordering::SeqCst) + 1
}

/// Run [`set_wallpaper`] for request `request` once no other runs.
///
/// A request superseded by a later one meanwhile is not run: the last picture asked for is the
/// one that ends up on the desktop, even when an earlier one took longer to write. Returns
/// `None` then.
///
/// # Errors
///
/// As [`set_wallpaper`].
pub fn set_wallpaper_in_turn(
    request: u64,
    path: &Path,
    output: Option<&str>,
) -> Result<Option<PathBuf>, WallpaperError> {
    let _running = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
    if LATEST.load(Ordering::SeqCst) != request {
        return Ok(None);
    }
    set_wallpaper(path, output).map(Some)
}

/// Make the picture at `path` the background of `output`, or of every display when `None`.
///
/// A file wmde-bg cannot show is written as a PNG first. The file the background shows is
/// also offered in the settings' wallpaper list, as upstream did. Returns that file.
///
/// Blocks on the decoder and the disk; call it off the drawing thread.
///
/// # Errors
///
/// Returns [`WallpaperError`] if the picture cannot be read or written, or if the background
/// settings cannot be written.
pub fn set_wallpaper(path: &Path, output: Option<&str>) -> Result<PathBuf, WallpaperError> {
    // wmde-bg is another process with another working directory.
    let path = fs::canonicalize(path)?;
    let dir = wallpaper_dir().ok_or(WallpaperError::NoDataDir)?;
    // The settings store the path as text: one that is not UTF-8 goes through a PNG of ours.
    let native = path.to_str().is_some() && is_native(&path);
    let source = if native {
        path
    } else {
        write_png(&path, &dir)?
    };

    let context = wmde_bg_config::context()?;
    if !apply(&context, &source, output)? {
        // Nothing changed, and wmde-bg rereads a file only on a change: the same file set again
        // after an edit kept the old picture. The switch written again makes it reread.
        context
            .0
            .set(wmde_bg_config::SAME_ON_ALL, context.same_on_all())?;
    }
    forget_unused(&context, &dir);
    // A written PNG is not offered: it goes once no entry points at it.
    if native && let Err(err) = offer_in_settings(&source) {
        tracing::warn!(
            "cannot add {} to the settings' wallpapers: {err}",
            source.display()
        );
    }
    Ok(source)
}

/// Where written wallpapers live: our own data directory, not the picture's folder, and not
/// the cache, which may be cleared while the background still points here.
fn wallpaper_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|base| base.join("wmde").join("viewer").join("wallpaper"))
}

/// Write the picture as a PNG wmde-bg can read. The name carries the source's path, size and
/// time, so the same picture set twice is written once, and another picture never takes the
/// name of one on screen: an entry left unchanged would keep the old picture shown.
fn write_png(path: &Path, dir: &Path) -> Result<PathBuf, WallpaperError> {
    let meta = fs::metadata(path)?;
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    let stem = path
        .file_stem()
        .map_or_else(|| "wallpaper".into(), |stem| stem.to_string_lossy());
    // A file name holds 255 bytes; the hash and the extension take 21.
    let mut end = stem.len().min(200);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    let target = dir.join(format!("{}-{:016x}.png", &stem[..end], hasher.finish()));
    if target.is_file() {
        return Ok(target);
    }

    let image = load_for_wallpaper(path, WRITTEN_MAX)?;
    fs::create_dir_all(dir)?;
    save_image(&image, &target)?;
    Ok(target)
}

fn output_key(output: &str) -> String {
    if output == wmde_bg_config::DEFAULT_BACKGROUND {
        output.to_owned()
    } else {
        format!("output.{output}")
    }
}

/// Point the entries at `source`, each keeping the rest of what it was. Returns whether any
/// setting changed.
fn apply(context: &Context, source: &Path, output: Option<&str>) -> Result<bool, WallpaperError> {
    let mut config = Config::load(context)?;
    // `set_entry` rewrites the list of outputs from the ones it knows; it knows them all only
    // once they are loaded.
    config.load_backgrounds(context);
    let listed = context.backgrounds();
    let same_on_all = context.same_on_all();
    let all = context.default_background();
    let own = |output: &str| {
        context
            .entry(&output_key(output))
            .unwrap_or_else(|_| all.clone())
    };
    let with_source = |entry: Entry, output: &str| Entry {
        output: output.to_owned(),
        source: Source::Path(source.to_path_buf()),
        ..entry
    };

    let mut changed = false;
    match output {
        None => {
            let default = wmde_bg_config::DEFAULT_BACKGROUND;
            changed |= put(context, &mut config, with_source(all.clone(), default))?;
            for output in &listed {
                changed |= put(context, &mut config, with_source(own(output), output))?;
            }
        }
        Some(output) => {
            // A display shows its own entry only while the switch is off and it is listed;
            // otherwise it shows `all`, and the new picture keeps that fitting.
            let shown = if !same_on_all && listed.iter().any(|listed| listed == output) {
                own(output)
            } else {
                all.clone()
            };
            // The other displays showed `all` too; they keep showing it once the switch is off,
            // not whatever their own entries held from before.
            if same_on_all {
                for other in listed.iter().filter(|other| *other != output) {
                    let entry = Entry {
                        output: other.clone(),
                        ..all.clone()
                    };
                    changed |= put(context, &mut config, entry)?;
                }
            }
            changed |= put(context, &mut config, with_source(shown, output))?;
            if same_on_all {
                context.set_same_on_all(false)?;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Write one entry. Returns whether it differed from the stored one.
fn put(context: &Context, config: &mut Config, entry: Entry) -> Result<bool, WallpaperError> {
    let changed = context.entry(&output_key(&entry.output)).ok().as_ref() != Some(&entry);
    config.set_entry(context, entry)?;
    Ok(changed)
}

/// Remove the written PNGs no entry points at any more. One on screen is never removed, and
/// neither is anything but a PNG: a `.part` file is a save in progress.
fn forget_unused(context: &Context, dir: &Path) {
    let keys = std::iter::once(wmde_bg_config::DEFAULT_BACKGROUND.to_owned()).chain(
        context
            .backgrounds()
            .into_iter()
            .map(|output| output_key(&output)),
    );
    let used: HashSet<PathBuf> = keys
        .filter_map(|key| match context.entry(&key) {
            Ok(Entry {
                source: Source::Path(path),
                ..
            }) => Some(path),
            _ => None,
        })
        .collect();
    let Ok(files) = fs::read_dir(dir) else {
        return;
    };
    for file in files.filter_map(Result::ok).map(|file| file.path()) {
        if file.extension().is_some_and(|ext| ext == "png")
            && !used.contains(&file)
            && let Err(err) = fs::remove_file(&file)
        {
            tracing::warn!(
                "cannot remove the unused wallpaper {}: {err}",
                file.display()
            );
        }
    }
}

/// Add the file to the pictures the settings offer as wallpapers.
fn offer_in_settings(source: &Path) -> Result<(), cosmic_config::Error> {
    let settings = cosmic_config::Config::new(SETTINGS, 1)?;
    let mut images: Vec<PathBuf> = settings.get(CUSTOM_IMAGES).unwrap_or_default();
    if !images.iter().any(|image| image == source) {
        images.push(source.to_path_buf());
        settings.set(CUSTOM_IMAGES, images)?;
    }
    Ok(())
}

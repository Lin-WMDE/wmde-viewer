// SPDX-License-Identifier: GPL-3.0-only

use crate::raw::RAW_TYPES;
use icu_collator::{
    Collator, CollatorBorrowed, CollatorPreferences, options::CollatorOptions,
    preferences::CollationNumericOrdering,
};
use icu_locale_core::Locale;
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tokio::task::spawn_blocking;
use viewer_config::{SortMode, SortOrder};

// WMDE: `jpe` and `svgz` are added, and the camera RAW extensions are `raw::RAW_TYPES`;
// `extensions` gives both lists.
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "gif", "webp", "bmp", "tiff", "tif", "ico", "avif", "ppm", "pgm",
    "pbm", "pnm", "qoi", "ff", "farbfeld", "hdr", "jxl", "svg", "svgz", "heif", "heic",
];

/// Every extension the album takes, lowercase: `EXTENSIONS` and the camera RAW ones.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    EXTENSIONS
        .iter()
        .copied()
        .chain(RAW_TYPES.iter().map(|(extension, _)| *extension))
}

#[derive(Debug, Clone, Default)]
pub struct NavState {
    dir: Option<PathBuf>,
    images: Vec<PathBuf>,
    cur_idx: Option<usize>,
}

impl NavState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current(&self) -> Option<&PathBuf> {
        self.cur_idx.and_then(|idx| self.images.get(idx))
    }

    #[must_use]
    pub const fn index(&self) -> Option<usize> {
        self.cur_idx
    }

    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.cur_idx.is_some()
    }

    #[must_use]
    pub const fn total(&self) -> usize {
        self.images.len()
    }

    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    #[must_use]
    pub fn images(&self) -> &[PathBuf] {
        &self.images
    }

    pub fn set_images(&mut self, dir: PathBuf, images: Vec<PathBuf>, select: Option<&Path>) {
        self.dir = Some(dir);
        self.images = images;
        self.cur_idx = select.and_then(|path| self.images.iter().position(|pos| pos == path));
        // WMDE: a file the directory scan leaves out (a name without a known extension, a
        // hidden file) is still the one asked for. It is shown on its own instead of being
        // swapped for the first picture of the folder, which Delete would then have trashed.
        if self.cur_idx.is_none()
            && let Some(path) = select
        {
            self.images = vec![path.to_path_buf()];
            self.cur_idx = Some(0);
        }
    }

    pub fn select(&mut self, idx: usize) -> Option<&PathBuf> {
        if idx < self.images.len() {
            self.cur_idx = Some(idx);
            self.current()
        } else {
            None
        }
    }

    pub const fn deselect(&mut self) {
        self.cur_idx = None;
    }

    /// WMDE: take the current picture out of the album, as the next directory scan would once
    /// its file is gone. The picture after it takes its place, the one before when it was the
    /// last; an album left empty has nothing selected. Returns the new selection.
    pub fn forget_current(&mut self) -> Option<&PathBuf> {
        let idx = self.cur_idx.filter(|&idx| idx < self.images.len())?;
        self.images.remove(idx);
        self.cur_idx = if idx < self.images.len() {
            Some(idx)
        } else {
            idx.checked_sub(1)
        };
        self.current()
    }

    pub fn go_next(&mut self) -> Option<&PathBuf> {
        if self.images.is_empty() {
            return None;
        }

        let next = (self.cur_idx.unwrap_or(0) + 1) % self.images.len();
        self.cur_idx = Some(next);
        self.current()
    }

    pub fn go_prev(&mut self) -> Option<&PathBuf> {
        if self.images.is_empty() {
            return None;
        }

        let prev = match self.cur_idx {
            Some(0) | None => self.images.len() - 1,
            Some(idx) => idx - 1,
        };
        self.cur_idx = Some(prev);
        self.current()
    }

    pub fn first(&mut self) -> Option<&PathBuf> {
        if self.images.is_empty() {
            return None;
        }

        self.cur_idx = Some(0);
        self.current()
    }

    pub fn last(&mut self) -> Option<&PathBuf> {
        if self.images.is_empty() {
            return None;
        }

        self.cur_idx = Some(self.images.len() - 1);
        self.current()
    }
}

#[must_use]
pub fn get_image_dir(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        path.parent().map(std::path::Path::to_path_buf)
    } else if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

pub async fn scan_dir(
    dir: &Path,
    include_hidden: bool,
    sort_mode: SortMode,
    sort_order: SortOrder,
) -> Vec<PathBuf> {
    let dir = dir.to_path_buf();
    spawn_blocking(move || scan_dir_sync(&dir, include_hidden, sort_mode, sort_order))
        .await
        .unwrap_or_default()
}

fn scan_dir_sync(
    dir: &Path,
    include_hidden: bool,
    sort_mode: SortMode,
    sort_order: SortOrder,
) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            if !include_hidden
                && let Some(name) = path.file_name().and_then(|name| name.to_str())
                && name.starts_with('.')
            {
                return false;
            }
            is_supported_image(path)
        })
        .collect();

    sort_images(&mut images, sort_mode, sort_order);
    images
}

/// WMDE: the album of the files given, as `scan_dir` orders a directory: those that are still
/// files, whatever their name, since each was asked for by name.
pub async fn sort_files(
    files: Vec<PathBuf>,
    sort_mode: SortMode,
    sort_order: SortOrder,
) -> Vec<PathBuf> {
    spawn_blocking(move || {
        let mut images: Vec<PathBuf> = files.into_iter().filter(|path| path.is_file()).collect();
        sort_images(&mut images, sort_mode, sort_order);
        images
    })
    .await
    .unwrap_or_default()
}

fn sort_images(images: &mut [PathBuf], sort_mode: SortMode, sort_order: SortOrder) {
    images.sort_by(|a, b| {
        let ordering = match sort_mode {
            SortMode::Name => name_cmp(a, b),
            SortMode::Date => {
                let a_time = fs::metadata(a)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let b_time = fs::metadata(b)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                a_time.cmp(&b_time)
            }
            SortMode::Size => {
                let a_size = fs::metadata(a).map_or(0, |meta| meta.len());
                let b_size = fs::metadata(b).map_or(0, |meta| meta.len());
                a_size.cmp(&b_size)
            }
        };

        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });
}

#[must_use]
pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| extensions().any(|known| known.eq_ignore_ascii_case(ext)))
}

/// WMDE: the collator the album sorts names with: wmde-files' own, for the interface language,
/// numbers by value. Unset, it is American English's, as wmde-files falls back to.
static SORTER: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();

/// Sort names for `language`, a BCP-47 tag: the interface language, as wmde-files does.
///
/// The next picture is then the one that stands next in the file manager's window; Ukrainian,
/// for one, puts Cyrillic before Latin. Takes effect only before the first album is sorted.
pub fn set_sort_language(language: &str) {
    let sorter = Locale::try_from_str(language)
        .ok()
        .and_then(collator)
        .unwrap_or_else(default_collator);
    if SORTER.set(sorter).is_err() {
        tracing::warn!("the album sort language is set already; {language} is ignored");
    }
}

fn sorter() -> &'static CollatorBorrowed<'static> {
    SORTER.get_or_init(default_collator)
}

fn collator(locale: Locale) -> Option<CollatorBorrowed<'static>> {
    let mut prefs = CollatorPreferences::from(locale);
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    Collator::try_new(prefs, CollatorOptions::default()).ok()
}

fn default_collator() -> CollatorBorrowed<'static> {
    Locale::try_from_str("en-US")
        .ok()
        .and_then(collator)
        .expect("compiled data carries an American English collator")
}

/// Compare two file names the way wmde-files orders them. It compares its display names, which
/// only add zero-width spaces the collator ignores, and reads names that are not UTF-8 lossily.
fn name_cmp(a: &Path, b: &Path) -> Ordering {
    let name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    sorter().compare(&name(a), &name(b))
}

// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the reason a toast or the empty canvas gives for a failure, in the interface
// language. The error's own text is English, from the libraries and from this code, and
// `trash::Error` displays as a debug dump; that text goes to the journal only.

use crate::fl;
use std::io;
use viewer_core::{LoadError, SaveError, WallpaperError};

pub fn io_reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => fl!("error-reason-not-found"),
        io::ErrorKind::PermissionDenied => fl!("error-reason-permission"),
        io::ErrorKind::ReadOnlyFilesystem => fl!("error-reason-read-only"),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => fl!("error-reason-no-space"),
        _ => fl!("error-reason-system"),
    }
}

pub fn load_reason(err: &LoadError) -> String {
    match err {
        // A file that ends early or holds nonsense is damaged, not a failing system.
        LoadError::Io(err) | LoadError::Decode(image::ImageError::IoError(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
            ) =>
        {
            fl!("error-reason-unreadable")
        }
        LoadError::Io(err) | LoadError::Decode(image::ImageError::IoError(err)) => io_reason(err),
        LoadError::Decode(_) | LoadError::UnsupportedFormat(_) => fl!("error-reason-unreadable"),
        LoadError::Cancelled => fl!("error-reason-system"),
    }
}

pub fn save_reason(err: &SaveError) -> String {
    match err {
        SaveError::Io(err) | SaveError::Encode(image::ImageError::IoError(err)) => io_reason(err),
        SaveError::Encode(_) => fl!("error-reason-unwritable"),
    }
}

pub fn wallpaper_reason(err: &WallpaperError) -> String {
    match err {
        WallpaperError::Load(err) => load_reason(err),
        WallpaperError::Save(err) => save_reason(err),
        WallpaperError::Io(err) => io_reason(err),
        WallpaperError::Config(_) | WallpaperError::NoDataDir => fl!("error-reason-system"),
    }
}

pub fn trash_reason(err: &trash::Error) -> String {
    match err {
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        trash::Error::FileSystem { source, .. } => io_reason(source),
        trash::Error::Os { code, .. } => io_reason(&io::Error::from_raw_os_error(*code)),
        trash::Error::RestoreCollision { .. } | trash::Error::RestoreTwins { .. } => {
            fl!("error-reason-name-taken")
        }
        _ => fl!("error-reason-system"),
    }
}

/// The reason as a sentence of its own: it is written to follow a colon.
pub fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

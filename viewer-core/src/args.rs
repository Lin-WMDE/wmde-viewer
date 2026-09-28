// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the command line, read the way wmde-photos read it and then some. Every path `%F` hands
// over is kept, not only the first; `file://` URLs, which the file manager and the portal pass,
// lose their percent encoding; a relative path is made absolute before the working directory
// can matter; and nothing given opens the pictures directory by its xdg-user-dirs name, which
// a Ukrainian profile gives as Зображення, not Pictures. Arguments are read as the system gives
// them: `std::env::args` panics on a name that is not UTF-8.

use std::{ffi::OsString, path::PathBuf};

/// The paths to open, in the order given; the pictures directory when no argument is. Arguments
/// that were given and name nothing open nothing, not an album nobody asked for.
#[must_use]
pub fn start_paths<I: IntoIterator<Item = OsString>>(args: I) -> Vec<PathBuf> {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.is_empty() {
        pictures_dir().into_iter().collect()
    } else {
        paths(args)
    }
}

/// The paths the arguments name, in order. An argument that names none is logged and left out.
#[must_use]
pub fn paths<I: IntoIterator<Item = OsString>>(args: I) -> Vec<PathBuf> {
    args.into_iter().filter_map(to_path).collect()
}

fn to_path(argument: OsString) -> Option<PathBuf> {
    if argument.is_empty() {
        tracing::warn!("ignoring an empty argument");
        return None;
    }
    if let Some(url) = argument.to_str().filter(|text| text.starts_with("file://")) {
        let path = url::Url::parse(url)
            .ok()
            .and_then(|url| url.to_file_path().ok());
        if path.is_none() {
            // A file URL on another host, or not one at all: there is no local file to open.
            tracing::warn!("cannot open {url}: not a local file");
        }
        return path;
    }
    let path = PathBuf::from(argument);
    std::path::absolute(&path)
        .inspect_err(|err| tracing::warn!("cannot open {}: {err}", path.display()))
        .ok()
}

/// The pictures directory, else the home directory, when either is there.
fn pictures_dir() -> Option<PathBuf> {
    dirs::picture_dir()
        .filter(|dir| dir.is_dir())
        .or_else(|| dirs::home_dir().filter(|dir| dir.is_dir()))
}

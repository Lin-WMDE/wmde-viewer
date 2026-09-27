// SPDX-License-Identifier: GPL-3.0-only

// WMDE: saving never truncates the file it replaces. `DynamicImage::save` creates the
// destination before it asks the encoder, so an encoder that refuses the pixel type (GIF
// and QOI on 16-bit, TIFF on grey with alpha) or a full disk left the original at 0 bytes.

use image::{DynamicImage, ImageFormat};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("{0}")]
    Encode(#[from] image::ImageError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Encode `image` in the format named by `path`'s extension and put it at `path`.
///
/// The picture is encoded in full before the disk is touched. It then goes to a temporary
/// file beside the destination, which is renamed over it only once complete, so a failure at
/// any step leaves the destination as it was. Where a rename would change what the file is -
/// a second hard link, an owner the process cannot reproduce, a directory that allows no new
/// file - the encoded bytes are written into the existing file instead.
///
/// A destination that already exists must be writable, as for any in-place write, and keeps
/// its permissions. A symbolic link is written through to its target, even one whose target
/// does not exist yet.
///
/// # Errors
///
/// Returns [`SaveError`] if the extension names no format the encoder knows, if the encoder
/// refuses the image, if an existing destination is not writable, or if the file cannot be
/// written.
pub fn save_image(image: &DynamicImage, path: &Path) -> Result<(), SaveError> {
    let format = ImageFormat::from_path(path)?;
    let target = resolve_target(path);
    let existing = fs::metadata(&target).ok();
    if existing.is_some() {
        // The rename below needs only the directory's permission and would replace a file
        // that refuses writes; refuse it here, as an in-place write does.
        OpenOptions::new().write(true).open(&target)?;
    }

    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, format)?;
    let encoded = encoded.into_inner();

    match &existing {
        Some(meta) if has_other_links(meta) => write_in_place(&target, &encoded)?,
        Some(meta) => match replace(&target, &encoded, Some(meta)) {
            Ok(()) => {}
            Err(Replace::Refused(_)) => write_in_place(&target, &encoded)?,
            Err(Replace::Failed(err)) => return Err(err.into()),
        },
        None => match replace(&target, &encoded, None) {
            Ok(()) => {}
            Err(Replace::Refused(err) | Replace::Failed(err)) => return Err(err.into()),
        },
    }
    Ok(())
}

/// Why a picture could not be renamed into place.
enum Replace {
    /// The directory or the owner does not allow it; the existing file may still take a write.
    Refused(io::Error),
    Failed(io::Error),
}

impl From<io::Error> for Replace {
    fn from(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::PermissionDenied {
            Self::Refused(err)
        } else {
            Self::Failed(err)
        }
    }
}

fn replace(target: &Path, encoded: &[u8], existing: Option<&Metadata>) -> Result<(), Replace> {
    let dir = target
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    // A hidden name without an image extension: the directory scan skips it either way.
    let mut builder = tempfile::Builder::new();
    builder.prefix(".wmde-viewer-").suffix(".part");
    // A new file gets the usual mode for a document. A replacement starts at tempfile's
    // private 0600 and takes the old file's owner and mode before the picture goes in, so a
    // private picture is never readable through the temporary file.
    #[cfg(unix)]
    if existing.is_none() {
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    }
    let mut tmp = builder.tempfile_in(dir)?;

    if let Some(meta) = existing {
        // Any refusal here - no right to the owner, a file system without chown or chmod -
        // leaves the old file to be written in place, which keeps both.
        keep_owner(tmp.as_file(), meta).map_err(Replace::Refused)?;
        tmp.as_file()
            .set_permissions(meta.permissions())
            .map_err(Replace::Refused)?;
    }
    tmp.write_all(encoded).map_err(Replace::Failed)?;
    tmp.as_file().sync_all().map_err(Replace::Failed)?;
    tmp.persist(target)
        .map_err(|err| Replace::from(err.error))?;
    Ok(())
}

/// Truncate the existing file and write the encoded picture into it. Only reached with the
/// picture fully encoded, so an encoder refusal cannot leave the file empty.
fn write_in_place(target: &Path, encoded: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).truncate(true).open(target)?;
    file.write_all(encoded)?;
    file.sync_all()
}

#[cfg(unix)]
fn has_other_links(meta: &Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(meta) > 1
}

#[cfg(not(unix))]
const fn has_other_links(_meta: &Metadata) -> bool {
    false
}

/// Give the new file the old one's owner and group. Refused when the process may not.
#[cfg(unix)]
fn keep_owner(file: &File, meta: &Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    std::os::unix::fs::fchown(file, Some(meta.uid()), Some(meta.gid()))
}

#[cfg(not(unix))]
const fn keep_owner(_file: &File, _meta: &Metadata) -> io::Result<()> {
    Ok(())
}

/// The file a save lands in. A symbolic link is followed to its target, a dangling one too,
/// so the link itself stays a link.
fn resolve_target(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let mut current = path.to_path_buf();
    // The kernel's own limit on a chain of links.
    for _ in 0..40 {
        let Ok(next) = fs::read_link(&current) else {
            break;
        };
        // `join` keeps an absolute link target as it is.
        current = current
            .parent()
            .map_or_else(|| next.clone(), |dir| dir.join(&next));
    }
    current
}

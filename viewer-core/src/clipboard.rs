// SPDX-License-Identifier: GPL-3.0-only

use std::{borrow::Cow, path::Path};

use crate::raw::raw_mime_type;
use cosmic::iced::clipboard::mime::AsMimeTypes;
use url::Url;

// WMDE: a copy is the file itself, three ways. Its bytes under its own type are what an image
// editor pastes; `text/uri-list` is what a file manager pastes, as a copy of the file; the path
// as text is what goes into a text field, when it is UTF-8. The bytes are never re-encoded, and a type without an
// entry in `image_mime_type` still copies as the file and its path.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    data: Vec<u8>,
    mime: Option<&'static str>,
    uri_list: String,
    // Empty for a path that is not UTF-8: a lossy one would name a file that does not exist.
    text: String,
}

impl ClipboardImage {
    /// Reads `path` and works out what can be offered for it.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        let mime = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(image_mime_type);
        Ok(Self::new(path, data, mime))
    }

    fn new(path: &Path, data: Vec<u8>, mime: Option<&'static str>) -> Self {
        // RFC 2483: one URI per line, CRLF after each.
        let uri_list = Url::from_file_path(path)
            .map(|url| format!("{url}\r\n"))
            .unwrap_or_default();
        Self {
            data,
            mime,
            uri_list,
            text: path.to_str().unwrap_or_default().to_string(),
        }
    }
}

impl AsMimeTypes for ClipboardImage {
    fn available(&self) -> Cow<'static, [String]> {
        let mut types: Vec<String> = self.mime.iter().map(ToString::to_string).collect();
        if !self.uri_list.is_empty() {
            types.push(URI_LIST.to_string());
        }
        if !self.text.is_empty() {
            types.extend(TEXT.iter().map(ToString::to_string));
        }
        Cow::Owned(types)
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        if Some(mime_type) == self.mime {
            Some(Cow::Owned(self.data.clone()))
        } else if mime_type == URI_LIST && !self.uri_list.is_empty() {
            Some(Cow::Owned(self.uri_list.clone().into_bytes()))
        } else if TEXT.contains(&mime_type) && !self.text.is_empty() {
            Some(Cow::Owned(self.text.clone().into_bytes()))
        } else {
            None
        }
    }
}

const URI_LIST: &str = "text/uri-list";
const TEXT: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

#[must_use]
pub fn image_mime_type(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" | "jpe" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "tiff" | "tif" => Some("image/tiff"),
        "avif" => Some("image/avif"),
        // WMDE: shared-mime-info's names. Upstream's `image/x-ico` is an alias no paste target
        // asks for.
        "ico" => Some("image/vnd.microsoft.icon"),
        "svg" => Some("image/svg+xml"),
        // WMDE: the types the fork opens on top of upstream's, and the ones the album opened
        // all along that copied nothing.
        "svgz" => Some("image/svg+xml-compressed"),
        "jxl" => Some("image/jxl"),
        "heif" | "heic" => Some("image/heif"),
        "hdr" => Some("image/vnd.radiance"),
        "ppm" => Some("image/x-portable-pixmap"),
        "pgm" => Some("image/x-portable-graymap"),
        "pbm" => Some("image/x-portable-bitmap"),
        "pnm" => Some("image/x-portable-anymap"),
        "qoi" => Some("image/qoi"),
        "ff" | "farbfeld" => Some("image/x-farbfeld"),
        other => raw_mime_type(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(copy: &ClipboardImage) -> Vec<String> {
        copy.available().into_owned()
    }

    #[test]
    fn a_picture_goes_as_its_bytes_the_file_and_the_path() {
        let copy = ClipboardImage::new(
            Path::new("/home/a/Фото 1.png"),
            vec![1, 2, 3],
            Some("image/png"),
        );
        assert_eq!(
            offered(&copy),
            [
                "image/png",
                "text/uri-list",
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING"
            ]
        );
        assert_eq!(copy.as_bytes("image/png").as_deref(), Some(&[1, 2, 3][..]));
        assert_eq!(
            copy.as_bytes("text/uri-list").as_deref(),
            Some("file:///home/a/%D0%A4%D0%BE%D1%82%D0%BE%201.png\r\n".as_bytes())
        );
        assert_eq!(
            copy.as_bytes("text/plain").as_deref(),
            Some("/home/a/Фото 1.png".as_bytes())
        );
        assert_eq!(copy.as_bytes("image/jpeg"), None);
    }

    #[test]
    fn a_type_without_a_name_still_copies_the_file_and_the_path() {
        let copy = ClipboardImage::new(Path::new("/tmp/a.xyz"), vec![1], None);
        assert_eq!(
            offered(&copy),
            [
                "text/uri-list",
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING"
            ]
        );
    }

    #[test]
    fn a_path_that_is_not_utf8_goes_as_the_file_only() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let path = Path::new(OsStr::from_bytes(b"/tmp/f\xF6to.jpg"));
        let copy = ClipboardImage::new(path, vec![1], Some("image/jpeg"));
        assert_eq!(offered(&copy), ["image/jpeg", "text/uri-list"]);
        assert_eq!(
            copy.as_bytes("text/uri-list").as_deref(),
            Some(&b"file:///tmp/f%F6to.jpg\r\n"[..])
        );
        assert_eq!(copy.as_bytes("text/plain"), None);
    }

    #[test]
    fn every_extension_the_album_takes_copies_under_a_type() {
        for extension in crate::nav::extensions() {
            assert!(
                image_mime_type(extension).is_some(),
                "{extension} copies under no type"
            );
        }
        assert_eq!(image_mime_type("ICO"), Some("image/vnd.microsoft.icon"));
        assert_eq!(image_mime_type("heic"), Some("image/heif"));
    }
}

// SPDX-License-Identifier: GPL-3.0-only

use crate::raw;
use cosmic::widget::image::Handle;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image as FirImage};
use image::{DynamicImage, ImageBuffer, Pixel, RgbaImage, metadata::Orientation};
use libheif_rs::{ColorSpace as HeifColorSpace, HeifContext, LibHeif, RgbChroma};
use resvg::{tiny_skia, usvg};
use std::{
    fmt::{self, Debug, Formatter},
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use thiserror::Error;
use turbojpeg::{Colorspace as JpegColorspace, Decompressor, Image, PixelFormat, ScalingFactor};
use zune_image::codecs::bmp::zune_core::{bit_depth::BitDepth, colorspace::ColorSpace};
use zune_image::image::Image as ZuneImage;

// Cap texture uploads at 2048px on the long edge; the full-resolution image
// is kept separately so edits and saves operate on the real pixels, not the texture.
const MAX_TEX: u32 = 2048;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("Failed to read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to decode image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    // WMDE: a camera RAW file is shown through the preview inside it, and this one has none.
    #[error("no preview inside the RAW file")]
    NoPreview,
    #[error("Task cancelled")]
    Cancelled,
}

#[derive(Clone)]
pub struct LoadedImage {
    pub handle: Handle,
    pub image: DynamicImage,
    pub width: u32,
    pub height: u32,
    pub path: PathBuf,
}

impl Debug for LoadedImage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // `handle` and `image` hold opaque pixel buffers; omit them from Debug.
        f.debug_struct("LoadedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

// Display texture, downscaled to MAX_TEX. The source image is left full-res.
fn display_handle(image: &DynamicImage) -> Handle {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    if (width > MAX_TEX || height > MAX_TEX)
        && let Ok((tw, th, pixels)) = fast_resize_rgba(rgba.as_raw(), width, height, MAX_TEX)
    {
        return Handle::from_rgba(tw, th, pixels);
    }
    Handle::from_rgba(width, height, rgba.into_raw())
}

// WMDE: EXIF orientation is applied to the pixels here, at decode. The texture is built with
// `Handle::from_rgba`, and iced applies EXIF only to `Handle::Path` and `Handle::Bytes`, so a
// photo taken with the phone turned stayed on its side - in the view, in the thumbnail strip,
// and in what Save wrote back, since saving re-encodes these pixels and drops the tag. HEIF is
// left out: libheif applies the file's own transforms while decoding.

/// The EXIF orientation of the file at `path`, if it carries one.
fn exif_orientation(path: &Path) -> Option<Orientation> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let exif = exif::Reader::new().read_from_container(&mut file).ok()?;
    let value = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)?;
    Orientation::from_exif(u8::try_from(value).ok()?)
}

/// A full-resolution decode of `path`, turned the way the file asks to be viewed.
fn loaded_image(path: &Path, image: DynamicImage) -> LoadedImage {
    turned(path, image, exif_orientation(path))
}

/// A full-resolution decode of `path`, turned by `orientation`.
fn turned(path: &Path, mut image: DynamicImage, orientation: Option<Orientation>) -> LoadedImage {
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let (width, height) = (image.width(), image.height());
    LoadedImage {
        handle: display_handle(&image),
        image,
        width,
        height,
        path: path.to_path_buf(),
    }
}

/// A thumbnail of `path` from RGBA `pixels`, turned the way the file asks to be viewed.
fn loaded_thumbnail(path: &Path, width: u32, height: u32, pixels: Vec<u8>) -> LoadedImage {
    thumbnail_turned(path, (width, height, pixels), exif_orientation(path))
}

/// A thumbnail of `path` from RGBA pixels, turned by `orientation`.
fn thumbnail_turned(
    path: &Path,
    (width, height, pixels): (u32, u32, Vec<u8>),
    orientation: Option<Orientation>,
) -> LoadedImage {
    let rgba = RgbaImage::from_raw(width, height, pixels).expect("pixel buffer matches dimensions");
    let mut image = DynamicImage::ImageRgba8(rgba);
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let (width, height) = (image.width(), image.height());
    LoadedImage {
        handle: Handle::from_rgba(width, height, image.as_bytes().to_vec()),
        image,
        width,
        height,
        path: path.to_path_buf(),
    }
}

/// Decode the image at `path` on a background thread.
///
/// # Errors
///
/// Returns [`LoadError`] if the file cannot be read, the format is unsupported
/// or fails to decode, or the decode task is cancelled before completion.
pub async fn load_image(path: PathBuf) -> Result<LoadedImage, LoadError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    rayon::spawn(move || {
        let result = load_image_sync(&path);
        let _ = tx.send(result);
    });

    rx.await.map_err(|_| LoadError::Cancelled)?
}

fn load_image_sync(path: &Path) -> Result<LoadedImage, LoadError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    if is_drawing(path) {
        return load_svg_at(path, MAX_TEX);
    }
    let head = file_head(path)?;
    if is_heif(&head) {
        return load_heif(path);
    }
    if raw::reads_as_raw(path, &head) {
        return load_raw(path);
    }

    // Use turbojpeg for JPEGs (faster than zune/image crate)
    if matches!(extension.as_str(), "jpg" | "jpeg" | "jpe")
        && let Ok(img) = load_jpeg_full(path)
    {
        return Ok(img);
    }
    // Fall through to other decoders if turbojpeg fails

    if is_zune_supported(&extension) {
        match load_with_zune(path) {
            Ok(img) => return Ok(img),
            Err(_) => {
                return load_with_image(path);
            }
        }
    }

    // Standard image formats via the 'image' crate
    let img = open_by_content(path)?;
    Ok(loaded_image(path, img))
}

/// WMDE: the whole picture at `path` for a wallpaper file.
///
/// It is turned the way the file asks and is no longer than `max` on either side; an SVG is
/// drawn to that size rather than to the texture size. Blocks; call it off the drawing thread.
///
/// # Errors
///
/// Returns [`LoadError`] if the file cannot be read or decoded.
pub fn load_for_wallpaper(path: &Path, max: u32) -> Result<DynamicImage, LoadError> {
    let image = if is_drawing(path) {
        load_svg_at(path, max)?.image
    } else {
        load_image_sync(path)?.image
    };
    Ok(if image.width() > max || image.height() > max {
        image.resize(max, max, image::imageops::FilterType::CatmullRom)
    } else {
        image
    })
}

// WMDE: the content picks the decoder, the extension only breaks a tie. A file opened from the
// file manager under a name the viewer does not know, or under none, then opens instead of
// failing on its name.
fn open_by_content(path: &Path) -> Result<DynamicImage, LoadError> {
    register_jpeg_xl();
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    // TIFF is also the container of most camera RAW formats (NEF, DNG, ARW and others), whose
    // first image is a small preview; decoding it would pass the preview off as the photograph.
    // The RAW names claimed went to `load_raw` before this; what comes here is one that is not.
    let named_tiff = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_none_or(|ext| ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff"));
    if reader.format() == Some(image::ImageFormat::Tiff) && !named_tiff {
        return Err(LoadError::UnsupportedFormat(
            "TIFF content under another name, likely a camera RAW".into(),
        ));
    }
    Ok(reader.decode()?)
}

/// WMDE: whether `path` names a drawing, plain or gzip-compressed. SVG is text, and neither kind
/// is told by content; usvg unpacks the compressed one itself.
#[must_use]
pub fn is_drawing(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"))
}

/// WMDE: the start of the file, which tells HEIF and a RAW container apart from the rest.
fn file_head(path: &Path) -> Result<Vec<u8>, LoadError> {
    let mut head = Vec::with_capacity(64);
    File::open(path)?.take(64).read_to_end(&mut head)?;
    Ok(head)
}

// WMDE: HEIF and AVIF are told by their `ftyp` box, not by the name, so an AVIF served behind a
// `.jpg` link or a HEIC saved without an extension still reaches libheif, and a JPEG named
// `.heic` does not. AVIF goes to libheif at all because the `image` crate's `avif` feature is
// the ravif encoder and decodes nothing; libheif reads AVIF with its irot/imir transforms and
// grids.
fn is_heif(head: &[u8]) -> bool {
    ftyp_brands(head).any(|brand| HEIF_BRANDS.contains(&brand))
}

/// The `ftyp` brands libheif reads.
pub(crate) const HEIF_BRANDS: [&[u8]; 13] = [
    b"avif", b"avis", b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs",
    b"mif1", b"mif2", b"msf1",
];

/// The brands of the ISO BMFF `ftyp` box that opens `head`, the major brand first. None for
/// anything that does not open with one.
pub(crate) fn ftyp_brands(head: &[u8]) -> impl Iterator<Item = &[u8]> {
    let is_ftyp = head.len() >= 12 && &head[4..8] == b"ftyp";
    // Box size, then `ftyp`, the major brand, a minor version and the compatible brands.
    let size = head
        .first_chunk::<4>()
        .map_or(0, |size| u32::from_be_bytes(*size) as usize);
    let major = head.get(8..12).filter(|_| is_ftyp);
    let compatible = head
        .get(16..size.min(head.len()))
        .filter(|_| is_ftyp)
        .unwrap_or_default();
    major.into_iter().chain(
        compatible
            .as_chunks::<4>()
            .0
            .iter()
            .map(<[u8; 4]>::as_slice),
    )
}

/// Load full JPEG using turbojpeg (faster than zune/image crate)
// JPEG header dimensions are far below u32::MAX, so the usize -> u32 casts
// cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn load_jpeg_full(path: &Path) -> Result<LoadedImage, LoadError> {
    let jpeg_data = fs::read(path)?;
    Ok(loaded_image(path, decode_jpeg(&jpeg_data)?))
}

/// WMDE: a whole JPEG in memory through turbojpeg, split from `load_jpeg_full` for the preview
/// inside a RAW file.
// JPEG header dimensions are far below u32::MAX, so the usize -> u32 casts
// cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn decode_jpeg(jpeg_data: &[u8]) -> Result<DynamicImage, LoadError> {
    let mut decompressor = Decompressor::new()
        .map_err(|e| LoadError::UnsupportedFormat(format!("TurboJPEG init failed: {e}")))?;

    let header = decompressor
        .read_header(jpeg_data)
        .map_err(|e| LoadError::UnsupportedFormat(format!("JPEG header error: {e}")))?;

    let width = header.width;
    let height = header.height;

    // WMDE: grey stays grey and colour is RGB: JPEG holds no alpha, and a picture saved again
    // keeps the layout it was read in.
    let grey = header.colorspace == JpegColorspace::Gray;
    let (format, channels) = if grey {
        (PixelFormat::GRAY, 1)
    } else {
        (PixelFormat::RGB, 3)
    };
    let mut pixels = vec![0u8; channels * width * height];

    let mut output = Image {
        pixels: pixels.as_mut_slice(),
        width,
        pitch: channels * width,
        height,
        format,
    };

    decompressor
        .decompress(jpeg_data, output.as_deref_mut())
        .map_err(|e| LoadError::UnsupportedFormat(format!("JPEG decode error: {e}")))?;

    let (width, height) = (width as u32, height as u32);
    Ok(if grey {
        DynamicImage::ImageLuma8(buffer(width, height, pixels)?)
    } else {
        DynamicImage::ImageRgb8(buffer(width, height, pixels)?)
    })
}

/// WMDE: a camera RAW file, through the largest preview inside it that decodes. A 12-bit or a
/// damaged one gives way to the next; see `raw.rs`.
fn load_raw(path: &Path) -> Result<LoadedImage, LoadError> {
    let bytes = fs::read(path)?;
    let previews = raw::previews(&bytes);
    let image = previews
        .iter()
        .find_map(|preview| decode_preview(preview.data))
        .ok_or(LoadError::NoPreview)?;
    Ok(turned(path, image, raw::orientation(&bytes, &previews)))
}

/// A preview JPEG: turbojpeg, which stops at a warning, and the `image` crate's decoder, which
/// reads through one, as the JPEG path above falls back to it.
fn decode_preview(data: &[u8]) -> Option<DynamicImage> {
    decode_jpeg(data)
        .or_else(|_| image::load_from_memory_with_format(data, image::ImageFormat::Jpeg))
        .ok()
}

/// WMDE: the thumbnail of a camera RAW file: the smallest preview that still covers it, often
/// one the camera wrote for the purpose, else the largest.
fn raw_thumbnail(path: &Path, max_size: u32) -> Result<LoadedImage, LoadError> {
    let bytes = fs::read(path)?;
    let previews = raw::previews(&bytes);
    let covering = previews
        .iter()
        .rev()
        .filter(|preview| preview.width.max(preview.height) >= max_size);
    let pixels = covering
        .chain(&previews)
        .find_map(|preview| {
            let pixels = decode_jpeg_data_scaled(preview.data, max_size).or_else(|_| {
                let image = decode_preview(preview.data).ok_or(LoadError::NoPreview)?;
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                if width <= max_size && height <= max_size {
                    Ok((width, height, rgba.into_raw()))
                } else {
                    fast_resize_rgba(rgba.as_raw(), width, height, max_size)
                }
            });
            pixels.ok()
        })
        .ok_or(LoadError::NoPreview)?;
    Ok(thumbnail_turned(
        path,
        pixels,
        raw::orientation(&bytes, &previews),
    ))
}

/// An image buffer over `pixels`, which a decoder produced for `width` x `height`.
fn buffer<P: Pixel>(
    width: u32,
    height: u32,
    pixels: Vec<P::Subpixel>,
) -> Result<ImageBuffer<P, Vec<P::Subpixel>>, LoadError> {
    ImageBuffer::from_raw(width, height, pixels).ok_or_else(|| {
        LoadError::UnsupportedFormat("the decoded pixels do not match the size".into())
    })
}

// WMDE: PNM and JPEG XL go to `open_by_content` instead. zune-ppm returns the samples of a
// file whose maxval is not 255 or 65535 unscaled, so such a picture showed almost black; the
// `image` crate scales by maxval and reads every PNM subtype. zune's JPEG XL always renders to
// float, whatever depth the file has.
fn is_zune_supported(extension: &str) -> bool {
    matches!(
        extension,
        "jpg" | "jpeg" | "png" | "bmp" | "qoi" | "ff" | "farbfeld" | "hdr"
    )
}

/// WMDE: JPEG XL through jxl-oxide's decoder for the `image` crate, the one wmde-bg reads it
/// with: the picture comes in its own depth, and `open_by_content` finds it by its signature.
pub(crate) fn register_jpeg_xl() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        jxl_oxide::integration::register_image_decoding_hook();
    });
}

// Image dimensions originate from a decoded image header and are far below
// u32::MAX, so the usize -> u32 casts cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn load_with_zune(path: &Path) -> Result<LoadedImage, LoadError> {
    let img = ZuneImage::open(path).map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;
    Ok(loaded_image(path, from_zune(img)?))
}

// WMDE: the picture in its own depth and channel layout, as `open_by_content` keeps it. Every
// zune decode used to become 8-bit RGBA, so "Save" wrote a 16-bit PNG back at 8 bits and gave
// a grey or RGB PNG an alpha channel. Colour spaces the `image` crate has no type for become
// RGB, or RGBA when they carry alpha; grey in float has no type there either.
fn from_zune(mut img: ZuneImage) -> Result<DynamicImage, LoadError> {
    let colorspace = img.colorspace();
    let float = img.depth() == BitDepth::Float32;
    let keep = match colorspace {
        ColorSpace::RGB | ColorSpace::RGBA => true,
        ColorSpace::Luma | ColorSpace::LumaA => !float,
        _ => false,
    };
    if !keep {
        let to = if colorspace.has_alpha() {
            ColorSpace::RGBA
        } else {
            ColorSpace::RGB
        };
        img.convert_color(to)
            .map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;
    }
    if !matches!(
        img.depth(),
        BitDepth::Eight | BitDepth::Sixteen | BitDepth::Float32
    ) {
        img.convert_depth(BitDepth::Eight)
            .map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;
    }

    let (width, height) = img.dimensions();
    let (width, height) = (
        u32::try_from(width).map_err(|_| LoadError::UnsupportedFormat("too wide".into()))?,
        u32::try_from(height).map_err(|_| LoadError::UnsupportedFormat("too tall".into()))?,
    );
    let colorspace = img.colorspace();
    let no_pixels = || LoadError::UnsupportedFormat("No pixel data".into());
    Ok(match img.depth() {
        BitDepth::Sixteen => {
            let pixels = img
                .flatten_frames::<u16>()
                .into_iter()
                .next()
                .ok_or_else(no_pixels)?;
            match colorspace {
                ColorSpace::Luma => DynamicImage::ImageLuma16(buffer(width, height, pixels)?),
                ColorSpace::LumaA => DynamicImage::ImageLumaA16(buffer(width, height, pixels)?),
                ColorSpace::RGB => DynamicImage::ImageRgb16(buffer(width, height, pixels)?),
                _ => DynamicImage::ImageRgba16(buffer(width, height, pixels)?),
            }
        }
        BitDepth::Float32 => {
            let pixels = img
                .flatten_frames::<f32>()
                .into_iter()
                .next()
                .ok_or_else(no_pixels)?;
            if colorspace == ColorSpace::RGB {
                DynamicImage::ImageRgb32F(buffer(width, height, pixels)?)
            } else {
                DynamicImage::ImageRgba32F(buffer(width, height, pixels)?)
            }
        }
        _ => {
            let pixels = img
                .flatten_frames::<u8>()
                .into_iter()
                .next()
                .ok_or_else(no_pixels)?;
            match colorspace {
                ColorSpace::Luma => DynamicImage::ImageLuma8(buffer(width, height, pixels)?),
                ColorSpace::LumaA => DynamicImage::ImageLumaA8(buffer(width, height, pixels)?),
                ColorSpace::RGB => DynamicImage::ImageRgb8(buffer(width, height, pixels)?),
                _ => DynamicImage::ImageRgba8(buffer(width, height, pixels)?),
            }
        }
    })
}

fn load_with_image(path: &Path) -> Result<LoadedImage, LoadError> {
    let img = open_by_content(path)?;
    Ok(loaded_image(path, img))
}

/// Decode the image at `path` and downscale it to fit `max_size` on its longest
/// edge, on a background thread.
///
/// # Errors
///
/// Returns [`LoadError`] if the file cannot be read, the format is unsupported
/// or fails to decode, or the decode task is cancelled before completion.
pub async fn load_thumbnail(path: PathBuf, max_size: u32) -> Result<LoadedImage, LoadError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    rayon::spawn(move || {
        let result = load_thumbnail_sync(&path, max_size);
        let _ = tx.send(result);
    });

    rx.await.map_err(|_| LoadError::Cancelled)?
}

fn load_thumbnail_sync(path: &Path, max_size: u32) -> Result<LoadedImage, LoadError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    if is_drawing(path) {
        return load_svg_at(path, max_size);
    }
    let head = file_head(path)?;
    if is_heif(&head) {
        let loaded = load_heif(path)?;
        if loaded.width <= max_size && loaded.height <= max_size {
            return Ok(loaded);
        }
        let pixels = loaded.image.to_rgba8().into_raw();
        let (w, h, resized) = fast_resize_rgba(&pixels, loaded.width, loaded.height, max_size)?;
        let handle = Handle::from_rgba(w, h, resized.clone());
        let rgba_image =
            RgbaImage::from_raw(w, h, resized).expect("pixel buffer matches dimensions");
        return Ok(LoadedImage {
            handle,
            image: DynamicImage::ImageRgba8(rgba_image),
            width: w,
            height: h,
            path: path.to_path_buf(),
        });
    }

    if raw::reads_as_raw(path, &head) {
        return raw_thumbnail(path, max_size);
    }

    // 1. For JPEGs, try EXIF thumbnail extraction (no full decode)
    if matches!(extension.as_str(), "jpg" | "jpeg" | "jpe") {
        if let Ok((width, height, pixels)) = extract_exif_thumbnail(path, max_size) {
            return Ok(loaded_thumbnail(path, width, height, pixels));
        }

        // 2. For JPEGs without EXIF, use turbojpeg with DCT scaling (4-8x faster)
        if let Ok((width, height, pixels)) = decode_jpeg_scaled(path, max_size) {
            return Ok(loaded_thumbnail(path, width, height, pixels));
        }
    }

    // 3. Fall back to full decode + resize (non-JPEGs or if turbojpeg fails)
    let (width, height, pixels) = if is_zune_supported(&extension) {
        match decode_and_resize_zune(path, max_size) {
            Ok(result) => result,
            Err(_) => decode_and_resize_image(path, max_size)?,
        }
    } else {
        decode_and_resize_image(path, max_size)?
    };

    Ok(loaded_thumbnail(path, width, height, pixels))
}

/// Extract embedded EXIF thumbnail from JPEG files.
/// Reads only a small portion of the file rather than decoding it fully.
fn extract_exif_thumbnail(path: &Path, max_size: u32) -> Result<(u32, u32, Vec<u8>), LoadError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let exif = exif::Reader::new()
        .read_from_container(&mut reader)
        .map_err(|e| LoadError::UnsupportedFormat(format!("No EXIF data: {e}")))?;

    // Get the thumbnail data
    let thumbnail = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .zip(exif.get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL));

    if thumbnail.is_none() {
        return Err(LoadError::UnsupportedFormat("No EXIF thumbnail".into()));
    }

    // The exif crate doesn't directly expose thumbnail bytes; re-read the
    // file and extract the thumbnail using the offset/length.
    let thumb_bytes = extract_thumbnail_bytes(path, &exif)?;

    // Decode the embedded JPEG thumbnail
    let img = image::load_from_memory_with_format(&thumb_bytes, image::ImageFormat::Jpeg)
        .map_err(LoadError::Decode)?;

    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    // If thumbnail is already small enough, return it
    if width <= max_size && height <= max_size {
        return Ok((width, height, rgba.into_raw()));
    }

    // Resize if thumbnail is larger than requested
    let pixels = rgba.into_raw();
    fast_resize_rgba(&pixels, width, height, max_size)
}

/// Extract raw thumbnail bytes from JPEG using EXIF offset/length
fn extract_thumbnail_bytes(path: &Path, exif: &exif::Exif) -> Result<Vec<u8>, LoadError> {
    let offset = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .and_then(|f| f.value.get_uint(0))
        .ok_or_else(|| LoadError::UnsupportedFormat("No thumbnail offset".into()))?;

    let length = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)
        .and_then(|f| f.value.get_uint(0))
        .ok_or_else(|| LoadError::UnsupportedFormat("No thumbnail length".into()))?;

    // Sanity check - thumbnails shouldn't be huge
    if length > 1_000_000 {
        return Err(LoadError::UnsupportedFormat("Thumbnail too large".into()));
    }

    let mut file = File::open(path)?;

    // EXIF data starts after APP1 marker, typically at offset 12 from file start
    // The offset in EXIF is relative to the TIFF header, which is inside APP1;
    // resolve it to the actual file offset.

    // Read the APP1 segment to find the TIFF header offset
    let mut header = [0u8; 12];
    file.read_exact(&mut header)?;

    // JPEG starts with FFD8, then APP1 marker FFE1, then 2-byte length, then "Exif\0\0"
    // TIFF header starts after "Exif\0\0"
    let tiff_offset = if &header[0..2] == b"\xFF\xD8" && &header[2..4] == b"\xFF\xE1" {
        // APP1 starts at offset 2, length is at 4-5, "Exif\0\0" is at 6-11
        // TIFF header is at offset 12
        12u64
    } else {
        // Fallback: scan for APP1 marker
        file.seek(SeekFrom::Start(0))?;
        find_tiff_header_offset(&mut file)?
    };

    // Seek to thumbnail position (TIFF header offset + thumbnail offset in EXIF)
    file.seek(SeekFrom::Start(tiff_offset + u64::from(offset)))?;

    let mut thumb_data = vec![0u8; length as usize];
    file.read_exact(&mut thumb_data)?;

    Ok(thumb_data)
}

/// Scan JPEG file to find the TIFF header offset within APP1
fn find_tiff_header_offset(file: &mut File) -> Result<u64, LoadError> {
    file.seek(SeekFrom::Start(0))?;

    let mut marker = [0u8; 2];
    file.read_exact(&mut marker)?;

    if marker != [0xFF, 0xD8] {
        return Err(LoadError::UnsupportedFormat("Not a JPEG file".into()));
    }

    loop {
        file.read_exact(&mut marker)?;

        if marker[0] != 0xFF {
            return Err(LoadError::UnsupportedFormat("Invalid JPEG marker".into()));
        }

        // Skip padding FF bytes
        while marker[1] == 0xFF {
            file.read_exact(&mut marker[1..2])?;
        }

        // Read segment length
        let mut len_bytes = [0u8; 2];
        file.read_exact(&mut len_bytes)?;
        let segment_len = u64::from(u16::from_be_bytes(len_bytes));

        if marker[1] == 0xE1 {
            // APP1 segment - check for EXIF
            let mut exif_header = [0u8; 6];
            file.read_exact(&mut exif_header)?;

            if &exif_header[0..4] == b"Exif" {
                // TIFF header starts here
                return Ok(file.stream_position()?);
            }

            // Non-EXIF APP1 - already consumed 6 bytes after len_bytes,
            // so remaining = segment_len - 2 (len) - 6 (header read) = segment_len - 8
            let current_pos = file.stream_position()?;
            file.seek(SeekFrom::Start(current_pos + segment_len - 8))?;
            continue;
        }

        // Skip to next segment
        let current_pos = file.stream_position()?;
        file.seek(SeekFrom::Start(current_pos + segment_len - 2))?;

        // End of image
        if marker[1] == 0xD9 {
            break;
        }
    }

    Err(LoadError::UnsupportedFormat(
        "No EXIF APP1 segment found".into(),
    ))
}

/// Decode JPEG with DCT scaling using turbojpeg (4-8x faster than full decode)
/// This decodes directly to a smaller resolution, skipping most IDCT computation
// JPEG header/scaled dimensions are far below u32::MAX, so the usize -> u32
// casts cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn decode_jpeg_scaled(path: &Path, max_size: u32) -> Result<(u32, u32, Vec<u8>), LoadError> {
    decode_jpeg_data_scaled(&fs::read(path)?, max_size)
}

/// WMDE: the same for a JPEG in memory, split out for the preview inside a RAW file.
// JPEG header/scaled dimensions are far below u32::MAX, so the usize -> u32
// casts cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn decode_jpeg_data_scaled(
    jpeg_data: &[u8],
    max_size: u32,
) -> Result<(u32, u32, Vec<u8>), LoadError> {
    // Create decompressor
    let mut decompressor = Decompressor::new()
        .map_err(|e| LoadError::UnsupportedFormat(format!("TurboJPEG init failed: {e}")))?;

    // Read header to get original dimensions
    let header = decompressor
        .read_header(jpeg_data)
        .map_err(|e| LoadError::UnsupportedFormat(format!("JPEG header error: {e}")))?;

    let (orig_width, orig_height) = (header.width as u32, header.height as u32);

    // Calculate the best scaling factor
    let scaling = calculate_jpeg_scale(orig_width, orig_height, max_size);

    decompressor
        .set_scaling_factor(scaling)
        .map_err(|e| LoadError::UnsupportedFormat(format!("Scale factor error: {e}")))?;

    let scaled = header.scaled(scaling);
    let width = scaled.width;
    let height = scaled.height;

    // Pre-allocate output buffer for RGBA (4 bytes per pixel)
    let mut pixels = vec![0u8; 4 * width * height];

    let mut output = Image {
        pixels: pixels.as_mut_slice(),
        width,
        pitch: 4 * width,
        height,
        format: PixelFormat::RGBA,
    };

    // Decompress with scaling directly to RGBA
    decompressor
        .decompress(jpeg_data, output.as_deref_mut())
        .map_err(|e| LoadError::UnsupportedFormat(format!("JPEG decode error: {e}")))?;

    let width = width as u32;
    let height = height as u32;

    // If the scaled image is still larger than max_size, do a final resize
    if width > max_size || height > max_size {
        return fast_resize_rgba(&pixels, width, height, max_size);
    }

    Ok((width, height, pixels))
}

/// Calculate the best JPEG scaling factor to get close to target size
// max_dim is a JPEG dimension (<= u32::MAX) and the DCT ratios are 1/1..1/8, so
// the f32 round-trip is exact for realistic image sizes and the result is
// non-negative and in u32 range.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn calculate_jpeg_scale(width: u32, height: u32, target: u32) -> ScalingFactor {
    let max_dim = width.max(height);

    // Available scaling factors in turbojpeg (sorted largest to smallest)
    // These are the standard DCT scaling factors
    let ratios: [(usize, usize); 4] = [
        (1, 1), // 100%
        (1, 2), // 50%
        (1, 4), // 25%
        (1, 8), // 12.5%
    ];

    // Find the smallest scale that produces an image >= target size
    // (decode slightly larger, then resize down for quality)
    for &(num, denom) in ratios.iter().rev() {
        let scaled = (max_dim as f32 * num as f32 / denom as f32) as u32;
        if scaled >= target {
            return ScalingFactor::new(num, denom);
        }
    }

    // If even 1/8 is too large, use 1/8 and resize after
    ScalingFactor::ONE_EIGHTH
}

/// Decode and resize using zune, returns (width, height, `rgba_pixels`)
// Image dimensions originate from a decoded image header and are far below
// u32::MAX, so the usize -> u32 casts cannot truncate.
#[allow(clippy::cast_possible_truncation)]
fn decode_and_resize_zune(path: &Path, max_size: u32) -> Result<(u32, u32, Vec<u8>), LoadError> {
    let mut img = ZuneImage::open(path).map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;

    img.convert_color(ColorSpace::RGBA)
        .map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;

    let (width, height) = img.dimensions();

    let pixels = img
        .flatten_to_u8()
        .into_iter()
        .next()
        .ok_or_else(|| LoadError::UnsupportedFormat("No pixel data".into()))?;

    // If already small enough, return directly
    if width <= max_size as usize && height <= max_size as usize {
        return Ok((width as u32, height as u32, pixels));
    }

    fast_resize_rgba(&pixels, width as u32, height as u32, max_size)
}

/// Decode and resize using image crate, returns (width, height, `rgba_pixels`)
fn decode_and_resize_image(path: &Path, max_size: u32) -> Result<(u32, u32, Vec<u8>), LoadError> {
    let img = open_by_content(path)?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    // If already small enough, return directly
    if width <= max_size && height <= max_size {
        return Ok((width, height, rgba.into_raw()));
    }

    let pixels = rgba.into_raw();
    fast_resize_rgba(&pixels, width, height, max_size)
}

/// Fast RGBA image resize using SIMD-optimized `fast_image_resize` crate
// Source dimensions are positive image sizes and `ratio` is in (0, 1], so the
// scaled product is non-negative and within u32 range after rounding; the f32
// round-trip is exact for realistic image sizes.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn fast_resize_rgba(
    pixels: &[u8],
    src_width: u32,
    src_height: u32,
    max_size: u32,
) -> Result<(u32, u32, Vec<u8>), LoadError> {
    // Calculate target dimensions maintaining aspect ratio
    let (dst_width, dst_height) = if src_width > src_height {
        let ratio = max_size as f32 / src_width as f32;
        (max_size, (src_height as f32 * ratio).round() as u32)
    } else {
        let ratio = max_size as f32 / src_height as f32;
        ((src_width as f32 * ratio).round() as u32, max_size)
    };

    // Ensure dimensions are at least 1
    let dst_width = dst_width.max(1);
    let dst_height = dst_height.max(1);

    // Create source image from pixel data
    let src_image = FirImage::from_vec_u8(src_width, src_height, pixels.to_vec(), PixelType::U8x4)
        .map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;

    let mut dst_image = FirImage::new(dst_width, dst_height, PixelType::U8x4);

    // Resize with bilinear algorithm (good balance of speed and quality for thumbnails)
    let mut resizer = Resizer::new();
    resizer
        .resize(
            &src_image,
            &mut dst_image,
            Some(&ResizeOptions::new().resize_alg(ResizeAlg::Convolution(
                fast_image_resize::FilterType::Bilinear,
            ))),
        )
        .map_err(|e| LoadError::UnsupportedFormat(e.to_string()))?;

    Ok((dst_width, dst_height, dst_image.into_vec()))
}

/// Scale an SVG to fill `max` in its larger dimension. SVG is vector, so
/// rendering above natural size stays crisp - no pixel upscaling.
// reason: vector sizes and the rounded fit result are small, non-negative f32s.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn svg_fit_dimensions(tree: &usvg::Tree, max: u32) -> (u32, u32) {
    let size = tree.size();
    let nat_w = size.width();
    let nat_h = size.height();
    let scale = (max as f32 / nat_w).min(max as f32 / nat_h);
    let w = (nat_w * scale).round().max(1.0) as u32;
    let h = (nat_h * scale).round().max(1.0) as u32;
    (w, h)
}

/// Rasterize an SVG tree to straight-alpha RGBA at the given pixel size.
// reason: un-premultiply arithmetic stays within u8 range by construction.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn render_svg(tree: &usvg::Tree, w: u32, h: u32) -> Result<Vec<u8>, LoadError> {
    let mut pixmap = tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| LoadError::UnsupportedFormat("SVG: failed to create pixmap".into()))?;

    let sx = w as f32 / tree.size().width();
    let sy = h as f32 / tree.size().height();
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );

    // tiny-skia pixels are premultiplied; `Handle::from_rgba` expects straight alpha.
    let mut pixels = pixmap.take();
    for chunk in pixels.chunks_exact_mut(4) {
        let a = u32::from(chunk[3]);
        if a > 0 && a < 255 {
            chunk[0] = ((u32::from(chunk[0]) * 255) / a).min(255) as u8;
            chunk[1] = ((u32::from(chunk[1]) * 255) / a).min(255) as u8;
            chunk[2] = ((u32::from(chunk[2]) * 255) / a).min(255) as u8;
        }
    }

    Ok(pixels)
}

fn load_svg_at(path: &Path, max: u32) -> Result<LoadedImage, LoadError> {
    let data = fs::read(path)?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| LoadError::UnsupportedFormat(format!("SVG: {e}")))?;

    let (w, h) = svg_fit_dimensions(&tree, max);
    let pixels = render_svg(&tree, w, h)?;

    let handle = Handle::from_rgba(w, h, pixels.clone());
    let rgba_image = RgbaImage::from_raw(w, h, pixels).expect("pixel buffer matches dimensions");

    Ok(LoadedImage {
        handle,
        image: DynamicImage::ImageRgba8(rgba_image),
        width: w,
        height: h,
        path: path.to_path_buf(),
    })
}

fn load_heif(path: &Path) -> Result<LoadedImage, LoadError> {
    let name = path
        .to_str()
        .ok_or_else(|| LoadError::UnsupportedFormat("HEIF: non-UTF-8 path".into()))?;

    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_file(name)
        .map_err(|e| LoadError::UnsupportedFormat(format!("HEIF: {e}")))?;

    let handle = ctx
        .primary_image_handle()
        .map_err(|e| LoadError::UnsupportedFormat(format!("HEIF: {e}")))?;

    // WMDE: alpha only when the file has it, and more than 8 bits (10- and 12-bit AVIF and HEIC)
    // as 16. It was always 8-bit RGBA, so a deep picture lost its depth before any edit.
    let alpha = handle.has_alpha_channel();
    let deep = handle.luma_bits_per_pixel() > 8;
    let chroma = match (deep, alpha) {
        (false, false) => RgbChroma::Rgb,
        (false, true) => RgbChroma::Rgba,
        (true, false) => RgbChroma::HdrRgbLe,
        (true, true) => RgbChroma::HdrRgbaLe,
    };
    let img = lib
        .decode(&handle, HeifColorSpace::Rgb(chroma), None)
        .map_err(|e| LoadError::UnsupportedFormat(format!("HEIF: {e}")))?;

    let plane = img
        .planes()
        .interleaved
        .ok_or_else(|| LoadError::UnsupportedFormat("HEIF: no interleaved plane".into()))?;

    // Use the DECODED image's dimensions, not the handle's: libheif applies
    // rotation/mirror (irot/imir) during decode, so a phone photo's handle reports
    // the pre-transform (ispe) size while the plane data is the post-transform size.
    // Copying with the handle's size then reads the wrong bytes (or out of bounds).
    let width = img.width();
    let height = img.height();
    if width == 0 || height == 0 {
        return Err(LoadError::UnsupportedFormat(
            "HEIF: invalid dimensions".into(),
        ));
    }

    let channels = if alpha { 4 } else { 3 };
    let sample_bytes = if deep { 2 } else { 1 };
    let stride = plane.stride;
    let row_bytes = width as usize * channels * sample_bytes;
    if stride < row_bytes || plane.data.len() < stride * height as usize {
        return Err(LoadError::UnsupportedFormat(
            "HEIF: truncated pixel data".into(),
        ));
    }
    // Copy row-by-row: the plane stride may exceed the packed row width.
    let mut bytes = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height as usize {
        let row_start = y * stride;
        bytes.extend_from_slice(&plane.data[row_start..row_start + row_bytes]);
    }

    let image = if deep {
        // Samples of `bits` bits, stretched over the full 16-bit range.
        let bits = u32::from(plane.bits_per_pixel.clamp(9, 16));
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| {
                let value = u16::from_le_bytes(*sample);
                if bits == 16 {
                    value
                } else {
                    (value << (16 - bits)) | (value >> (2 * bits - 16))
                }
            })
            .collect();
        if alpha {
            DynamicImage::ImageRgba16(buffer(width, height, samples)?)
        } else {
            DynamicImage::ImageRgb16(buffer(width, height, samples)?)
        }
    } else if alpha {
        DynamicImage::ImageRgba8(buffer(width, height, bytes)?)
    } else {
        DynamicImage::ImageRgb8(buffer(width, height, bytes)?)
    };
    // The texture goes through `display_handle` like every other picture's, capped at MAX_TEX.
    // libheif has turned the picture already; the file carries no EXIF orientation to apply.
    let (width, height) = (image.width(), image.height());
    Ok(LoadedImage {
        handle: display_handle(&image),
        image,
        width,
        height,
        path: path.to_path_buf(),
    })
}

/// Read DPI from EXIF data (JPEG/TIFF only)
#[must_use]
pub fn read_dpi(path: &Path) -> Option<u32> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let exif = exif::Reader::new().read_from_container(&mut file).ok()?;
    exif_dpi(&exif)
}

// WMDE: split from `read_dpi` for the details drawer, which has the EXIF read already.
// The DPI value is rounded and clamped into [0, u32::MAX] before the cast, so
// it cannot truncate or lose a sign.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn exif_dpi(exif: &exif::Exif) -> Option<u32> {
    let x_res = exif.get_field(exif::Tag::XResolution, exif::In::PRIMARY)?;

    match x_res.value {
        // A malformed EXIF rational could be negative or absurdly large; round
        // and clamp into u32 range rather than letting `as` wrap silently.
        exif::Value::Rational(ref v) if !v.is_empty() => {
            Some(v[0].to_f64().round().clamp(0.0, f64::from(u32::MAX)) as u32)
        }
        _ => None,
    }
}

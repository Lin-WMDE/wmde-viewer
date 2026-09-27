// SPDX-License-Identifier: GPL-3.0-only

// WMDE: what a picture's file says about itself, for the details drawer - the rows wmde-photos
// showed, read the way it read them. Values come back as data: the drawer words them in the
// interface language, since the EXIF crate's own formatting is English ("1/250 s", "50 deg
// 27 min", "fired, return light not detected").

use crate::loader::{HEIF_BRANDS, exif_dpi, ftyp_brands};
use exif::{In, Tag, Value};
use image::ImageFormat;
use resvg::usvg;
use std::{
    fs::{self, File},
    io::{BufReader, Read},
    path::Path,
    time::SystemTime,
};

/// Everything the details drawer shows that has to be read from the file.
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// The format of the content, whatever the name says.
    pub format: Option<&'static str>,
    pub dpi: Option<u32>,
    /// The size an SVG drawing declares. The cached picture of an SVG is its rendering at the
    /// texture size and says nothing about the drawing.
    pub drawing_size: Option<(u32, u32)>,
    /// `None` when the file carries no EXIF at all.
    pub exif: Option<Photograph>,
}

/// The photograph's own account of when, with what and where it was taken. A field the file
/// leaves out, or fills with nothing, is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Photograph {
    pub taken: Option<Taken>,
    /// Maker and model as one line.
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub exposure: Option<Exposure>,
    /// The f-number.
    pub aperture: Option<f64>,
    pub iso: Option<u32>,
    /// In millimetres.
    pub focal_length: Option<f64>,
    pub flash: Option<Flash>,
    /// In degrees, north positive.
    pub latitude: Option<f64>,
    /// In degrees, east positive.
    pub longitude: Option<f64>,
    pub software: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub copyright: Option<String>,
}

/// When the shutter fired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Taken {
    /// The camera's clock, in the EXIF layout. Without a time zone: few cameras record one.
    Clock {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
    /// A value some other program wrote in another layout, shown as written.
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exposure {
    /// 1/n of a second, n with at most one decimal: 1/250, 1/2.5.
    Fraction(f64),
    Seconds(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flash {
    /// `None` when the camera has no flash.
    pub fired: Option<bool>,
    pub mode: Option<FlashMode>,
    pub red_eye_reduction: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashMode {
    Forced,
    Suppressed,
    Auto,
}

/// Read what the file at `path` says about itself. Blocks on the file; call it off the
/// drawing thread.
#[must_use]
pub fn read_details(path: &Path) -> Details {
    let meta = fs::metadata(path).ok();
    let exif = File::open(path).ok().and_then(|file| {
        exif::Reader::new()
            .read_from_container(&mut BufReader::new(file))
            .ok()
    });
    let format = content_format(path);
    Details {
        size: meta.as_ref().map(fs::Metadata::len),
        created: meta.as_ref().and_then(|meta| meta.created().ok()),
        modified: meta.as_ref().and_then(|meta| meta.modified().ok()),
        accessed: meta.as_ref().and_then(|meta| meta.accessed().ok()),
        format,
        dpi: exif.as_ref().and_then(exif_dpi),
        drawing_size: (format == Some("SVG"))
            .then(|| drawing_size(path))
            .flatten(),
        exif: exif.as_ref().map(photograph),
    }
}

// Drawing sizes are positive and far below u32::MAX once rounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn drawing_size(path: &Path) -> Option<(u32, u32)> {
    let data = fs::read(path).ok()?;
    let size = usvg::Tree::from_data(&data, &usvg::Options::default())
        .ok()?
        .size();
    Some((
        (size.width().round() as u32).max(1),
        (size.height().round() as u32).max(1),
    ))
}

fn photograph(exif: &exif::Exif) -> Photograph {
    let text = |tag| text(exif, tag);
    Photograph {
        taken: taken(exif),
        camera: camera(text(Tag::Make), text(Tag::Model)),
        lens: text(Tag::LensModel),
        exposure: rational(exif, Tag::ExposureTime).and_then(exposure),
        aperture: rational(exif, Tag::FNumber).and_then(positive),
        iso: uint(exif, Tag::PhotographicSensitivity).filter(|iso| *iso > 0),
        focal_length: rational(exif, Tag::FocalLength).and_then(positive),
        flash: uint(exif, Tag::Flash).map(flash),
        latitude: coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, b'S'),
        longitude: coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, b'W'),
        software: text(Tag::Software),
        author: text(Tag::Artist),
        description: text(Tag::ImageDescription),
        copyright: text(Tag::Copyright),
    }
}

/// A text field, its parts joined; `None` when it holds only blanks.
fn text(exif: &exif::Exif, tag: Tag) -> Option<String> {
    let Value::Ascii(parts) = &exif.get_field(tag, In::PRIMARY)?.value else {
        return None;
    };
    let text = parts
        .iter()
        .map(|part| String::from_utf8_lossy(part).trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    (!text.is_empty()).then_some(text)
}

fn rational(exif: &exif::Exif, tag: Tag) -> Option<exif::Rational> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(values) => values.first().copied(),
        _ => None,
    }
}

fn uint(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

fn positive(value: exif::Rational) -> Option<f64> {
    Some(value.to_f64()).filter(|value| value.is_finite() && *value > 0.0)
}

fn taken(exif: &exif::Exif) -> Option<Taken> {
    let Value::Ascii(parts) = &exif.get_field(Tag::DateTimeOriginal, In::PRIMARY)?.value else {
        return None;
    };
    match exif::DateTime::from_ascii(parts.first()?) {
        // All zeros is how some cameras say the clock was never set.
        Ok(time) if time.year == 0 && time.month == 0 && time.day == 0 => None,
        Ok(time) => Some(Taken::Clock {
            year: time.year,
            month: time.month,
            day: time.day,
            hour: time.hour,
            minute: time.minute,
            second: time.second,
        }),
        Err(exif::Error::BlankValue(_)) => None,
        Err(_) => text(exif, Tag::DateTimeOriginal).map(Taken::Text),
    }
}

/// Maker and model as one line. Most makers repeat their name in the model, and "NIKON
/// CORPORATION NIKON D750" helps nobody.
fn camera(make: Option<String>, model: Option<String>) -> Option<String> {
    match (make, model) {
        (Some(make), Some(model)) => {
            let maker = make.split_whitespace().next().unwrap_or_default();
            if model.to_lowercase().starts_with(&maker.to_lowercase()) {
                Some(model)
            } else {
                Some(format!("{make} {model}"))
            }
        }
        (make, model) => make.or(model),
    }
}

/// Times under a second read as the fraction a camera shows, 1/250 or 1/2.5 rather than 0.004
/// or 0.4, when the time is one: within 3 % of a whole 1/n (cameras store APEX-rounded values
/// such as 667/10000 for 1/15) or within 0.5 % of a 1/n with one decimal. Anything else, 0.6 s
/// for one, stays in seconds.
fn exposure(time: exif::Rational) -> Option<Exposure> {
    if time.num == 0 || time.denom == 0 {
        return None;
    }
    let seconds = time.to_f64();
    if seconds < 1.0 {
        let per_second = 1.0 / seconds;
        let whole = per_second.round();
        if whole >= 2.0 && (per_second - whole).abs() <= per_second * 0.03 {
            return Some(Exposure::Fraction(whole));
        }
        let tenths = (per_second * 10.0).round();
        if (per_second * 10.0 - tenths).abs() <= per_second * 10.0 * 0.005 {
            return Some(Exposure::Fraction(tenths / 10.0));
        }
    }
    Some(Exposure::Seconds(seconds))
}

const fn flash(value: u32) -> Flash {
    // Bit 0: the flash fired. Bits 3-4: the mode. Bit 5: there is no flash. Bit 6: red-eye
    // reduction. The return-light bits 1-2 are sensor detail and are left out.
    if value & 0x20 != 0 && value & 1 == 0 {
        return Flash {
            fired: None,
            mode: None,
            red_eye_reduction: false,
        };
    }
    Flash {
        fired: Some(value & 1 != 0),
        mode: match (value >> 3) & 3 {
            1 => Some(FlashMode::Forced),
            2 => Some(FlashMode::Suppressed),
            3 => Some(FlashMode::Auto),
            _ => None,
        },
        red_eye_reduction: value & 0x40 != 0,
    }
}

/// Degrees from degrees, minutes and seconds, negative in the hemisphere named by `negative`.
/// A coordinate without its hemisphere is left out: it could be either.
fn coordinate(exif: &exif::Exif, tag: Tag, reference: Tag, negative: u8) -> Option<f64> {
    let Value::Rational(parts) = &exif.get_field(tag, In::PRIMARY)?.value else {
        return None;
    };
    let [degrees, minutes, seconds] = parts.get(..3)? else {
        return None;
    };
    let Value::Ascii(hemisphere) = &exif.get_field(reference, In::PRIMARY)?.value else {
        return None;
    };
    let hemisphere = *hemisphere.first()?.first()?;
    let value = degrees.to_f64() + minutes.to_f64() / 60.0 + seconds.to_f64() / 3600.0;
    if !value.is_finite() {
        return None;
    }
    Some(if hemisphere.eq_ignore_ascii_case(&negative) {
        -value
    } else {
        value
    })
}

/// The format of the content, told the way the loader tells it: HEIF and AVIF by their `ftyp`
/// box, JPEG XL by its signature, the rest by the `image` crate. SVG is text and goes by its
/// name.
fn content_format(path: &Path) -> Option<&'static str> {
    let mut head = Vec::with_capacity(64);
    File::open(path)
        .and_then(|file| file.take(64).read_to_end(&mut head))
        .ok()?;
    let brands: Vec<&[u8]> = ftyp_brands(&head).collect();
    if brands
        .iter()
        .any(|brand| matches!(*brand, b"avif" | b"avis"))
    {
        return Some("AVIF");
    }
    if brands.iter().any(|brand| HEIF_BRANDS.contains(brand)) {
        return Some("HEIF");
    }
    if head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n") {
        return Some("JPEG XL");
    }
    if let Ok(format) = image::guess_format(&head) {
        return format_name(format);
    }
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        .then_some("SVG")
}

const fn format_name(format: ImageFormat) -> Option<&'static str> {
    Some(match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        ImageFormat::Pnm => "PNM",
        ImageFormat::Tiff => "TIFF",
        ImageFormat::Tga => "TGA",
        ImageFormat::Dds => "DDS",
        ImageFormat::Bmp => "BMP",
        ImageFormat::Ico => "ICO",
        ImageFormat::Hdr => "HDR",
        ImageFormat::OpenExr => "OpenEXR",
        ImageFormat::Farbfeld => "Farbfeld",
        ImageFormat::Avif => "AVIF",
        ImageFormat::Qoi => "QOI",
        _ => return None,
    })
}

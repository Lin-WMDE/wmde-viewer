// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the photograph's EXIF worded for the details drawer. Units and hemispheres come from
// the interface language; `viewer_core::details` hands over bare values.

use crate::fl;
use std::fmt::Write as _;
use viewer_core::details::{Exposure, Flash, FlashMode, Taken};

/// The camera's clock as it read, with no time zone added: the file does not say which. A
/// date the calendar does not have is shown by its numbers.
pub fn taken(taken: &Taken) -> String {
    match *taken {
        Taken::Text(ref text) => text.clone(),
        Taken::Clock {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } => {
            let time =
                chrono::NaiveDate::from_ymd_opt(i32::from(year), u32::from(month), u32::from(day))
                    .and_then(|date| {
                        date.and_hms_opt(u32::from(hour), u32::from(minute), u32::from(second))
                    });
            // `to_string()` panics on a malformed pattern, and the release profile aborts.
            let mut text = String::new();
            if let Some(time) = time
                && write!(text, "{}", time.format(&fl!("date-time-format-camera"))).is_ok()
            {
                return text;
            }
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
        }
    }
}

pub fn exposure(exposure: Exposure) -> String {
    let time = match exposure {
        Exposure::Fraction(per_second) => format!("1/{}", decimal(per_second, 1)),
        Exposure::Seconds(seconds) => decimal(seconds, 2),
    };
    fl!("image-exposure-value", time = time)
}

/// Two decimals: phone lenses are f/1.78 and f/1.85, and f/0.95 exists.
#[must_use]
pub fn aperture(f_number: f64) -> String {
    format!("f/{}", decimal(f_number, 2))
}

pub fn focal_length(mm: f64) -> String {
    fl!("image-focal-length-value", mm = decimal(mm, 2))
}

pub fn flash(flash: Flash) -> String {
    let mut parts = vec![match flash.fired {
        Some(true) => fl!("image-flash-fired"),
        Some(false) => fl!("image-flash-not-fired"),
        None => fl!("image-flash-absent"),
    }];
    parts.extend(flash.mode.map(|mode| match mode {
        FlashMode::Forced => fl!("image-flash-forced"),
        FlashMode::Suppressed => fl!("image-flash-suppressed"),
        FlashMode::Auto => fl!("image-flash-auto"),
    }));
    if flash.red_eye_reduction {
        parts.push(fl!("image-flash-red-eye"));
    }
    parts.join(", ")
}

pub fn latitude(degrees: f64) -> String {
    let hemisphere = if degrees < 0.0 { "south" } else { "north" };
    fl!(
        "image-latitude-value",
        degrees = decimal(degrees.abs(), 6),
        hemisphere = hemisphere
    )
}

pub fn longitude(degrees: f64) -> String {
    let hemisphere = if degrees < 0.0 { "west" } else { "east" };
    fl!(
        "image-longitude-value",
        degrees = decimal(degrees.abs(), 6),
        hemisphere = hemisphere
    )
}

#[must_use]
pub fn megapixels(width: u32, height: u32) -> String {
    format!("{:.1}", f64::from(width) * f64::from(height) / 1_000_000.0)
}

/// `value` with at most `places` decimals and no trailing zeros: 2.8, 11, 4.25.
fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

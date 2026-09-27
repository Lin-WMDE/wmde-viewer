// SPDX-License-Identifier: GPL-3.0-only

use cosmic::iced::Color;
use image::{DynamicImage, Rgb, Rgba, RgbaImage};
use tiny_skia::{LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

// WMDE: annotations are composited at the image's own depth and channel layout. The loader
// keeps most formats in their native pixel type (an RGB WebP, a grey or 16-bit TIFF), and
// the upstream `as_mut_rgba8().expect(..)` panicked on them, taking the unsaved edits along;
// converting to RGBA8 instead would cost a 16-bit file its depth and an opaque one an alpha
// channel it never had once saved.

/// Make `image` able to take a coloured overlay.
///
/// RGB and RGBA at 8 and 16 bits and in float stay as they are. Grey gains colour channels:
/// 8-bit grey becomes RGB8, 16-bit grey RGB16 or RGBA16. Anything else becomes RGBA8.
pub fn prepare_for_overlay(image: &mut DynamicImage) {
    let promoted = match image {
        DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageRgba8(_)
        | DynamicImage::ImageRgb16(_)
        | DynamicImage::ImageRgba16(_)
        | DynamicImage::ImageRgb32F(_)
        | DynamicImage::ImageRgba32F(_) => return,
        DynamicImage::ImageLuma8(_) => DynamicImage::ImageRgb8(image.to_rgb8()),
        DynamicImage::ImageLuma16(_) => DynamicImage::ImageRgb16(image.to_rgb16()),
        DynamicImage::ImageLumaA16(_) => DynamicImage::ImageRgba16(image.to_rgba16()),
        _ => DynamicImage::ImageRgba8(image.to_rgba8()),
    };
    *image = promoted;
}

/// Source-over of one colour onto the pixel at `(x, y)`, at the image's own depth.
///
/// `color` is straight, channels 0.0..=1.0, with coverage `alpha`. The image has been through
/// [`prepare_for_overlay`]; a layout that function does not leave is left untouched.
pub fn blend_pixel(image: &mut DynamicImage, x: u32, y: u32, color: [f32; 3], alpha: f32) {
    match image {
        DynamicImage::ImageRgb8(buf) => blend_opaque(buf.get_pixel_mut(x, y), color, alpha),
        DynamicImage::ImageRgba8(buf) => blend_translucent(buf.get_pixel_mut(x, y), color, alpha),
        DynamicImage::ImageRgb16(buf) => blend_opaque(buf.get_pixel_mut(x, y), color, alpha),
        DynamicImage::ImageRgba16(buf) => {
            blend_translucent(buf.get_pixel_mut(x, y), color, alpha);
        }
        DynamicImage::ImageRgb32F(buf) => blend_opaque(buf.get_pixel_mut(x, y), color, alpha),
        DynamicImage::ImageRgba32F(buf) => {
            blend_translucent(buf.get_pixel_mut(x, y), color, alpha);
        }
        _ => {}
    }
}

/// A channel value mapped to and from 0.0..=1.0.
trait Channel: Copy {
    fn to_unit(self) -> f32;
    fn from_unit(value: f32) -> Self;
}

impl Channel for u8 {
    fn to_unit(self) -> f32 {
        f32::from(self) / 255.0
    }

    fn from_unit(value: f32) -> Self {
        intensity_u8(value * 255.0)
    }
}

impl Channel for u16 {
    fn to_unit(self) -> f32 {
        f32::from(self) / 65535.0
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // reason: clamped to 0.0..=65535.0 then rounded, in-range for u16
    fn from_unit(value: f32) -> Self {
        (value * 65535.0).round().clamp(0.0, 65535.0) as Self
    }
}

impl Channel for f32 {
    fn to_unit(self) -> f32 {
        self
    }

    fn from_unit(value: f32) -> Self {
        value
    }
}

fn blend_opaque<C: Channel>(pixel: &mut Rgb<C>, color: [f32; 3], alpha: f32) {
    for (channel, src) in pixel.0.iter_mut().zip(color) {
        *channel = C::from_unit(src.mul_add(alpha, channel.to_unit() * (1.0 - alpha)));
    }
}

fn blend_translucent<C: Channel>(pixel: &mut Rgba<C>, color: [f32; 3], alpha: f32) {
    let dst_alpha = pixel.0[3].to_unit();
    let out_alpha = dst_alpha.mul_add(1.0 - alpha, alpha);
    if out_alpha <= 0.0 {
        return;
    }
    for (channel, src) in pixel.0[..3].iter_mut().zip(color) {
        *channel = C::from_unit(
            (channel.to_unit() * dst_alpha).mul_add(1.0 - alpha, src * alpha) / out_alpha,
        );
    }
    pixel.0[3] = C::from_unit(out_alpha);
}

pub fn build_path(f: impl FnOnce(&mut PathBuilder)) -> Option<Path> {
    let mut path_builder = PathBuilder::new();
    f(&mut path_builder);
    path_builder.finish()
}

// Quantize a 0.0..=1.0 color component to an 8-bit channel. Rounds to nearest
// and clamps so out-of-gamut inputs cannot wrap. `as u8` on a float already
// saturates, but truncates instead of rounding, hence the explicit round.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // reason: value clamped to 0.0..=255.0 then rounded, in-range for u8
fn channel_u8(component: f32) -> u8 {
    (component * 255.0).round().clamp(0.0, 255.0) as u8
}

// Quantize an already-scaled 0.0..=255.0 intensity to an 8-bit channel.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // reason: value clamped to 0.0..=255.0 then rounded, in-range for u8
const fn intensity_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Stroke a path onto a `DynamicImage` with the given color, width, and line cap.
///
/// # Panics
///
/// Panics if the image dimensions cannot back a pixmap (zero-sized or larger than tiny-skia
/// supports).
pub fn stroke_on_image(
    image: &mut DynamicImage,
    path: &Path,
    color: Color,
    width: f32,
    line_cap: LineCap,
) {
    prepare_for_overlay(image);
    let (img_width, img_height) = (image.width(), image.height());

    // Render stroke onto a transparent overlay to avoid premultiplication issues
    let mut overlay =
        Pixmap::new(img_width, img_height).expect("image dimensions should produce a valid pixmap");
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        channel_u8(color.r),
        channel_u8(color.g),
        channel_u8(color.b),
        channel_u8(color.a),
    );

    let stroke = Stroke {
        width,
        line_cap,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };

    overlay.stroke_path(path, &paint, &stroke, Transform::identity(), None);

    // Composite overlay onto the image buffer
    blend_overlay(image, &overlay);
}

/// Fill a path onto a `DynamicImage` with the given color.
///
/// # Panics
///
/// Panics if the image dimensions cannot back a pixmap (zero-sized or larger than tiny-skia
/// supports).
pub fn fill_on_image(image: &mut DynamicImage, path: &Path, color: Color) {
    prepare_for_overlay(image);
    let (img_width, img_height) = (image.width(), image.height());

    let mut overlay =
        Pixmap::new(img_width, img_height).expect("image dimensions should produce a valid pixmap");
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        channel_u8(color.r),
        channel_u8(color.g),
        channel_u8(color.b),
        channel_u8(color.a),
    );

    overlay.fill_path(
        path,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    blend_overlay(image, &overlay);
}

fn blend_overlay(image: &mut DynamicImage, overlay: &Pixmap) {
    let width = image.width() as usize;
    let DynamicImage::ImageRgba8(dst) = image else {
        for (index, src) in overlay.data().as_chunks::<4>().0.iter().enumerate() {
            let alpha = f32::from(src[3]) / 255.0;
            if alpha == 0.0 {
                continue;
            }
            // Un-premultiply the overlay
            let color = [src[0], src[1], src[2]].map(|c| f32::from(c) / 255.0 / alpha);
            #[allow(clippy::cast_possible_truncation)]
            // reason: x < width and y < height, both u32
            blend_pixel(
                image,
                (index % width) as u32,
                (index / width) as u32,
                color,
                alpha,
            );
        }
        return;
    };
    blend_overlay_rgba8(dst, overlay);
}

fn blend_overlay_rgba8(dst: &mut RgbaImage, overlay: &Pixmap) {
    let overlay_data = overlay.data();
    let dst_data: &mut [u8] = dst.as_mut();

    for (dst_pixel, src_chunk) in dst_data
        .chunks_exact_mut(4)
        .zip(overlay_data.chunks_exact(4))
    {
        let src_alpha = f32::from(src_chunk[3]) / 255.0;
        if src_alpha == 0.0 {
            continue;
        }

        // Un-premultiply the overlay
        let src_red = f32::from(src_chunk[0]) / src_alpha;
        let src_green = f32::from(src_chunk[1]) / src_alpha;
        let src_blue = f32::from(src_chunk[2]) / src_alpha;

        let dst_alpha = f32::from(dst_pixel[3]) / 255.0;
        let dst_red = f32::from(dst_pixel[0]);
        let dst_green = f32::from(dst_pixel[1]);
        let dst_blue = f32::from(dst_pixel[2]);

        // Source-over compositing
        let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);

        if out_alpha > 0.0 {
            dst_pixel[0] = intensity_u8(
                (dst_red * dst_alpha).mul_add(1.0 - src_alpha, src_red * src_alpha) / out_alpha,
            );
            dst_pixel[1] = intensity_u8(
                (dst_green * dst_alpha).mul_add(1.0 - src_alpha, src_green * src_alpha) / out_alpha,
            );
            dst_pixel[2] = intensity_u8(
                (dst_blue * dst_alpha).mul_add(1.0 - src_alpha, src_blue * src_alpha) / out_alpha,
            );
            dst_pixel[3] = intensity_u8(out_alpha * 255.0);
        }
    }
}

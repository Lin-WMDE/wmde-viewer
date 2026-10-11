// SPDX-License-Identifier: GPL-3.0-only

pub mod args;
mod budget;
pub mod cache;
pub mod clipboard;
pub mod details;
pub mod loader;
pub mod nav;
pub mod raw;
pub mod save;
mod svg_fonts;
pub mod wallpaper;

// Re-exports
pub use cache::{CachedImage, ImageCache};
pub use clipboard::{ClipboardImage, image_mime_type};
pub use details::{Details, read_details};
pub use loader::{LoadError, LoadedImage, is_drawing, load_image, load_thumbnail, read_dpi};
pub use nav::{
    NavState, get_image_dir, is_supported_image, scan_dir, set_sort_language, sort_files,
};
pub use save::{SaveError, keeps_depth, save_image};
pub use wallpaper::{WallpaperError, set_wallpaper};

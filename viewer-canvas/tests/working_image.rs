// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the canvas shares the cached picture's pixels until an edit replaces them, and the
// edit leaves the cached one as it was.

use image::DynamicImage;
use std::sync::Arc;
use viewer_canvas::ViewportManager;

#[test]
fn the_canvas_shares_the_picture_until_an_edit_replaces_it() {
    let original = Arc::new(DynamicImage::new_rgb8(4, 2));
    let mut canvas = ViewportManager::new();
    canvas.set_image(None, Some(Arc::clone(&original)));
    assert!(std::ptr::eq(
        canvas.working_image().expect("working image"),
        Arc::as_ptr(&original)
    ));

    canvas.map_working_image(DynamicImage::rotate90);
    let turned = canvas.working_image().expect("working image");
    assert_eq!((turned.width(), turned.height()), (2, 4));
    assert_eq!((original.width(), original.height()), (4, 2));
    assert_eq!(Arc::strong_count(&original), 1, "the canvas let go of it");

    canvas.replace_working_image(Arc::clone(&original));
    assert!(std::ptr::eq(
        canvas.working_image().expect("working image"),
        Arc::as_ptr(&original)
    ));
}

#[test]
fn an_empty_canvas_takes_no_working_image() {
    let mut canvas = ViewportManager::new();
    canvas.replace_working_image(Arc::new(DynamicImage::new_rgb8(1, 1)));
    canvas.map_working_image(DynamicImage::rotate90);
    assert!(canvas.working_image().is_none());
}

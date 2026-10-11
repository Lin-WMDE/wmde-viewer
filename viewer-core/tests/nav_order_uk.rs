// SPDX-License-Identifier: GPL-3.0-only

// WMDE: with a Ukrainian interface the album sorts as wmde-files does for it - Cyrillic names
// before Latin ones. The sort language is set once per process, so this file stands alone.

use std::fs;
use viewer_config::{SortMode, SortOrder};
use viewer_core::{scan_dir, set_sort_language};

#[tokio::test]
async fn ukrainian_puts_cyrillic_first() {
    set_sort_language("uk");
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "beach.jpg",
        "Їжак.png",
        "Зима.jpg",
        "apple.png",
        "ґанок.jpg",
    ] {
        fs::write(dir.path().join(name), b"x").unwrap();
    }
    let names: Vec<String> = scan_dir(dir.path(), false, SortMode::Name, SortOrder::Ascending)
        .await
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "ґанок.jpg",
            "Зима.jpg",
            "Їжак.png",
            "apple.png",
            "beach.jpg"
        ]
    );
}

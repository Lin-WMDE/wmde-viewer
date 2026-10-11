// SPDX-License-Identifier: GPL-3.0-only

use std::fs;
use std::path::{Path, PathBuf};

use viewer_config::{SortMode, SortOrder};
use viewer_core::{NavState, is_supported_image, scan_dir, sort_files};

fn test_images_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../test_images"))
}

#[tokio::test]
async fn scan_returns_only_supported_images() {
    let dir = test_images_dir();
    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert!(!images.is_empty());

    for path in &images {
        assert!(
            is_supported_image(path),
            "{} should be a supported image",
            path.display(),
        );
    }
}

#[tokio::test]
async fn scan_name_ascending_is_sorted() {
    let dir = test_images_dir();
    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert!(!images.is_empty());

    // Needs more than one entry to compare against the reversed order below.
    assert!(images.len() > 1);

    // Verify ascending and descending produce inverse orderings
    let mut rev = scan_dir(&dir, false, SortMode::Name, SortOrder::Descending).await;
    assert_eq!(images.len(), rev.len());
    rev.reverse();
    assert_eq!(images, rev);
}

#[tokio::test]
async fn scan_name_descending_is_reverse_sorted() {
    let dir = test_images_dir();
    let asc = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    let mut rev = scan_dir(&dir, false, SortMode::Name, SortOrder::Descending).await;
    assert_eq!(asc.len(), rev.len());

    rev.reverse();
    assert_eq!(asc, rev);
}

#[tokio::test]
async fn scan_size_ascending_is_ordered() {
    let dir = test_images_dir();
    let images = scan_dir(&dir, false, SortMode::Size, SortOrder::Ascending).await;
    assert!(!images.is_empty());

    for pair in images.windows(2) {
        let a_size = fs::metadata(&pair[0]).unwrap().len();
        let b_size = fs::metadata(&pair[1]).unwrap().len();
        assert!(a_size <= b_size);
    }
}

#[tokio::test]
async fn scan_date_ascending_is_ordered() {
    let dir = test_images_dir();
    let images = scan_dir(&dir, false, SortMode::Date, SortOrder::Ascending).await;
    assert!(!images.is_empty());

    for pair in images.windows(2) {
        let a_time = fs::metadata(&pair[0]).unwrap().modified().unwrap();
        let b_time = fs::metadata(&pair[1]).unwrap().modified().unwrap();
        assert!(a_time <= b_time);
    }
}

#[tokio::test]
async fn scan_hidden_files_off() {
    let dir = std::env::temp_dir().join("cosmic-viewer-nav-hidden-test");
    let _ = fs::create_dir_all(&dir);
    fs::write(dir.join("visible.jpg"), b"fake").unwrap();
    fs::write(dir.join(".hidden.png"), b"fake").unwrap();

    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    for path in &images {
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            !name.starts_with('.'),
            "hidden file {name} should be excluded"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scan_hidden_files_on() {
    let dir = std::env::temp_dir().join("cosmic-viewer-nav-hidden-test-on");
    let _ = fs::create_dir_all(&dir);
    fs::write(dir.join("visible.jpg"), b"fake").unwrap();
    fs::write(dir.join(".hidden.png"), b"fake").unwrap();

    let images = scan_dir(&dir, true, SortMode::Name, SortOrder::Ascending).await;
    let has_hidden = images
        .iter()
        .any(|p| p.file_name().unwrap().to_str().unwrap().starts_with('.'));
    assert!(has_hidden, "hidden file should be included when flag is on");

    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scan_nonexistent_dir_returns_empty() {
    let dir = Path::new("/no/such/directory/for/cosmic/viewer");
    let images = scan_dir(dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert!(images.is_empty());
}

#[tokio::test]
async fn scan_empty_dir_returns_empty() {
    let dir = std::env::temp_dir().join("cosmic-viewer-nav-empty-test");
    let _ = fs::create_dir_all(&dir);
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }

    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert!(images.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scan_filters_non_image_files() {
    let dir = std::env::temp_dir().join("cosmic-viewer-nav-filter-test");
    let _ = fs::create_dir_all(&dir);
    fs::write(dir.join("readme.txt"), b"text").unwrap();
    fs::write(dir.join("video.mp4"), b"video").unwrap();
    fs::write(dir.join("photo.jpg"), b"fake").unwrap();

    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert_eq!(images.len(), 1);
    assert_eq!(
        images[0].file_name().unwrap().to_str().unwrap(),
        "photo.jpg",
    );

    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scan_handles_symlinks_gracefully() {
    let dir = std::env::temp_dir().join("cosmic-viewer-nav-symlink-test");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    fs::write(dir.join("real.jpg"), b"fake image").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(dir.join("real.jpg"), dir.join("link.jpg"));
        // Also create a broken symlink
        let _ = symlink("/nonexistent/path.jpg", dir.join("broken.jpg"));
    }

    // Should not panic regardless of symlink state
    let images = scan_dir(&dir, false, SortMode::Name, SortOrder::Ascending).await;
    assert!(!images.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn is_supported_image_consistency() {
    // Supported extensions are accepted; unsupported ones are rejected.
    assert!(is_supported_image(Path::new("photo.jpg")));
    assert!(is_supported_image(Path::new("photo.jpeg")));
    assert!(is_supported_image(Path::new("photo.png")));
    assert!(is_supported_image(Path::new("photo.gif")));
    assert!(is_supported_image(Path::new("photo.webp")));
    assert!(is_supported_image(Path::new("photo.bmp")));
    assert!(is_supported_image(Path::new("photo.tiff")));
    assert!(is_supported_image(Path::new("photo.ico")));
    assert!(is_supported_image(Path::new("photo.avif")));
    assert!(is_supported_image(Path::new("photo.svg")));
    assert!(is_supported_image(Path::new("photo.heif")));
    assert!(is_supported_image(Path::new("photo.heic")));

    assert!(!is_supported_image(Path::new("file.txt")));
    assert!(!is_supported_image(Path::new("file.mp4")));
    assert!(!is_supported_image(Path::new("file.pdf")));
    assert!(!is_supported_image(Path::new("file")));
}

// WMDE: the file asked for is the one shown, even when the scan did not list it.
#[test]
fn a_file_the_scan_left_out_is_shown_on_its_own() {
    let dir = PathBuf::from("/pictures");
    let listed = vec![dir.join("a.jpg"), dir.join("b.jpg")];
    let asked = dir.join("photo.download");

    let mut nav = NavState::new();
    nav.set_images(dir.clone(), listed.clone(), Some(&asked));
    assert_eq!(nav.images(), std::slice::from_ref(&asked));
    assert_eq!(nav.current(), Some(&asked));

    // A listed file keeps its album, and no selection keeps the whole list unselected.
    nav.set_images(dir.clone(), listed.clone(), Some(&listed[1]));
    assert_eq!(nav.images(), listed.as_slice());
    assert_eq!(nav.index(), Some(1));
    nav.set_images(dir, listed.clone(), None);
    assert_eq!(nav.images(), listed.as_slice());
    assert_eq!(nav.index(), None);
}

#[test]
fn a_deleted_picture_leaves_the_album_and_the_next_one_takes_its_place() {
    let dir = PathBuf::from("/pictures");
    let listed: Vec<PathBuf> = ["a.jpg", "b.jpg", "c.jpg"]
        .iter()
        .map(|name| dir.join(name))
        .collect();
    let mut nav = NavState::new();
    nav.set_images(dir, listed.clone(), Some(&listed[0]));

    assert_eq!(nav.forget_current(), Some(&listed[1]));
    assert_eq!(nav.index(), Some(0));
    assert_eq!(nav.images(), &listed[1..]);

    // The last picture gives way to the one before it.
    nav.select(1);
    assert_eq!(nav.forget_current(), Some(&listed[1]));
    assert_eq!(nav.images(), std::slice::from_ref(&listed[1]));

    // The only picture leaves an empty album with nothing selected.
    assert_eq!(nav.forget_current(), None);
    assert!(nav.is_empty());
    assert_eq!(nav.index(), None);
    assert_eq!(nav.forget_current(), None);
}

// WMDE: several files on the command line are an album of their own: ordered as a directory is,
// whatever their names, and without those no longer there.
#[tokio::test]
async fn files_given_are_ordered_as_the_album_orders() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let b = dir.path().join("b10.jpg");
    let a = other.path().join("b2.png");
    let odd = dir.path().join("a1.scan");
    for path in [&b, &a, &odd] {
        fs::write(path, b"x").unwrap();
    }
    let gone = dir.path().join("gone.jpg");
    let given = vec![b.clone(), gone, a.clone(), odd.clone()];
    assert_eq!(
        sort_files(given.clone(), SortMode::Name, SortOrder::Ascending).await,
        [odd.clone(), a.clone(), b.clone()]
    );
    assert_eq!(
        sort_files(given, SortMode::Name, SortOrder::Descending).await,
        [b, a, odd]
    );
}

/// The names of a directory's pictures, in the album's order.
async fn album_names(names: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    for name in names {
        fs::write(dir.path().join(name), b"x").unwrap();
    }
    scan_dir(dir.path(), false, SortMode::Name, SortOrder::Ascending)
        .await
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

// WMDE: the album stands in the order wmde-files shows the directory in - its collator, numbers
// by value, letter case and punctuation as the file manager weighs them.
#[tokio::test]
async fn names_sort_as_the_file_manager_sorts_them() {
    let names = album_names(&[
        "DSC04126.ARW",
        "img10.png",
        "drawing.svgz",
        "Зима.jpg",
        "DSC_1842.NEF",
        "img2.png",
        "Img3.png",
    ])
    .await;
    assert_eq!(
        names,
        [
            "drawing.svgz",
            "DSC_1842.NEF",
            "DSC04126.ARW",
            "img2.png",
            "Img3.png",
            "img10.png",
            "Зима.jpg",
        ]
    );
}

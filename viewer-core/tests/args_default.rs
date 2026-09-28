// SPDX-License-Identifier: GPL-3.0-only

// WMDE: nothing on the command line opens the pictures directory by its xdg-user-dirs name, not
// `$HOME/Pictures`. One test function: the directories come from the environment, which every
// test in a binary shares.

use viewer_core::args::start_paths;

#[test]
fn nothing_given_opens_the_pictures_directory_by_its_own_name() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let pictures = home.path().join("Зображення");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("user-dirs.dirs"),
        "XDG_PICTURES_DIR=\"$HOME/Зображення\"\n",
    )
    .unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", &config);
    }

    // Not there yet: the home directory.
    assert_eq!(start_paths(Vec::new()), [home.path().to_path_buf()]);
    std::fs::create_dir(&pictures).unwrap();
    assert_eq!(start_paths(Vec::new()), std::slice::from_ref(&pictures));
    // Given and naming nothing: nothing, not the pictures directory.
    assert!(start_paths(["".into()]).is_empty());
    // Something given: that, and no pictures directory.
    assert_eq!(
        start_paths(["/x.jpg".into()]),
        [std::path::PathBuf::from("/x.jpg")]
    );
}

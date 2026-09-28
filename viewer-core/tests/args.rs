// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the command line keeps every path it is given, as `%F` hands them over.

use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};
use viewer_core::args::paths;

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn every_path_is_kept_in_the_order_given() {
    assert_eq!(
        paths(args(&["/a/b.jpg", "/c.png", "/d"])),
        [
            PathBuf::from("/a/b.jpg"),
            PathBuf::from("/c.png"),
            PathBuf::from("/d")
        ]
    );
}

#[test]
fn a_file_url_loses_its_percent_encoding() {
    assert_eq!(
        paths(args(&[
            "file:///home/someone/Pictures/%D0%BD%D0%B0%20%D0%BC%D0%BE%D1%80%D1%96.jpg"
        ])),
        [PathBuf::from("/home/someone/Pictures/на морі.jpg")]
    );
}

#[test]
fn a_file_url_on_another_host_and_an_empty_argument_name_nothing() {
    assert_eq!(
        paths(args(&["file://remote/share/x.jpg", "", "/kept.jpg"])),
        [PathBuf::from("/kept.jpg")]
    );
}

#[test]
fn a_relative_path_is_made_absolute_against_the_working_directory() {
    let here = std::env::current_dir().unwrap();
    assert_eq!(
        paths(args(&["holiday.jpg", "../up.png"])),
        [here.join("holiday.jpg"), here.join("../up.png")]
    );
}

#[test]
fn a_name_that_is_not_utf8_is_kept_as_it_is() {
    let name = OsStr::from_bytes(b"/pictures/caf\xe9.jpg");
    assert_eq!(paths([name.to_os_string()]), [PathBuf::from(name)]);
}

#[test]
fn only_a_file_url_is_read_as_one() {
    // A relative name that happens to start with `file:` is a name.
    let here = std::env::current_dir().unwrap();
    assert_eq!(
        paths(args(&["file:scan.jpg"])),
        [here.join("file:scan.jpg")]
    );
}

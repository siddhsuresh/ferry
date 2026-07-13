//! Behavioural tests for keel-vfs's device-free surface.
//!
//! The `Walk` / `UploadFiles` / `DownloadFiles` ops all require a live
//! `MtpSession`, whose only constructor (`configure()`) runs the real
//! OpenSession ladder over a `Transport` and drives the real transaction engine +
//! codec for every op. Its fields are `pub(crate)`, so keel-vfs (a separate
//! crate) cannot build one without a byte-level scripted `FakeDevice` — and that
//! scripting lives in keel-mtp, so replaying it here would re-test
//! keel-mtp/keel-proto rather than keel-vfs. This file therefore covers the
//! *pure* (no-device) subjects: the `fix_slash` / `get_full_path` / `extension`
//! tables, `percent`, and the hidden/disallowed classification.
//!
//! Integration tests are a separate crate and can only reach keel-vfs's public
//! surface plus its dev-dependencies. `keel_proto` is a normal (not dev)
//! dependency, so its types are not nameable here — every assertion below uses
//! only primitive arguments, which is why no `ObjectInfo`-based case (e.g.
//! `is_object_a_dir`) appears. The `eq_fold` Unicode case-insensitivity is
//! exercised where its function lives — `path::eq_fold` is private, so its
//! Unicode cases are pinned in path.rs's in-module `#[cfg(test)]` block.

use keel_vfs::VfsError;
use keel_vfs::object::extension;
use keel_vfs::path::{fix_slash, get_full_path};
use keel_vfs::progress::percent;
use keel_vfs::walk::{DISALLOWED_FILES, is_disallowed_files, is_hidden_file};

/// `fix_slash`: the full path-normalization table.
#[test]
fn fix_slash_full_go_table() {
    let cases: &[(&str, &str)] = &[
        ("", "/"),
        (".", "/"),
        ("/./", "/"),
        ("././", "/"),
        ("/../", "/"),
        ("/", "/"),
        ("//", "/"),
        ("/abc", "/abc"),
        ("//bcd", "/bcd"),
        ("/cde/", "/cde"),
        ("/def//", "/def"),
        ("efg/", "/efg"),
        ("fgh", "/fgh"),
        ("ghi/124", "/ghi/124"),
        ("hij/124/", "/hij/124"),
        ("/ijk/124/", "/ijk/124"),
    ];
    for (input, want) in cases {
        assert_eq!(&fix_slash(input), want, "fixSlash({input:?})");
    }
}

/// `get_full_path`: the full parent+filename join table.
#[test]
fn get_full_path_full_go_table() {
    let cases: &[(&str, &str, &str)] = &[
        ("/", "abc", "/abc"),
        ("//", "bcd", "/bcd"),
        ("/", "cde/", "/cde"),
        ("/def", "abc/", "/def/abc"),
        ("/efg/", "abc/", "/efg/abc"),
    ];
    for (parent, filename, want) in cases {
        assert_eq!(
            &get_full_path(parent, filename),
            want,
            "getFullPath({parent:?}, {filename:?})"
        );
    }
}

/// `extension`: the full 19-row table — including the `tar` two-part special
/// case (e.g. `.tar.gz` → `tar.gz`), the base-name behaviour (rows with `/`), the
/// leading-dot dotfiles, and the `is_dir => ""` rows.
#[test]
fn extension_full_go_table() {
    let cases: &[(&str, bool, &str)] = &[
        ("", false, ""),
        ("abc.xyz.tar.gz", false, "tar.gz"),
        ("abc.xyz.tar.tar", false, "tar.tar"),
        ("xyz.tar.gz", false, "tar.gz"),
        ("tar.gz", false, "gz"),
        ("abc.gz", false, "gz"),
        (".gz", false, "gz"),
        (".tar", false, "tar"),
        (".tar.gz", false, "tar.gz"),
        ("tar.tar.gz", false, "tar.gz"),
        (".htaccess", false, "htaccess"),
        ("abc.txt", false, "txt"),
        ("abc", false, ""),
        (
            "github.com/ganeshrvel/one-archiver/e2e_list_test.go",
            false,
            "go",
        ),
        ("one-archiver/e2e_list_test.go", false, "go"),
        ("e2e_list_test.go/.go.psd", false, "psd"),
        ("abc", true, ""),
        ("abc.tar", true, ""),
        ("abc.tar.gz", true, ""),
    ];
    for (filename, is_dir, want) in cases {
        assert_eq!(
            &extension(filename, *is_dir),
            want,
            "extension({filename:?}, isDir={is_dir})"
        );
    }
}

/// `percent`: the load-bearing `total <= 0 => 0` guard (why every bulk
/// percentage is 0 when preprocessing is off), plus normal ratios.
#[test]
fn percent_zero_total_and_normal() {
    // total <= 0 => 0 (the guard).
    assert_eq!(percent(5.0, 0.0), 0.0);
    assert_eq!(percent(0.0, 0.0), 0.0);
    assert_eq!(percent(123.0, 0.0), 0.0);
    assert_eq!(percent(1.0, -1.0), 0.0);
    // normal.
    assert_eq!(percent(1.0, 4.0), 25.0);
    assert_eq!(percent(4.0, 4.0), 100.0);
    assert_eq!(percent(1.0, 2.0), 50.0);
}

/// `is_hidden_file`: `len > 0 && filename[0..1] == "."`. A leading-dot child is
/// skipped; the root is exempt — that exemption is enforced in walk.rs, not this
/// classifier.
#[test]
fn is_hidden_file_leading_dot() {
    assert!(is_hidden_file(".DS_Store"));
    assert!(is_hidden_file(".hidden"));
    assert!(is_hidden_file(".1")); // leading-dot name
    assert!(is_hidden_file(".a.txt")); // leading-dot name
    assert!(!is_hidden_file("visible"));
    assert!(!is_hidden_file("a.txt"));
    assert!(!is_hidden_file("")); // the len > 0 guard
}

/// `is_disallowed_files` over `DISALLOWED_FILES`. The comparison is `==`, so it
/// is an EXACT match — not a substring or case fold.
#[test]
fn is_disallowed_files_exact_match() {
    // The two members of the disallowed list.
    assert!(is_disallowed_files(".DS_Store"));
    assert!(is_disallowed_files("[-----DS_Store.mtp.test----].txt"));
    // Exact-match only: no substring, no case fold.
    assert!(!is_disallowed_files("a.DS_Store"));
    assert!(!is_disallowed_files(".DS_Store.bak"));
    assert!(!is_disallowed_files("ds_store"));
    assert!(!is_disallowed_files("photo.jpg"));
    // The list itself is exactly these two entries.
    assert_eq!(
        DISALLOWED_FILES,
        [".DS_Store", "[-----DS_Store.mtp.test----].txt"]
    );
}

/// The `VfsError::Cancelled` variant (which backs the upload/download
/// `should_cancel` seam) must render as exactly `"transfer cancelled by user"`.
/// keel-ffi maps this variant to `ErrorTransferCancelled` by type, but the error
/// mapper also has a substring fallback on that message — required for wire
/// compatibility with the app, so this pins the string.
#[test]
fn cancelled_display_matches_go_transfer_cancelled_string() {
    assert_eq!(
        VfsError::Cancelled.to_string(),
        "transfer cancelled by user"
    );
    assert!(
        VfsError::Cancelled
            .to_string()
            .contains("transfer cancelled by user")
    );
}

/// The `NoStorage` message — pinned so the FFI error mapper's text keeps matching.
#[test]
fn no_storage_message_matches_go() {
    assert_eq!(VfsError::NoStorage.to_string(), "no storage found");
}

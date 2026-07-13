//! Behavioural parity tests — SELECTIVE ports of the go-mtpx test suite.
//!
//! Why these and not a full `Walk`/`UploadFiles`/`DownloadFiles` port: go-mtpx's
//! `walk_test.go` / `upload_files_test.go` / `download_files_test.go` are all
//! device tests — they call `Initialize(Init{})`, `FetchStorages`, then assert
//! against a fixture tree (`/mtp-test-files`) physically present on a plugged-in
//! phone. keel's path-level ops are generic over `MtpSession<T: Transport>`, but
//! `MtpSession`'s only constructor is `configure()`, which runs the real
//! OpenSession ladder over a `Transport` and drives the real transaction engine +
//! codec for every op; its fields are `pub(crate)`, so keel-vfs (a separate crate)
//! cannot even build one without a byte-level scripted `FakeDevice`. That scripting
//! is keel-mtp's fake-device surface (plan M2), not the vfs gate's — replaying it
//! here would re-test keel-mtp/keel-proto, not keel-vfs. So per the gate task's
//! fallback clause, this file ports the go-mtpx tests whose subjects are *pure*
//! (no device): the `utils_test.go` tables (`fixSlash`, `getFullPath`,
//! `extension`) verbatim, `Percent`, and the hidden/disallowed classification.
//!
//! The device-dependent walk/dirops/transfer *semantics* were cross-checked
//! against the Go source line-by-line during the gate pass; the divergence list
//! is in the gate report, not here.
//!
//! Integration tests are a separate crate and can only reach keel-vfs's public
//! surface plus its dev-dependencies. `keel_proto` is a normal (not dev)
//! dependency, so its types are not nameable here — every assertion below uses
//! only primitive arguments, which is why no `ObjectInfo`-based case (e.g.
//! `is_object_a_dir`) appears. The `strings.EqualFold` Unicode case-insensitivity
//! (go-mtpx helpers.go:98/108) is exercised where its function actually lives —
//! `path::eq_fold` is private, so its Unicode cases are pinned in path.rs's
//! in-module `#[cfg(test)]` block.

use keel_vfs::VfsError;
use keel_vfs::object::extension;
use keel_vfs::path::{fix_slash, get_full_path};
use keel_vfs::progress::percent;
use keel_vfs::walk::{DISALLOWED_FILES, is_disallowed_files, is_hidden_file};

/// go-mtpx `TestUtils` / `Test fixSlash` (utils_test.go:13-22). The full table,
/// verbatim — every entry from `filenameList` mapped to `dirList`.
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

/// go-mtpx `Test getFullPath` (utils_test.go:24-62). The full table, verbatim.
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

/// go-mtpx `Test extension` (utils_test.go:64-155). The full 19-row table,
/// verbatim — including the `tar` two-part special case (`allowedSecondExtensions`,
/// const.go:20), the `filepath.Split` base-name behaviour (rows with `/`), the
/// leading-dot dotfiles, and the `isDir => ""` rows.
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

/// go-mtpx `Percent` (utils.go:177-183). The load-bearing `total <= 0 => 0` guard
/// (why every bulk percentage is 0 when preprocessing is off — pinned by the
/// transfer tests' `BulkFileSize.Progress == 0` assertions), plus normal ratios.
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

/// go-mtpx `isHiddenFile` (utils.go:258): `len > 0 && filename[0:1] == "."`.
/// Semantics pinned by walk_test.go's `skipHiddenFiles` cases (a leading-dot child
/// is skipped; the root is exempt — that exemption is enforced in walk.rs, not the
/// classifier, and is cross-checked in the gate report).
#[test]
fn is_hidden_file_leading_dot() {
    assert!(is_hidden_file(".DS_Store"));
    assert!(is_hidden_file(".hidden"));
    assert!(is_hidden_file(".1")); // walk_test.go:482 (mock_dir4/.1)
    assert!(is_hidden_file(".a.txt")); // walk_test.go:497 (mock_dir4/.a.txt)
    assert!(!is_hidden_file("visible"));
    assert!(!is_hidden_file("a.txt"));
    assert!(!is_hidden_file("")); // Go's `len > 0` guard
}

/// go-mtpx `isDisallowedFiles` (utils.go:165) over `disallowedFiles` (const.go:18).
/// `StringContains` compares with `==` (utils.go:200), so it is an EXACT match, not
/// a substring or fold — pinned by walk_test.go:408/425 (the `.DS_Store` root errors
/// and the `[-----DS_Store.mtp.test----].txt` sentinel is walked/skipped).
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
    // The list itself is exactly these two entries (const.go:18).
    assert_eq!(
        DISALLOWED_FILES,
        [".DS_Store", "[-----DS_Store.mtp.test----].txt"]
    );
}

/// Reconciliation check (gate pass): the keel-only `VfsError::Cancelled` variant
/// (added to unblock the upload/download authors' `should_cancel` seam) must render
/// as Go's `TransferCancelledError.Error()` string, `"transfer cancelled by user"`
/// (ferry/kernel/send_to_js/errors.go:17). keel-ffi normally maps this variant to
/// `ErrorTransferCancelled` by type, but the Go mapper also has a substring fallback
/// (`strings.Contains(e.Error(), "transfer cancelled by user")`,
/// send_to_js/helpers.go:15-17); this keeps that fallback firing.
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

/// go-mtpx `NoStorageError` message (main.go:63) — pinned so the FFI error mapper's
/// text keeps matching.
#[test]
fn no_storage_message_matches_go() {
    assert_eq!(VfsError::NoStorage.to_string(), "no storage found");
}

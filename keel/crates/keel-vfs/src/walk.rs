//! `walk` — pre-order DFS over a device directory tree, go-mtpx parity.
//!
//! Faithful port of go-mtpx `main.go::Walk` (18-171) plus its recursive worker
//! `helpers.go::proccessWalk` (319-384). Behaviour that is load-bearing and
//! preserved verbatim (each cited to its Go origin):
//!   * pre-order DFS in raw device handle order (the order `GetObjectHandles`
//!     returns — no sorting);
//!   * the root directory itself is never surfaced to the callback — only its
//!     descendants (main.go:165, proccessWalk starts one level down);
//!   * a root that is a *file* yields exactly one callback (main.go:154-163);
//!   * `skip_hidden_files`: a child whose name starts with '.' is skipped, but the
//!     root is exempt (the hidden test lives only in proccessWalk, helpers.go:342);
//!   * `skip_disallowed_files`: a disallowed ROOT is a hard `InvalidPath` error
//!     (main.go:146-151) while a disallowed CHILD is silently skipped
//!     (helpers.go:347) — the asymmetry is deliberate;
//!   * unreadable children are silently skipped (helpers.go:334-337);
//!   * a callback error aborts the whole walk (helpers.go:357-360, main.go:155).
//!
//! Generic over [`Transport`], matching the rest of keel-vfs (path.rs/object.rs) —
//! only device.rs needs the concrete USB type.
//!
//! Deviation from Go's return shape (documented for the gate): Go returns
//! `(objectId, totalFiles, totalDirectories, err)` and, on the callback/recursion
//! error path, hands back the *partial* counts alongside the error. keel returns
//! `Result<(objectId, totalFiles, totalDirectories), VfsError>`, so partial counts
//! are dropped on the error path. This is behaviour-preserving for every observer:
//! Ferry's FFI `_walk` (ferry/kernel/helpers.go:139) discards the counts entirely
//! (`_, _, _, err = mtpx.Walk(...)`) and reads only the callback-accumulated
//! slice, and go-mtpx's own `DownloadFiles` reads the counts only on the success
//! path (`if err != nil { return ... }` before `totalFiles += _totalFiles`,
//! main.go:634). No caller reads counts-with-error.

use keel_mtp::{MtpSession, Transport};

use crate::error::VfsError;
use crate::object::{FileInfo, from_object_id};
use crate::path::{FORMAT_ALL, get_object_from_path};

/// go-mtpx `const.go:18`. Names matched here are rejected (root) or skipped
/// (child) when `skip_disallowed_files` is set. The second entry is the test
/// sentinel go-mtpx ships so its suite can exercise the disallowed path without
/// a real `.DS_Store`; it is load-bearing (walk_test.go:408/411 depend on it).
pub const DISALLOWED_FILES: [&str; 2] = [".DS_Store", "[-----DS_Store.mtp.test----].txt"];

/// go-mtpx `WalkCb` (structs.go:34): `func(objectId, fi, err) error`. The Go
/// callback's always-nil `err` parameter is dropped — go-mtpx never passes a
/// non-nil error to it (every call site passes `nil`: helpers.go:357, main.go:155),
/// so the leading `if err != nil { return err }` guard every caller wrote was
/// dead code. Callers that need to keep a `FileInfo` past the call clone it.
pub type WalkCb<'a> = dyn FnMut(u32, &FileInfo) -> Result<(), VfsError> + 'a;

/// List a directory tree. `recursive` controls descent; `skip_disallowed_files`
/// and `skip_hidden_files` control the two filter classes. Returns
/// `(object_id, total_files, total_directories)` where `object_id` is the object
/// the `full_path` resolved to. Directory counts exclude the root itself.
///
/// Port of `main.go::Walk` (18-171).
pub fn walk<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
    recursive: bool,
    skip_disallowed_files: bool,
    skip_hidden_files: bool,
    cb: &mut dyn FnMut(u32, &FileInfo) -> Result<(), VfsError>,
) -> Result<(u32, i64, i64), VfsError> {
    // main.go:140 — resolve the path to its object (InvalidPath on miss / "" ).
    let fi = get_object_from_path(session, storage_id, full_path)?;

    // main.go:146-151 — a disallowed ROOT is a hard error (children only skip).
    if skip_disallowed_files && is_disallowed_files(&fi.name) {
        return Err(VfsError::InvalidPath(format!(
            "disallowed file {}",
            fi.name
        )));
    }

    // main.go:154-163 — root is a file ⇒ exactly one callback, no recursion.
    if !fi.is_dir {
        cb(fi.object_id, &fi)?;
        return Ok((fi.object_id, 1, 0));
    }

    // main.go:165 — root is a dir ⇒ walk its children (root not surfaced). The
    // RAW `full_path` (not the fixed one) is threaded down as the parent path,
    // exactly as Go passes `fullPath` into proccessWalk.
    let (total_files, total_directories) = process_walk(
        session,
        storage_id,
        fi.object_id,
        full_path,
        recursive,
        skip_disallowed_files,
        skip_hidden_files,
        cb,
    )?;

    Ok((fi.object_id, total_files, total_directories))
}

/// Recursive worker — port of `helpers.go::proccessWalk` (319-384).
///
/// `object_id`/`parent_full_path` are Go's `FileProp{ObjectId, FullPath}`; the
/// object id here is always set (a real handle or the 0xFFFFFFFF root), so Go's
/// `GetObjectFromObjectIdOrPath` always dispatches to the by-id branch — we call
/// `from_object_id` directly (byte-identical device traffic).
#[allow(clippy::too_many_arguments)]
fn process_walk<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    object_id: u32,
    parent_full_path: &str,
    recursive: bool,
    skip_disallowed_files: bool,
    skip_hidden_files: bool,
    cb: &mut dyn FnMut(u32, &FileInfo) -> Result<(), VfsError>,
) -> Result<(i64, i64), VfsError> {
    // helpers.go:320 — re-resolve the directory (a real GetObjectInfo round-trip
    // for non-root; root short-circuits with no device call). Preserved for
    // conformance fidelity even though its object id equals `object_id`.
    let dir = from_object_id(session, object_id, parent_full_path)?;

    // helpers.go:326-329 — list the children; a failure is a ListDirectory error
    // (distinct from GetObjectFromParentIdAndFilename, which wraps as FileObject).
    let handles = session
        .object_handles(storage_id, FORMAT_ALL, dir.object_id)
        .map_err(VfsError::ListDirectory)?;

    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;

    for obj_id in handles {
        // helpers.go:334-337 — unreadable children are silently skipped.
        let fi = match from_object_id(session, obj_id, parent_full_path) {
            Ok(fi) => fi,
            Err(_) => continue,
        };

        // helpers.go:342 — hidden (unix-style) child skipped.
        if skip_hidden_files && is_hidden_file(&fi.name) {
            continue;
        }
        // helpers.go:347 — disallowed child skipped (a disallowed root errored).
        if skip_disallowed_files && is_disallowed_files(&fi.name) {
            continue;
        }

        // helpers.go:351-355 — count BEFORE invoking the callback.
        if fi.is_dir {
            total_directories += 1;
        } else {
            total_files += 1;
        }

        // helpers.go:357-360 — a callback error aborts the entire walk.
        cb(obj_id, &fi)?;

        // helpers.go:362-370 — descend only when asked, and only into dirs.
        if !recursive || !fi.is_dir {
            continue;
        }

        let (child_files, child_dirs) = process_walk(
            session,
            storage_id,
            obj_id,
            &fi.full_path,
            recursive,
            skip_disallowed_files,
            skip_hidden_files,
            cb,
        )?;
        total_files += child_files;
        total_directories += child_dirs;
    }

    Ok((total_files, total_directories))
}

/// go-mtpx `utils.go::isDisallowedFiles` (165) — exact-match membership in
/// [`DISALLOWED_FILES`] (Go's `StringContains` compares with `==`, not a fold).
/// Shared with `upload`/`download` (they filter on the same list, main.go:367 /
/// helpers.go:467); import it as `crate::walk::is_disallowed_files`.
pub fn is_disallowed_files(name: &str) -> bool {
    DISALLOWED_FILES.contains(&name)
}

/// go-mtpx `utils.go::isHiddenFile` (258) — unix-style hidden test: the name
/// begins with a '.'. Empty names are not hidden (Go guards `len > 0` first;
/// `starts_with` is already false for "").
pub fn is_hidden_file(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure-helper coverage only: walk/process_walk need a live MtpSession, driven
    // by a FakeDevice in the crate/gate-level integration tests once device.rs and
    // a fake transport land.

    #[test]
    fn disallowed_is_exact_match_not_substring() {
        // go-mtpx StringContains uses `==` (utils.go:197-205).
        assert!(is_disallowed_files(".DS_Store"));
        assert!(is_disallowed_files("[-----DS_Store.mtp.test----].txt"));
        assert!(!is_disallowed_files("a.DS_Store")); // substring must NOT match
        assert!(!is_disallowed_files("ds_store")); // case-sensitive
        assert!(!is_disallowed_files("photo.jpg"));
    }

    #[test]
    fn hidden_is_leading_dot() {
        assert!(is_hidden_file(".hidden"));
        assert!(is_hidden_file(".DS_Store"));
        assert!(!is_hidden_file("visible"));
        assert!(!is_hidden_file("")); // len==0 guard in Go
        assert!(!is_hidden_file("a.txt"));
    }
}

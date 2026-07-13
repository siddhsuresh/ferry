//! `walk` — pre-order DFS over a device directory tree.
//!
//! Walks a directory tree via its recursive worker `process_walk`. Load-bearing
//! behaviour:
//!   * pre-order DFS in raw device handle order (the order `GetObjectHandles`
//!     returns — no sorting);
//!   * the root directory itself is never surfaced to the callback — only its
//!     descendants (the walk starts one level down);
//!   * a root that is a *file* yields exactly one callback;
//!   * `skip_hidden_files`: a child whose name starts with '.' is skipped, but the
//!     root is exempt (the hidden test applies only to children);
//!   * `skip_disallowed_files`: a disallowed ROOT is a hard `InvalidPath` error
//!     while a disallowed CHILD is silently skipped — the asymmetry is deliberate;
//!   * unreadable children are silently skipped;
//!   * a callback error aborts the whole walk.
//!
//! Generic over [`Transport`], matching the rest of keel-vfs (path.rs/object.rs) —
//! only device.rs needs the concrete USB type.
//!
//! Returns `Result<(objectId, totalFiles, totalDirectories), VfsError>`. On the
//! callback/recursion error path the partial counts are dropped: no caller reads
//! counts alongside an error — the FFI walk discards the counts entirely and reads
//! only the callback-accumulated slice, and the download path reads the counts
//! only after checking for success.

use keel_mtp::{MtpSession, Transport};

use crate::error::VfsError;
use crate::object::{FileInfo, from_object_id};
use crate::path::{FORMAT_ALL, get_object_from_path};

/// Names matched here are rejected (root) or skipped (child) when
/// `skip_disallowed_files` is set. The second entry is a test sentinel so the
/// suite can exercise the disallowed path without a real `.DS_Store`; it is
/// load-bearing.
pub const DISALLOWED_FILES: [&str; 2] = [".DS_Store", "[-----DS_Store.mtp.test----].txt"];

/// The walk callback. It receives `(objectId, fi)`; there is no error parameter —
/// the walk never invokes the callback with a failure. Callers that need to keep a
/// `FileInfo` past the call clone it.
pub type WalkCb<'a> = dyn FnMut(u32, &FileInfo) -> Result<(), VfsError> + 'a;

/// List a directory tree. `recursive` controls descent; `skip_disallowed_files`
/// and `skip_hidden_files` control the two filter classes. Returns
/// `(object_id, total_files, total_directories)` where `object_id` is the object
/// the `full_path` resolved to. Directory counts exclude the root itself.
pub fn walk<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
    recursive: bool,
    skip_disallowed_files: bool,
    skip_hidden_files: bool,
    cb: &mut dyn FnMut(u32, &FileInfo) -> Result<(), VfsError>,
) -> Result<(u32, i64, i64), VfsError> {
    // Resolve the path to its object (InvalidPath on miss / "").
    let fi = get_object_from_path(session, storage_id, full_path)?;

    // A disallowed ROOT is a hard error (children only skip).
    if skip_disallowed_files && is_disallowed_files(&fi.name) {
        return Err(VfsError::InvalidPath(format!(
            "disallowed file {}",
            fi.name
        )));
    }

    // Root is a file ⇒ exactly one callback, no recursion.
    if !fi.is_dir {
        cb(fi.object_id, &fi)?;
        return Ok((fi.object_id, 1, 0));
    }

    // Root is a dir ⇒ walk its children (root not surfaced). The RAW `full_path`
    // (not the fixed one) is threaded down as the parent path.
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

/// Recursive worker for [`walk`].
///
/// The object id here is always set (a real handle or the 0xFFFFFFFF root), so we
/// call `from_object_id` directly rather than dispatching through the
/// handle-or-path resolver.
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
    // Re-resolve the directory (a real GetObjectInfo round-trip for non-root; root
    // short-circuits with no device call) even though its object id equals
    // `object_id`.
    let dir = from_object_id(session, object_id, parent_full_path)?;

    // List the children; a failure is a ListDirectory error (distinct from
    // get_object_from_parent_id_and_filename, which wraps as FileObject).
    let handles = session
        .object_handles(storage_id, FORMAT_ALL, dir.object_id)
        .map_err(VfsError::ListDirectory)?;

    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;

    for obj_id in handles {
        // Unreadable children are silently skipped.
        let fi = match from_object_id(session, obj_id, parent_full_path) {
            Ok(fi) => fi,
            Err(_) => continue,
        };

        // Hidden (unix-style) child skipped.
        if skip_hidden_files && is_hidden_file(&fi.name) {
            continue;
        }
        // Disallowed child skipped (a disallowed root errored).
        if skip_disallowed_files && is_disallowed_files(&fi.name) {
            continue;
        }

        // Count BEFORE invoking the callback.
        if fi.is_dir {
            total_directories += 1;
        } else {
            total_files += 1;
        }

        // A callback error aborts the entire walk.
        cb(obj_id, &fi)?;

        // Descend only when asked, and only into dirs.
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

/// Exact-match (case-sensitive, not a fold) membership in [`DISALLOWED_FILES`].
/// Shared with `upload`/`download`, which filter on the same list; import it as
/// `crate::walk::is_disallowed_files`.
pub fn is_disallowed_files(name: &str) -> bool {
    DISALLOWED_FILES.contains(&name)
}

/// Unix-style hidden test: the name begins with a '.'. Empty names are not hidden
/// (`starts_with` is already false for "").
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
        // Membership uses `==`, so only exact names match.
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
        assert!(!is_hidden_file("")); // empty is not hidden
        assert!(!is_hidden_file("a.txt"));
    }
}

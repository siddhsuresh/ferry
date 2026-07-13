//! `upload_files` — local disk → device: enumerate a local tree, mirror it onto
//! the device, and stream each file across with progress reporting.
//!
//! Operates on a generic `&mut MtpSession<T>`, so it drives both the real
//! `UsbTransport` and the test `FakeDevice`. The FFI reaches it via
//! `Device::session_mut()`.
//!
//! Load-bearing behaviours:
//!   * `ObjectFormat` is ALWAYS `OFC_Undefined` (0x3000), never inferred from the
//!     file extension;
//!   * `CompressedSize` saturates to 0xFFFFFFFF for files > 4 GiB;
//!   * the per-file existence check is Unicode-case-insensitive, and an overwrite
//!     deletes the existing object then recreates it;
//!   * `bulk_files_sent` is incremented **before** the transfer, while
//!     `pinfo.files_sent` is updated **after** it — an off-by-one the wire contract
//!     depends on (during a file's own ticks, `files_sent` still reflects the count
//!     before it);
//!   * per-chunk instantaneous speed and bulk/active percentages are computed on
//!     every tick;
//!   * symlinks are never followed (symlink-aware traversal + an explicit skip);
//!   * `.DS_Store` and the test sentinel are filtered out.
//!
//! Directory memoisation: the walk callback caches each destination dir's object
//! id in a per-source dict so children reuse their parent's id. A pre-order walk
//! always visits a parent before its children, so the cache hit always fires; the
//! create-and-memoise paths below exist as a correct fallback for the miss.
//!
//! Cancellation: `should_cancel` is checked once per preprocessed file and once
//! per progress tick. On a fire we abort with the distinct `VfsError::Cancelled`,
//! which keel-ffi maps to `ErrorTransferCancelled`.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::path::Path;
use std::time::SystemTime;

use keel_mtp::{MtpError, MtpSession, Transport};
use keel_proto::ObjectInfo;

use crate::error::VfsError;
use crate::object::{FileInfo, extension};
use crate::path::{FileProp, fix_slash, get_full_path, get_object_from_parent_id_and_filename};
use crate::progress::{ProgressInfo, TransferStatus, percent, transfer_rate};
use crate::walk::is_disallowed_files;

// `map_source_path_to_destination_path` and `go_filepath_dir` are used only by
// the transfer modules. They are defined privately here (and duplicated in
// download.rs) so each transfer module stands alone; hoist to `path.rs` if
// preferred.

use crate::dirops::{delete_file, make_directory};

/// `OFC_Undefined`. `SendObjectInfo` for an uploaded file ALWAYS uses this
/// format — the format is never derived from the extension. A private literal so
/// this file does not couple to whatever name `keel_proto::consts` exposes for it.
const OFC_UNDEFINED: u16 = 0x3000;

/// The `> 4 GiB` saturation sentinel for `ObjectInfo.compressed_size`.
const COMPRESSED_SIZE_SENTINEL: u32 = 0xFFFF_FFFF;

/// Preprocess callback, invoked once per local file during the counting pass.
/// The local file's metadata and its full path are handed over; keel-ffi derives
/// name/size/mtime for the preprocess payload from these.
pub type LocalPreprocessCb<'a> = &'a mut dyn FnMut(&Metadata, &str) -> Result<(), VfsError>;

/// Progress callback, invoked on every per-chunk tick.
pub type ProgressCb<'a> = &'a mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>;

/// Transfer files from the local disk to the device.
///
/// Returns `(destination_object_id, bulk_files_sent, bulk_size_sent)`.
///
/// On failure this returns a plain `Err` and drops the partial counts — every
/// caller discards them on error anyway, so nothing observable is lost.
#[allow(clippy::too_many_arguments)]
pub fn upload_files<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    sources: &[String],
    destination: &str,
    preprocess_files: bool,
    preprocess_cb: LocalPreprocessCb<'_>,
    progress_cb: ProgressCb<'_>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(u32, i64, i64), VfsError> {
    let _destination = fix_slash(destination);

    let mut pinfo = ProgressInfo::new();

    // Left at 0 unless preprocessing runs. Zeroed, they make every bulk
    // percentage `percent(x, 0) == 0`.
    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;
    let mut total_size: i64 = 0;

    // Running totals returned to the caller.
    let mut bulk_files_sent: i64 = 0;
    let mut bulk_size_sent: i64 = 0;

    // --- optional preprocess pass ---
    if preprocess_files {
        let (tf, td, ts) = walk_local_files(sources, &mut |meta: &Metadata, full_path: &str| {
            // Skip directories before the user callback.
            if meta.is_dir() {
                return Ok(());
            }
            // Cancel is checked per preprocessed file.
            if should_cancel() {
                return Err(VfsError::Cancelled);
            }
            preprocess_cb(meta, full_path)?;
            Ok(())
        })?;
        total_files = tf;
        total_directories = td;
        total_size = ts;
    }

    // Create the destination dir; a failure returns raw (unclassified).
    let dest_parent_id = make_directory(session, storage_id, &_destination)?;

    pinfo.total_files = total_files;
    pinfo.total_directories = total_directories;
    pinfo.bulk_file_size.total = total_size;

    // --- per-source local walk ---
    for source in sources {
        let _source = fix_slash(source);
        let source_parent_path = go_filepath_dir(&_source);

        // The per-source memo dict, seeded with the destination root.
        let mut dict: HashMap<String, u32> = HashMap::new();
        dict.insert(_destination.clone(), dest_parent_id);

        let walk_result = walk_local(Path::new(&_source), &mut |path: &Path,
                                                                meta: &Metadata|
         -> Result<(), VfsError> {
            process_upload_entry(
                session,
                storage_id,
                path,
                meta,
                &source_parent_path,
                &_destination,
                &mut dict,
                &mut pinfo,
                &mut bulk_files_sent,
                &mut bulk_size_sent,
                total_files,
                total_size,
                should_cancel,
                progress_cb,
            )
        });

        // Error classification, with a Cancelled short-circuit in front.
        if let Err(we) = walk_result {
            return Err(match we {
                WalkLocalError::Cb(VfsError::Cancelled) => VfsError::Cancelled,
                // An `InvalidPath` (the file-open failure path) passes through as-is.
                WalkLocalError::Cb(e @ VfsError::InvalidPath(_)) => e,
                // Everything else the callback returned becomes a FileTransfer error.
                WalkLocalError::Cb(e) => {
                    VfsError::FileTransfer(format!("an error occured while uploading files. {e}"))
                }
                // A raw local-fs error from the walk machinery (metadata/read-dir),
                // classified by kind.
                WalkLocalError::Io(e) => classify_upload_patherror(e),
            });
        }
    }

    // --- final Completed tick ---
    pinfo.status = TransferStatus::Completed;
    // The Completed tick is a progress tick, so cancel is honoured here too.
    if should_cancel() {
        return Err(VfsError::Cancelled);
    }
    progress_cb(&pinfo)?;

    Ok((dest_parent_id, bulk_files_sent, bulk_size_sent))
}

/// Handle one entry of the per-source walk. Pulled into a free function so the
/// per-chunk progress closure it builds is not nested inside another closure
/// (which would tangle the borrow checker).
#[allow(clippy::too_many_arguments)]
fn process_upload_entry<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    path: &Path,
    meta: &Metadata,
    source_parent_path: &str,
    destination: &str,
    dict: &mut HashMap<String, u32>,
    pinfo: &mut ProgressInfo,
    bulk_files_sent: &mut i64,
    bulk_size_sent: &mut i64,
    total_files: i64,
    total_size: i64,
    should_cancel: &dyn Fn() -> bool,
    progress_cb: ProgressCb<'_>,
) -> Result<(), VfsError> {
    let name = file_name_str(path);

    // Never follow symlinks. The symlink metadata reports the link itself, so this
    // also stops us descending into a symlinked directory.
    if is_symlink_local(meta) {
        return Ok(());
    }
    // Filter disallowed files (`.DS_Store`, test sentinel).
    if is_disallowed_files(&name) {
        return Ok(());
    }

    let source_file_path = fix_slash(&path.to_string_lossy());
    let (destination_parent_path, destination_file_path) =
        map_source_path_to_destination_path(&source_file_path, source_parent_path, destination);

    let size = meta.len() as i64;
    let is_dir = meta.is_dir();

    // --- directory ---
    if is_dir {
        // Create the child dir on the device and memoise its object id under the
        // destination path, so files placed inside it resolve their parent from
        // the cache.
        let obj_id = make_directory(session, storage_id, &destination_file_path)?;
        dict.insert(destination_file_path, obj_id);
        return Ok(());
    }

    // --- file ---
    // Resolve the parent object id from the memo, creating the parent dir on a miss.
    let file_parent_id: u32 = match dict.get(&destination_parent_path) {
        Some(&pid) => pid,
        None => {
            // Miss: create the parent dir and memoise it under its own path so
            // sibling files reuse it. (A pre-order walk memoises the parent first,
            // so this rarely fires.)
            let obj_id = make_directory(session, storage_id, &destination_parent_path)?;
            dict.insert(destination_parent_path.clone(), obj_id);
            obj_id
        }
    };

    // Open the local file. A failure is an InvalidPath, which the caller passes
    // straight through.
    let mut file_buf =
        File::open(&source_file_path).map_err(|e| VfsError::InvalidPath(e.to_string()))?;

    // CompressedSize saturates for > 4 GiB.
    let compressed_size = if size > 0xFFFF_FFFF {
        COMPRESSED_SIZE_SENTINEL
    } else {
        size as u32
    };

    // Local mtime. `modified()` is Ok on the supported platforms; a failure
    // degrades to the epoch rather than panicking.
    let mod_time = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    let f_obj = ObjectInfo {
        storage_id,
        object_format: OFC_UNDEFINED,
        parent_object: file_parent_id,
        filename: name.clone(),
        compressed_size,
        modification_date: Some(mod_time),
        ..Default::default()
    };

    // bulk_files_sent is bumped BEFORE the transfer — load-bearing for the
    // files_sent off-by-one.
    *bulk_files_sent += 1;

    // Snapshot the active file into pinfo. object_id is filled in once
    // send_object_info returns.
    pinfo.file_info = FileInfo {
        info: f_obj.clone(),
        size,
        is_dir,
        mod_time: Some(mod_time), // FileInfo.mod_time is Option<SystemTime>
        name: f_obj.filename.clone(),
        full_path: destination_file_path.clone(),
        parent_path: destination_parent_path.clone(),
        extension: extension(&f_obj.filename, is_dir),
        parent_id: f_obj.parent_object,
        object_id: 0,
    };
    pinfo.latest_sent_time = SystemTime::now();

    // The per-chunk progress closure is scoped tightly so its `&mut pinfo` borrow
    // is released before we touch pinfo again below.
    let obj_id = {
        let mut prev_sent_size: i64 = 0;
        let mut size_progress = |total: i64, sent: i64, obj_id: u32| -> Result<(), VfsError> {
            pinfo.file_info.object_id = obj_id;
            pinfo.active_file_size.total = total;
            pinfo.active_file_size.sent = sent;
            pinfo.active_file_size.progress = percent(sent as f32, total as f32);

            let chunk_size = sent - prev_sent_size;
            *bulk_size_sent += chunk_size;

            pinfo.bulk_file_size.sent = *bulk_size_sent;
            pinfo.bulk_file_size.progress = percent(*bulk_size_sent as f32, total_size as f32);

            pinfo.speed = transfer_rate(chunk_size, pinfo.latest_sent_time);
            progress_cb(pinfo)?;

            pinfo.latest_sent_time = SystemTime::now();
            prev_sent_size = sent;
            Ok(())
        };

        handle_make_file(
            session,
            storage_id,
            &f_obj,
            &mut file_buf,
            size,
            true, // overwrite_existing
            should_cancel,
            &mut size_progress,
        )?
    };

    // Update the post-transfer bookkeeping.
    pinfo.files_sent = *bulk_files_sent;
    pinfo.files_sent_progress = percent(*bulk_files_sent as f32, total_files as f32);
    pinfo.file_info.object_id = obj_id;
    dict.insert(destination_file_path, obj_id);

    Ok(())
}

/// A per-chunk progress sink for the device transfer:
/// `FnMut(total, sent, object_id) -> Result<(), VfsError>`.
type SizeProgressCb<'a> = &'a mut dyn FnMut(i64, i64, u32) -> Result<(), VfsError>;

/// Create the device file.
///
/// Existence check (Unicode-case-insensitive, in
/// `get_object_from_parent_id_and_filename`) → overwrite by delete-then-recreate
/// → `SendObjectInfo` → streamed `SendObject`.
#[allow(clippy::too_many_arguments)]
fn handle_make_file<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    obj: &ObjectInfo,
    file_buf: &mut File,
    size: i64,
    overwrite_existing: bool,
    should_cancel: &dyn Fn() -> bool,
    size_progress_cb: SizeProgressCb<'_>,
) -> Result<u32, VfsError> {
    // Does the file already exist under this parent?
    match get_object_from_parent_id_and_filename(
        session,
        storage_id,
        obj.parent_object,
        &obj.filename,
    ) {
        Ok(fi) => {
            if !overwrite_existing {
                // Reuse the existing handle.
                return Ok(fi.object_id);
            }
            // Overwrite: delete the existing object first, going through the full
            // delete_file path (which re-checks existence and tolerates RC 0x2009
            // as "already gone").
            let fp = FileProp {
                object_id: fi.object_id,
                full_path: String::new(),
            };
            delete_file(session, storage_id, std::slice::from_ref(&fp))?;
        }
        // Not found: nothing to delete, fall through to create. Any other error
        // propagates.
        Err(VfsError::FileNotFound(_)) => {}
        Err(e) => return Err(e),
    }

    // SendObjectInfo reserves the handle.
    let obj_id = session
        .send_object_info(storage_id, obj.parent_object, obj)
        .map_err(VfsError::SendObject)?;

    // Stream the bytes via SendObject.
    //
    // The device transfer callback must return `MtpError`, but our progress path
    // produces `VfsError` (and cancellation). We smuggle both out via a flag + an
    // out-slot and stop the transfer with a benign, non-poisoning
    // `MtpError::Closed` sentinel that we always intercept before it can escape.
    let cancelled = Cell::new(false);
    let mut inner_err: Option<VfsError> = None;

    let res = {
        let mut prog = |sent: u64, _handle: u32| -> Result<(), MtpError> {
            // Cancel is checked once per progress tick.
            if should_cancel() {
                cancelled.set(true);
                return Err(MtpError::Closed);
            }
            match size_progress_cb(size, sent as i64, obj_id) {
                Ok(()) => Ok(()),
                Err(e) => {
                    inner_err = Some(e);
                    Err(MtpError::Closed)
                }
            }
        };
        session.send_object(file_buf, size as u64, &mut prog)
    };

    if cancelled.get() {
        return Err(VfsError::Cancelled);
    }
    if let Some(e) = inner_err {
        // A genuine progress-callback error (never a cancel); return it and let
        // the caller classify it. In practice keel-ffi's progress callback never
        // errors, so this is dead.
        return Err(e);
    }
    // A real SendObject failure becomes a SendObject error.
    session_send_object_result(res)?;

    Ok(obj_id)
}

/// Map a `SendObject` result into the vfs error taxonomy.
#[inline]
fn session_send_object_result(res: Result<(), MtpError>) -> Result<(), VfsError> {
    res.map_err(VfsError::SendObject)
}

// ---------------------------------------------------------------------------
// Local-filesystem walk — used by both the per-source upload loop and
// `walk_local_files`.
//
// Semantics: pre-order (a directory is visited before its children), children in
// sorted-by-name order, symlink-aware (symlinks reported as links, never
// followed), and errors from metadata/read-dir surfaced to the caller. Callbacks
// never request a skip, so no skip-dir/skip-all sentinel is modelled.
// ---------------------------------------------------------------------------

/// The two error origins the upload classifier distinguishes: a value the walk
/// callback returned ([`Self::Cb`], a `VfsError`) versus a raw local-fs error from
/// the traversal itself ([`Self::Io`]).
enum WalkLocalError {
    Cb(VfsError),
    Io(std::io::Error),
}

fn walk_local(
    root: &Path,
    cb: &mut dyn FnMut(&Path, &Metadata) -> Result<(), VfsError>,
) -> Result<(), WalkLocalError> {
    // Read the root's symlink metadata; a failure surfaces directly to the caller.
    match std::fs::symlink_metadata(root) {
        Ok(meta) => walk_local_inner(root, &meta, cb),
        Err(e) => Err(WalkLocalError::Io(e)),
    }
}

fn walk_local_inner(
    path: &Path,
    info: &Metadata,
    cb: &mut dyn FnMut(&Path, &Metadata) -> Result<(), VfsError>,
) -> Result<(), WalkLocalError> {
    // A non-directory is a single callback, then return.
    if !info.is_dir() {
        return cb(path, info).map_err(WalkLocalError::Cb);
    }

    // Read + sort children first. A ReadDir failure aborts the directory.
    let names = match read_dir_names(path) {
        Ok(n) => n,
        Err(e) => return Err(WalkLocalError::Io(e)),
    };

    // Then visit the directory itself (pre-order).
    cb(path, info).map_err(WalkLocalError::Cb)?;

    for name in names {
        let child = path.join(&name);
        let child_meta = match std::fs::symlink_metadata(&child) {
            Ok(m) => m,
            Err(e) => return Err(WalkLocalError::Io(e)),
        };
        walk_local_inner(&child, &child_meta, cb)?;
    }
    Ok(())
}

/// Directory entry names, sorted. Sorting by `OsString` gives byte-order, which
/// matches the string sort for ASCII names in practice.
fn read_dir_names(path: &Path) -> std::io::Result<Vec<OsString>> {
    let mut names: Vec<OsString> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}

/// Count files/dirs/bytes over the sources, invoking `cb` for every non-symlink,
/// non-disallowed entry. Error classification: a raw local-fs error → permission
/// ⇒ `FilePermission`, else `LocalFile`; anything the callback returned (incl.
/// `Cancelled`) → passed through raw.
fn walk_local_files(
    sources: &[String],
    cb: &mut dyn FnMut(&Metadata, &str) -> Result<(), VfsError>,
) -> Result<(i64, i64, i64), VfsError> {
    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;
    let mut total_size: i64 = 0;

    for source in sources {
        let result = walk_local(Path::new(source), &mut |path: &Path,
                                                         meta: &Metadata|
         -> Result<(), VfsError> {
            let name = file_name_str(path);
            // Skip symlinks and disallowed files.
            if is_symlink_local(meta) {
                return Ok(());
            }
            if is_disallowed_files(&name) {
                return Ok(());
            }
            cb(meta, &path.to_string_lossy())?;
            if !meta.is_dir() {
                total_files += 1;
                total_size += meta.len() as i64;
            } else {
                total_directories += 1;
            }
            Ok(())
        });

        if let Err(we) = result {
            return Err(match we {
                // A raw local-fs error: permission ⇒ FilePermission, else
                // LocalFile. (Unlike the main upload classifier, this one does NOT
                // special-case NotExist.)
                WalkLocalError::Io(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        VfsError::FilePermission(e.to_string())
                    } else {
                        VfsError::LocalFile(e.to_string())
                    }
                }
                // Return the callback error raw (this is where a Cancelled from the
                // preprocess cb survives).
                WalkLocalError::Cb(e) => e,
            });
        }
    }

    Ok((total_files, total_directories, total_size))
}

/// True if the entry is a symlink.
fn is_symlink_local(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// The base name of a path.
fn file_name_str(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Strip the source parent prefix off the source path, join the remainder under
/// the destination, and split back into `(parent, file)`. Shared by upload and
/// download (see the note above); uses `path::get_full_path` for the join+clean.
fn map_source_path_to_destination_path(
    source_path: &str,
    source_parent_path: &str,
    destination_path: &str,
) -> (String, String) {
    // Remove the prefix if present, else leave unchanged.
    let trimmed = source_path
        .strip_prefix(source_parent_path)
        .unwrap_or(source_path);
    let full_path = get_full_path(destination_path, trimmed);
    (go_filepath_dir(&full_path), full_path)
}

/// The parent directory of an already-`fix_slash`'d (clean, absolute) path — used
/// for `source_parent_path` and inside [`map_source_path_to_destination_path`].
/// Inputs are guaranteed clean (they come out of [`fix_slash`] /
/// [`get_full_path`]), so this collapses to "strip the last component".
fn go_filepath_dir(p: &str) -> String {
    match p.rfind('/') {
        None => ".".to_string(),       // no separator ⇒ "."
        Some(0) => "/".to_string(),    // e.g. "/a" ⇒ "/"
        Some(i) => p[..i].to_string(), // e.g. "/a/b/c" ⇒ "/a/b"
    }
}

/// Classify a raw local-fs error from the upload walk: permission ⇒
/// `FilePermission`, NotExist ⇒ `InvalidPath`, else `LocalFile`.
fn classify_upload_patherror(e: std::io::Error) -> VfsError {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => VfsError::FilePermission(e.to_string()),
        std::io::ErrorKind::NotFound => VfsError::InvalidPath(e.to_string()),
        _ => VfsError::LocalFile(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_dir_matches_filepath_dir() {
        assert_eq!(go_filepath_dir("/a/b/c"), "/a/b");
        assert_eq!(go_filepath_dir("/a"), "/");
        assert_eq!(go_filepath_dir("/"), "/");
        assert_eq!(go_filepath_dir("/mock_dir1"), "/");
        assert_eq!(go_filepath_dir("noslash"), ".");
    }

    #[test]
    fn map_source_to_destination() {
        // Upload: /Users/x/mock_dir1/1/a.txt under parent /Users/x → dest/mock_dir1/1/a.txt
        let (parent, file) = map_source_path_to_destination_path(
            "/Users/x/mock_dir1/1/a.txt",
            "/Users/x",
            "/mtp-dest",
        );
        assert_eq!(file, "/mtp-dest/mock_dir1/1/a.txt");
        assert_eq!(parent, "/mtp-dest/mock_dir1/1");
    }

    #[test]
    fn compressed_size_saturates_past_4gib() {
        let big: i64 = 0x1_0000_0000; // 4 GiB
        let cs = if big > 0xFFFF_FFFF {
            COMPRESSED_SIZE_SENTINEL
        } else {
            big as u32
        };
        assert_eq!(cs, 0xFFFF_FFFF);

        let small: i64 = 35;
        let cs2 = if small > 0xFFFF_FFFF {
            COMPRESSED_SIZE_SENTINEL
        } else {
            small as u32
        };
        assert_eq!(cs2, 35);
    }

    #[test]
    fn undefined_format_constant() {
        assert_eq!(OFC_UNDEFINED, 0x3000);
    }
}

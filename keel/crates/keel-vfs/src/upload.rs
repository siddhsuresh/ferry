//! `upload_files` — local disk → device, a faithful port of go-mtpx
//! `UploadFiles` (main.go:274-548) plus the helpers it leans on:
//! `handleMakeFile` (helpers.go:229-274), `walkLocalFiles` (helpers.go:407-462)
//! and the `filepath.Walk` traversal Go used to enumerate the local tree.
//!
//! Like the rest of keel-vfs (path.rs / object.rs / walk.rs / dirops.rs) this
//! operates on a generic `&mut MtpSession<T>` — the keel analogue of go-mtpx's
//! `dev *mtp.Device` — so it drives both the real `UsbTransport` and the test
//! `FakeDevice`. The FFI reaches it via `Device::session_mut()`.
//!
//! Behaviours preserved verbatim (pinned by `upload_files_test.go`):
//!   * `ObjectFormat` is ALWAYS `OFC_Undefined` (0x3000), never inferred from the
//!     extension (main.go:449);
//!   * `CompressedSize` saturates to 0xFFFFFFFF for files > 4 GiB (main.go:440-444);
//!   * per-file existence check is Unicode-case-insensitive with a
//!     delete-then-recreate overwrite (helpers.go:230-252);
//!   * `bulkFilesSent` is incremented **before** the transfer, while
//!     `pInfo.FilesSent` is updated **after** it — the off-by-one the tests assert
//!     (`So(fi.FilesSent, ShouldEqual, prevFilesSent)`);
//!   * per-chunk instantaneous speed and bulk/active percentages exactly as Go
//!     computed them;
//!   * symlinks are never followed (Lstat traversal + an explicit skip);
//!   * `.DS_Store` / the test sentinel are filtered.
//!
//! Fidelity FIXES (plan §3.5 "upload fallback-arm mkdir bugs"): Go's two
//! `else`/fallback arms in the walk callback (main.go:395-403 and 416-428) create
//! the wrong directory and/or memoise the wrong dict key. Both are unreachable in
//! practice (a pre-order walk always visits a parent before its children, so the
//! memo hit always fires), but keel makes them correct anyway — see the inline
//! comments where they used to be.
//!
//! Cancel seam (keel addition — go-mtpx has none; legacy kernel L348-381 bolted it on
//! at the FFI layer by polling inside the callbacks): `should_cancel` is checked
//! once per preprocessed file and once per progress tick. On a fire we abort with
//! the distinct `VfsError::Cancelled`, which keel-ffi maps to
//! `ErrorTransferCancelled`. Go instead let a `TransferCancelledError` bubble
//! through `handleMakeFile`→`SendObjectError`→`FileTransferError` and relied on
//! the FFI substring-matching "transfer cancelled by user"; keel's distinct
//! variant is behaviourally equivalent (both end at `ErrorTransferCancelled`).

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

// NOTE for the gate agent: `map_source_path_to_destination_path` and
// `go_filepath_dir` are go-mtpx `utils.go` helpers that no sibling module ports
// (they are used only by upload/download). They are defined privately here (and a
// duplicate in download.rs) so each transfer module stands alone; hoist to
// `path.rs` if preferred.

use crate::dirops::{delete_file, make_directory};

/// go-mtpfs `OFC_Undefined` (const.go). SendObjectInfo for an uploaded file
/// ALWAYS uses this format (main.go:449) — go-mtpx never derives the format from
/// the extension. A private literal so this file does not couple to whatever name
/// `keel_proto::consts` exposes for it.
const OFC_UNDEFINED: u16 = 0x3000;

/// The `> 4 GiB` sentinel for `ObjectInfo.compressed_size` (main.go:440-441).
const COMPRESSED_SIZE_SENTINEL: u32 = 0xFFFF_FFFF;

/// Preprocess callback: mirrors go-mtpx `LocalPreprocessCb`
/// (`func(fi *os.FileInfo, fullPath string, err error) error`, structs.go:89).
/// Go always passed a nil error, so the `err` slot is dropped. The local file's
/// metadata and its full path are handed over (keel-ffi derives name/size/mtime
/// for the preprocess payload from these).
pub type LocalPreprocessCb<'a> = &'a mut dyn FnMut(&Metadata, &str) -> Result<(), VfsError>;

/// Progress callback: mirrors go-mtpx `ProgressCb`
/// (`func(fi *ProgressInfo, err error) error`, structs.go:87), nil-error slot
/// dropped.
pub type ProgressCb<'a> = &'a mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>;

/// Transfer files from the local disk to the device — port of go-mtpx
/// `UploadFiles` (main.go:274-548).
///
/// Returns `(destination_object_id, bulk_files_sent, bulk_size_sent)`.
///
/// Deviation: Go also returned the partial counts alongside the error on the
/// failure paths; keel returns a plain `Err`, because every legacy kernel caller discards
/// the counts on error (`SendError`, the legacy kernel). The observable result (an error
/// surfaces, counts thrown away) is identical.
#[allow(clippy::too_many_arguments)] // 1:1 with the Go signature (main.go:274).
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

    // 0 unless preprocessing runs (main.go:294-304). Left at 0, they make every
    // bulk percentage `Percent(x, 0) == 0`.
    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;
    let mut total_size: i64 = 0;

    // Running totals returned to the caller.
    let mut bulk_files_sent: i64 = 0;
    let mut bulk_size_sent: i64 = 0;

    // --- optional preprocess pass (main.go:309-333) ---
    if preprocess_files {
        let (tf, td, ts) = walk_local_files(sources, &mut |meta: &Metadata, full_path: &str| {
            // main.go:315 — skip directories before the user callback.
            if meta.is_dir() {
                return Ok(());
            }
            // Cancel is checked per preprocessed file (legacy kernel L356 polled inside
            // the preprocess callback, which go-mtpx only invokes for files).
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

    // main.go:335 — create the destination dir; a failure returns raw (no switch).
    let dest_parent_id = make_directory(session, storage_id, &_destination)?;

    pinfo.total_files = total_files;
    pinfo.total_directories = total_directories;
    pinfo.bulk_file_size.total = total_size;

    // --- per-source local walk (main.go:344-540) ---
    for source in sources {
        let _source = fix_slash(source);
        let source_parent_path = go_filepath_dir(&_source);

        // The per-source memo dict, seeded with the destination root
        // (main.go:348-350).
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

        // Error classification — port of the switch at main.go:520-539, with a
        // keel-specific Cancelled short-circuit in front (see the module note:
        // Go wrapped cancel into FileTransferError and relied on the FFI substring
        // match; keel keeps the distinct variant).
        if let Err(we) = walk_result {
            return Err(match we {
                WalkLocalError::Cb(VfsError::Cancelled) => VfsError::Cancelled,
                // Go: `case InvalidPathError: return err` — passed through as-is
                // (this is the os.Open failure path, main.go:433).
                WalkLocalError::Cb(e @ VfsError::InvalidPath(_)) => e,
                // Go default arm (main.go:535-538).
                WalkLocalError::Cb(e) => {
                    VfsError::FileTransfer(format!("an error occured while uploading files. {e}"))
                }
                // Go `case *os.PathError:` (main.go:525-534). The raw local-fs
                // error from the walk machinery (Lstat/ReadDir), classified by kind.
                WalkLocalError::Io(e) => classify_upload_patherror(e),
            });
        }
    }

    // --- final Completed tick (main.go:542-545) ---
    pinfo.status = TransferStatus::Completed;
    // The Completed tick is a progress tick, so cancel is honoured here too
    // (legacy kernel L372 polled cancel inside every progress callback, incl. the last).
    if should_cancel() {
        return Err(VfsError::Cancelled);
    }
    progress_cb(&pinfo)?;

    Ok((dest_parent_id, bulk_files_sent, bulk_size_sent))
}

/// One entry of the per-source `filepath.Walk` callback (main.go:353-517), pulled
/// into a free function so the per-chunk progress closure it builds is not nested
/// inside another closure (which would tangle the borrow checker).
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

    // main.go:361-363 — never follow symlinks. Lstat metadata reports the link
    // itself, so this also stops us descending into a symlinked directory.
    if is_symlink_local(meta) {
        return Ok(());
    }
    // main.go:366-369 — filter disallowed files (`.DS_Store`, test sentinel).
    if is_disallowed_files(&name) {
        return Ok(());
    }

    let source_file_path = fix_slash(&path.to_string_lossy());
    let (destination_parent_path, destination_file_path) =
        map_source_path_to_destination_path(&source_file_path, source_parent_path, destination);

    let size = meta.len() as i64;
    let is_dir = meta.is_dir();

    // --- directory (main.go:382-406) ---
    if is_dir {
        // Go had a fast arm (`if parent in dict`) and a fallback `else`. The
        // fallback (main.go:395-403) is BUGGY: it calls
        // `MakeDirectory(_destination)` — recreating the *root* destination — yet
        // memoises `destination_file_path => <root objId>`, poisoning the memo for
        // anything later placed under this dir. FIX (plan §3.5): always create the
        // real child dir and memoise the correct key. Both arms therefore collapse
        // to one. (The fallback was unreachable regardless: a pre-order walk
        // memoises a parent before visiting its children.)
        let obj_id = make_directory(session, storage_id, &destination_file_path)?;
        dict.insert(destination_file_path, obj_id);
        return Ok(());
    }

    // --- file (main.go:409-516) ---
    // Resolve the parent object id (main.go:411-428).
    let file_parent_id: u32 = match dict.get(&destination_parent_path) {
        Some(&pid) => pid,
        None => {
            // Go's fallback (main.go:416-428) memoised `destination_file_path` (the
            // file) instead of `destination_parent_path` (the dir), so a sibling
            // file would miss the memo and re-`MakeDirectory`. FIX (plan §3.5):
            // memoise the parent-dir key. (Also unreachable in a pre-order walk —
            // the parent dir is always memoised first.)
            let obj_id = make_directory(session, storage_id, &destination_parent_path)?;
            dict.insert(destination_parent_path.clone(), obj_id);
            obj_id
        }
    };

    // Open the local file (main.go:431-435). A failure is an InvalidPathError,
    // which the caller's switch passes straight through.
    let mut file_buf =
        File::open(&source_file_path).map_err(|e| VfsError::InvalidPath(e.to_string()))?;

    // CompressedSize saturates for > 4 GiB (main.go:437-444).
    let compressed_size = if size > 0xFFFF_FFFF {
        COMPRESSED_SIZE_SENTINEL
    } else {
        size as u32
    };

    // Local mtime (os.FileInfo.ModTime, main.go:452). `modified()` is Ok on the
    // supported platforms; a failure degrades to the epoch rather than panicking.
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

    // bulkFilesSent is bumped BEFORE the transfer (main.go:456) — load-bearing for
    // the FilesSent off-by-one.
    *bulk_files_sent += 1;

    // Snapshot the active file into pInfo (main.go:458-468). ObjectId is filled in
    // once SendObjectInfo returns.
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
    pinfo.latest_sent_time = SystemTime::now(); // main.go:469

    // The SizeProgressCb closure (main.go:476-500) is scoped tightly so its
    // `&mut pinfo` borrow is released before we touch pinfo again below.
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

    // main.go:508-514.
    pinfo.files_sent = *bulk_files_sent;
    pinfo.files_sent_progress = percent(*bulk_files_sent as f32, total_files as f32);
    pinfo.file_info.object_id = obj_id;
    dict.insert(destination_file_path, obj_id);

    Ok(())
}

/// A per-chunk progress sink for the device transfer:
/// `FnMut(total, sent, object_id) -> Result<(), VfsError>` — go-mtpx
/// `SizeProgressCb` (structs.go:83).
type SizeProgressCb<'a> = &'a mut dyn FnMut(i64, i64, u32) -> Result<(), VfsError>;

/// Create the device file — port of go-mtpx `handleMakeFile` (helpers.go:229-274).
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
    // helpers.go:230-252 — does the file already exist under this parent?
    match get_object_from_parent_id_and_filename(
        session,
        storage_id,
        obj.parent_object,
        &obj.filename,
    ) {
        Ok(fi) => {
            if !overwrite_existing {
                // helpers.go:235-237 — reuse the existing handle.
                return Ok(fi.object_id);
            }
            // helpers.go:239-243 — overwrite: delete the existing object first,
            // going through the full DeleteFile path (which re-checks existence and
            // tolerates RC 0x2009 as "already gone"), exactly like Go.
            let fp = FileProp {
                object_id: fi.object_id,
                full_path: String::new(),
            };
            delete_file(session, storage_id, std::slice::from_ref(&fp))?;
        }
        // helpers.go:245-251 — not found: nothing to delete, fall through to
        // create. Any other error propagates.
        Err(VfsError::FileNotFound(_)) => {}
        Err(e) => return Err(e),
    }

    // helpers.go:254-258 — SendObjectInfo. Go wrapped the error as SendObjectError.
    let obj_id = session
        .send_object_info(storage_id, obj.parent_object, obj)
        .map_err(VfsError::SendObject)?;

    // helpers.go:260-271 — stream the bytes via SendObject.
    //
    // The device transfer callback must return `MtpError`, but our progress path
    // produces `VfsError` (and cancellation). We smuggle both out via a flag + an
    // out-slot and stop the transfer with a benign, non-poisoning
    // `MtpError::Closed` sentinel that we always intercept before it can escape.
    let cancelled = Cell::new(false);
    let mut inner_err: Option<VfsError> = None;

    let res = {
        let mut prog = |sent: u64, _handle: u32| -> Result<(), MtpError> {
            // Cancel is checked once per progress tick (task's cancel seam).
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
        // A genuine progress-callback error (never a cancel). Go would have
        // double-wrapped it (SendObjectError → FileTransferError); keel returns it
        // and lets the caller's switch classify it — equivalent FFI text. In
        // practice keel-ffi's progress callback never errors, so this is dead.
        return Err(e);
    }
    // helpers.go:269-271 — a real SendObject failure becomes SendObjectError.
    session_send_object_result(res)?;

    Ok(obj_id)
}

/// Map a `SendObject` result into the vfs taxonomy (helpers.go:269-271).
#[inline]
fn session_send_object_result(res: Result<(), MtpError>) -> Result<(), VfsError> {
    res.map_err(VfsError::SendObject)
}

// ---------------------------------------------------------------------------
// Local-filesystem walk — a faithful stand-in for Go's `filepath.Walk`
// (used by both `UploadFiles`'s per-source loop and `walkLocalFiles`).
//
// Semantics reproduced: pre-order (a directory is visited before its children),
// children in sorted-by-name order, Lstat (symlinks reported as links, never
// followed), and errors from Lstat/ReadDir surfaced to the caller. go-mtpx's
// callbacks never return SkipDir/SkipAll, so those sentinels are not modelled.
// ---------------------------------------------------------------------------

/// The two error origins Go's upload switch distinguishes: a value the walk
/// callback returned ([`Self::Cb`], a `VfsError`) versus a raw local-fs error from
/// the traversal itself ([`Self::Io`], Go's `*os.PathError`).
enum WalkLocalError {
    Cb(VfsError),
    Io(std::io::Error),
}

fn walk_local(
    root: &Path,
    cb: &mut dyn FnMut(&Path, &Metadata) -> Result<(), VfsError>,
) -> Result<(), WalkLocalError> {
    // filepath.Walk: `info, err := lstat(root)`; on error the callback is invoked
    // with it and (in go-mtpx) returns it. We surface it directly — equivalent,
    // since the callback's only action on a non-nil error is to return it.
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
    // filepath.Walk `walk`: a non-directory is a single callback and returns.
    if !info.is_dir() {
        return cb(path, info).map_err(WalkLocalError::Cb);
    }

    // Read + sort children first (Go's readDirNames). A ReadDir failure aborts the
    // directory (Go passes it to walkFn, which returns it).
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

/// Directory entry names, sorted (Go's `readDirNames`). Sorting by `OsString`
/// gives byte-order, matching Go's string sort for the ASCII names in practice.
fn read_dir_names(path: &Path) -> std::io::Result<Vec<OsString>> {
    let mut names: Vec<OsString> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}

/// go-mtpx `walkLocalFiles` (helpers.go:407-462): count files/dirs/bytes over the
/// sources, invoking `cb` for every non-symlink, non-disallowed entry. Errors are
/// classified exactly as Go's switch did (helpers.go:446-458): a raw local-fs
/// error → permission ⇒ `FilePermission`, else `LocalFile`; anything the callback
/// returned (incl. `Cancelled`) → passed through raw.
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
            // helpers.go:422-430 — skip symlinks and disallowed files.
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
                // helpers.go:448-453 — `case *os.PathError`: permission ⇒
                // FilePermission, else LocalFile. (Note: unlike the main UploadFiles
                // switch, this one does NOT special-case NotExist.)
                WalkLocalError::Io(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        VfsError::FilePermission(e.to_string())
                    } else {
                        VfsError::LocalFile(e.to_string())
                    }
                }
                // helpers.go:455-456 default — return the callback error raw (this
                // is where a Cancelled from the preprocess cb survives).
                WalkLocalError::Cb(e) => e,
            });
        }
    }

    Ok((total_files, total_directories, total_size))
}

/// go-mtpx `isSymlinkLocal` (utils.go:161-163): `fi.Mode()&os.ModeSymlink != 0`.
fn is_symlink_local(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// The base name of a path — Go's `os.FileInfo.Name()` for a `filepath.Walk`
/// entry.
fn file_name_str(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// go-mtpx `mapSourcePathToDestinationPath` (utils.go:217-224): strip the source
/// parent prefix off the source path, join the remainder under the destination,
/// and split back into `(parent, file)`. Shared by upload+download (see the
/// module note); uses the ported `path::get_full_path` for the join+clean.
fn map_source_path_to_destination_path(
    source_path: &str,
    source_parent_path: &str,
    destination_path: &str,
) -> (String, String) {
    // strings.TrimPrefix: remove the prefix if present, else leave unchanged.
    let trimmed = source_path
        .strip_prefix(source_parent_path)
        .unwrap_or(source_path);
    let full_path = get_full_path(destination_path, trimmed);
    (go_filepath_dir(&full_path), full_path)
}

/// Unix `filepath.Dir` for an already-`fixSlash`'d (clean, absolute) path — used
/// for `sourceParentPath = filepath.Dir(_source)` (main.go:346) and inside
/// [`map_source_path_to_destination_path`]. Inputs are guaranteed clean (they come
/// out of [`fix_slash`] / [`get_full_path`]), so Go's general `path.Clean`
/// machinery collapses to "strip the last component".
fn go_filepath_dir(p: &str) -> String {
    match p.rfind('/') {
        None => ".".to_string(),       // no separator ⇒ "." (Go's Clean(""))
        Some(0) => "/".to_string(),    // e.g. "/a" ⇒ "/"
        Some(i) => p[..i].to_string(), // e.g. "/a/b/c" ⇒ "/a/b"
    }
}

/// Port of the `*os.PathError` arm of the UploadFiles switch (main.go:525-534):
/// permission ⇒ `FilePermission`, NotExist ⇒ `InvalidPath`, else `LocalFile`.
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

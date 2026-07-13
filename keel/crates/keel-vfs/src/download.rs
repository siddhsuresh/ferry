//! `download_files` — device → local disk, a faithful port of go-mtpx
//! `DownloadFiles` (main.go:556-700) plus its helpers `processDownloadFiles`
//! (helpers.go:464-535), `processDownloadFilesError` (helpers.go:537-560),
//! `handleMakeLocalFile` (helpers.go:277-306), `makeLocalDirectory`
//! (helpers.go:387-404) and `restoreLocalFileTimestamp` (helpers.go:563-572).
//!
//! Operates on a generic `&mut MtpSession<T>` like the rest of keel-vfs (the
//! keel analogue of go-mtpx's `dev *mtp.Device`); the FFI reaches it via
//! `Device::session_mut()`.
//!
//! Behaviours preserved:
//!   * each object is fetched whole via GetObject and streamed straight to
//!     `File::create` — a **silent** local overwrite (helpers.go:278);
//!   * the device modification time is restored onto the local file after the
//!     write (helpers.go:305);
//!   * local parent directories are created on demand (helpers.go:484-491);
//!   * `.DS_Store` / the test sentinel are filtered;
//!   * the **final `sent = total` fix-up tick** (helpers.go:298-303): GetObject's
//!     progress under-reports by the first data packet's bytes (the same quirk Go
//!     had — see keel-mtp's `bulk_read`), so when the last reported `sent` is short
//!     of the file size, one extra progress tick with `sent == total` is emitted.
//!     Load-bearing for the `ActiveFileSize.Sent == .Total` download assertions.
//!
//! Fidelity FIX (plan §3.5 "nondeterministic download order"): with pre-processing
//! on, Go accumulated files into a `map[string]…` and then `range`d it — Go map
//! iteration order is randomised, so the transfer order (and thus the
//! progress-event sequence) was nondeterministic. keel uses a [`BTreeMap`] keyed by
//! destination path: same last-write-wins de-duplication Go's map gave, but a
//! stable, sorted iteration order.
//!
//! Structural deviation (forced by Rust's borrow checker): in Go's no-preprocess
//! path the transfer runs *inside* the `Walk` callback, i.e. it uses the device
//! while `Walk` is still enumerating with it. keel cannot hold `&mut MtpSession` in
//! both `walk` and its callback, so it collects the walked `FileInfo`s first
//! (callback touches only a `Vec`), then transfers them. The device-DFS transfer
//! order is preserved, so the file sequence and every progress value are identical;
//! only enumeration/transfer interleaving (unobservable) differs.
//!
//! Cancel seam (keel addition, see upload.rs): `should_cancel` is polled once per
//! preprocessed file and once per progress tick; a fire aborts with the distinct
//! `VfsError::Cancelled` (Go bubbled `TransferCancelledError` through
//! `FileTransferError` and relied on the FFI substring match — equivalent).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::{File, FileTimes};
use std::time::SystemTime;

use keel_mtp::{MtpError, MtpSession, Transport};

use crate::error::VfsError;
use crate::object::FileInfo;
use crate::path::{fix_slash, get_full_path, get_object_from_path};
use crate::progress::{ProgressInfo, TransferStatus, percent, transfer_rate};
use crate::walk::{is_disallowed_files, walk};

/// Download preprocess callback: go-mtpx `MtpPreprocessCb`
/// (`func(fi *FileInfo, err error) error`, structs.go:91), nil-error slot dropped.
pub type MtpPreprocessCb<'a> = &'a mut dyn FnMut(&FileInfo) -> Result<(), VfsError>;

/// Progress callback (same shape as upload): go-mtpx `ProgressCb`.
pub type ProgressCb<'a> = &'a mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>;

/// go-mtpx `processDownloadFilesProps` (structs.go:98-101) — the mutable transfer
/// bookkeeping threaded through `processDownloadFiles`.
struct DownloadProps {
    destination_file_parent_path: String,
    destination_file_path: String,
    source_parent_path: String,
    bulk_files_sent: i64,
    bulk_size_sent: i64,
    total_files: i64,
    total_size: i64,
}

/// go-mtpx `downloadFilesObjectCacheContainer` (structs.go:105-108).
struct DownloadCacheEntry {
    file_info: FileInfo,
    source_parent_path: String,
    destination_file_parent_path: String,
    destination_file_path: String,
}

/// Splits the two error origins Go's `processDownloadFilesError` switch cares
/// about: a raw local-fs error ([`Self::Io`], Go's `*os.PathError` from
/// `os.Create`/`os.Chtimes`) versus everything else ([`Self::Vfs`] — already a
/// typed mtpx error, an mtp error, or `Cancelled`).
enum DlError {
    Vfs(VfsError),
    Io(std::io::Error),
}

/// A per-chunk progress sink (see upload.rs): `FnMut(total, sent, object_id)`.
type SizeProgressCb<'a> = &'a mut dyn FnMut(i64, i64, u32) -> Result<(), VfsError>;

/// Transfer files from the device to the local disk — port of go-mtpx
/// `DownloadFiles` (main.go:556-700).
///
/// Returns `(bulk_files_sent, bulk_size_sent)`. As with `upload_files`, the
/// partial counts Go returned alongside an error are dropped (keel-ffi discards
/// them on error anyway).
#[allow(clippy::too_many_arguments)] // 1:1 with the Go signature (main.go:556).
pub fn download_files<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    sources: &[String],
    destination: &str,
    preprocess_files: bool,
    preprocess_cb: MtpPreprocessCb<'_>,
    progress_cb: ProgressCb<'_>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(i64, i64), VfsError> {
    let _destination = fix_slash(destination);

    let mut pinfo = ProgressInfo::new();

    let mut total_files: i64 = 0;
    let mut total_directories: i64 = 0;
    let mut total_size: i64 = 0;

    let bulk_files_sent: i64 = 0;
    let bulk_size_sent: i64 = 0;

    // The preprocess object cache (main.go:589). Fidelity fix: a BTreeMap for a
    // deterministic, sorted iteration order (Go's `map` was randomised).
    let mut cache: BTreeMap<String, DownloadCacheEntry> = BTreeMap::new();

    // --- optional preprocess pass (main.go:590-637) ---
    if preprocess_files {
        for source in sources {
            let _source = fix_slash(source);
            let source_parent_path = go_filepath_dir(&_source);

            let walk_res = walk(
                session,
                storage_id,
                &_source,
                true,  // recursive
                true,  // skip_disallowed_files
                false, // skip_hidden_files
                &mut |_object_id: u32, fi: &FileInfo| -> Result<(), VfsError> {
                    let (destination_file_parent_path, destination_file_path) =
                        map_source_path_to_destination_path(
                            &fi.full_path,
                            &source_parent_path,
                            &_destination,
                        );

                    // main.go:605 — cache EVERY walked object (dirs included).
                    cache.insert(
                        destination_file_path.clone(),
                        DownloadCacheEntry {
                            file_info: fi.clone(),
                            source_parent_path: source_parent_path.clone(),
                            destination_file_parent_path,
                            destination_file_path,
                        },
                    );

                    if fi.is_dir {
                        return Ok(());
                    }
                    if is_disallowed_files(&fi.name) {
                        return Ok(());
                    }
                    // Cancel per preprocessed file (legacy kernel L454, polled inside the
                    // download preprocess callback — reached only for real files).
                    if should_cancel() {
                        return Err(VfsError::Cancelled);
                    }
                    preprocess_cb(fi)?;
                    total_size += fi.size;
                    Ok(())
                },
            );

            // main.go:630-632 — a Walk failure returns raw.
            let (_oid, tf, td) = walk_res?;
            total_files += tf;
            total_directories += td;
        }
    }

    pinfo.total_files = total_files;
    pinfo.total_directories = total_directories;
    pinfo.bulk_file_size.total = total_size;

    let mut dfprops = DownloadProps {
        destination_file_parent_path: String::new(),
        destination_file_path: String::new(),
        source_parent_path: String::new(),
        bulk_files_sent,
        bulk_size_sent,
        total_files,
        total_size,
    };

    if !cache.is_empty() {
        // --- preprocessed path (main.go:650-661): transfer from the cache in the
        // BTreeMap's deterministic key order.
        for entry in cache.values() {
            dfprops.source_parent_path = entry.source_parent_path.clone();
            dfprops.destination_file_parent_path = entry.destination_file_parent_path.clone();
            dfprops.destination_file_path = entry.destination_file_path.clone();

            if let Err(e) = process_download_files(
                session,
                &mut pinfo,
                &entry.file_info,
                progress_cb,
                &mut dfprops,
                should_cancel,
            ) {
                return Err(process_download_files_error(e));
            }
        }
    } else {
        // --- no-preprocess path (main.go:662-692). See the module note: collect,
        // then transfer, to avoid holding `&mut MtpSession` in both `walk` and its
        // callback.
        for source in sources {
            let _source = fix_slash(source);

            // main.go:666-669 — validate the source path; a failure returns raw.
            get_object_from_path(session, storage_id, &_source)?;

            let source_parent_path = go_filepath_dir(&_source);

            let mut collected: Vec<FileInfo> = Vec::new();
            let walk_res = walk(
                session,
                storage_id,
                &_source,
                true,
                true,
                false,
                &mut |_object_id: u32, fi: &FileInfo| -> Result<(), VfsError> {
                    collected.push(fi.clone());
                    Ok(())
                },
            );
            if let Err(e) = walk_res {
                return Err(process_download_files_error(DlError::Vfs(e)));
            }

            for fi in &collected {
                let (destination_file_parent_path, destination_file_path) =
                    map_source_path_to_destination_path(
                        &fi.full_path,
                        &source_parent_path,
                        &_destination,
                    );
                dfprops.source_parent_path = source_parent_path.clone();
                dfprops.destination_file_parent_path = destination_file_parent_path;
                dfprops.destination_file_path = destination_file_path;

                if let Err(e) = process_download_files(
                    session,
                    &mut pinfo,
                    fi,
                    progress_cb,
                    &mut dfprops,
                    should_cancel,
                ) {
                    return Err(process_download_files_error(e));
                }
            }
        }
    }

    // --- final Completed tick (main.go:694-697) ---
    pinfo.status = TransferStatus::Completed;
    if should_cancel() {
        return Err(VfsError::Cancelled);
    }
    progress_cb(&pinfo)?;

    Ok((dfprops.bulk_files_sent, dfprops.bulk_size_sent))
}

/// Port of go-mtpx `processDownloadFiles` (helpers.go:464-535) — download a single
/// walked object (dir → local mkdir; file → GetObject + progress).
fn process_download_files<T: Transport>(
    session: &mut MtpSession<T>,
    pinfo: &mut ProgressInfo,
    fi: &FileInfo,
    progress_cb: ProgressCb<'_>,
    dfprops: &mut DownloadProps,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), DlError> {
    // helpers.go:466-468.
    if is_disallowed_files(&fi.name) {
        return Ok(());
    }

    // helpers.go:471-480 — a directory becomes a local directory, carrying its
    // device modification time.
    if fi.is_dir {
        make_local_directory(&dfprops.destination_file_path, fi.mod_time)?;
        return Ok(());
    }

    // helpers.go:483-491 — create the local parent on demand (with "now" as its
    // mtime, since we lack the parent's real one here).
    if !file_exists_local(&dfprops.destination_file_parent_path) {
        make_local_directory(
            &dfprops.destination_file_parent_path,
            Some(SystemTime::now()),
        )?;
    }

    // helpers.go:494 — counted BEFORE the transfer.
    dfprops.bulk_files_sent += 1;
    pinfo.latest_sent_time = SystemTime::now(); // helpers.go:496
    pinfo.file_info = fi.clone(); // helpers.go:497

    // Pulled out so the progress closure can borrow `dfprops` (for bulk_size_sent)
    // without aliasing the `destination_file_path` handed to handle_make_local_file.
    let destination_file_path = dfprops.destination_file_path.clone();
    let total_size = dfprops.total_size;

    let transfer_result = {
        let mut prev_sent_size: i64 = 0;
        // The SizeProgressCb closure (helpers.go:502-525).
        let mut size_progress = |total: i64, sent: i64, _obj_id: u32| -> Result<(), VfsError> {
            pinfo.active_file_size.total = total;
            pinfo.active_file_size.sent = sent;
            pinfo.active_file_size.progress = percent(sent as f32, total as f32);

            let chunk_size = sent - prev_sent_size;
            dfprops.bulk_size_sent += chunk_size;

            pinfo.bulk_file_size.sent = dfprops.bulk_size_sent;
            pinfo.bulk_file_size.progress =
                percent(dfprops.bulk_size_sent as f32, total_size as f32);

            pinfo.speed = transfer_rate(chunk_size, pinfo.latest_sent_time);
            progress_cb(pinfo)?;

            pinfo.latest_sent_time = SystemTime::now();
            prev_sent_size = sent;
            Ok(())
        };

        handle_make_local_file(
            session,
            fi,
            &destination_file_path,
            should_cancel,
            &mut size_progress,
        )
    };
    transfer_result?;

    // helpers.go:531-532.
    pinfo.files_sent = dfprops.bulk_files_sent;
    pinfo.files_sent_progress = percent(dfprops.bulk_files_sent as f32, dfprops.total_files as f32);

    Ok(())
}

/// Port of go-mtpx `handleMakeLocalFile` (helpers.go:277-306): create (silently
/// overwrite) the local file, stream the object into it, emit the sent=total
/// fix-up tick if progress fell short, then restore the device mtime.
fn handle_make_local_file<T: Transport>(
    session: &mut MtpSession<T>,
    fi: &FileInfo,
    destination: &str,
    should_cancel: &dyn Fn() -> bool,
    size_progress_cb: SizeProgressCb<'_>,
) -> Result<(), DlError> {
    // helpers.go:278-282 — os.Create: create or truncate (silent overwrite).
    let mut f = File::create(destination).map_err(DlError::Io)?;

    // GetObject streams the object to `f`; progress reports cumulative bytes.
    let cancelled = Cell::new(false);
    let mut inner_err: Option<VfsError> = None;
    let mut total_sent: i64 = 0;

    let res = {
        let mut prog = |sent: u64, _handle: u32| -> Result<(), MtpError> {
            // Cancel per progress tick.
            if should_cancel() {
                cancelled.set(true);
                return Err(MtpError::Closed); // benign, non-poisoning; always intercepted
            }
            match size_progress_cb(fi.size, sent as i64, fi.object_id) {
                Ok(()) => {
                    total_sent = sent as i64; // helpers.go:290
                    Ok(())
                }
                Err(e) => {
                    inner_err = Some(e);
                    Err(MtpError::Closed)
                }
            }
        };
        session.get_object(fi.object_id, &mut f, &mut prog)
    };

    if cancelled.get() {
        return Err(DlError::Vfs(VfsError::Cancelled));
    }
    if let Some(e) = inner_err {
        return Err(DlError::Vfs(e));
    }
    // helpers.go:294-296 — a GetObject failure returns raw (an mtp error; the
    // caller's switch lands it in the FileTransferError default arm).
    res.map_err(|e| DlError::Vfs(VfsError::Mtp(e)))?;

    // helpers.go:298-303 — the sent=total fix-up tick. GetObject's progress omits
    // the first data packet's bytes (keel-mtp bulk_read, matching Go), so a
    // successful transfer usually ends with total_sent < fi.size; emit one final
    // tick at 100 %.
    if total_sent < fi.size {
        if should_cancel() {
            return Err(DlError::Vfs(VfsError::Cancelled));
        }
        size_progress_cb(fi.size, fi.size, fi.object_id).map_err(DlError::Vfs)?;
    }

    // helpers.go:305 — restore the device modification time onto the local file.
    restore_local_file_timestamp(destination, fi.mod_time)
}

/// Port of go-mtpx `makeLocalDirectory` (helpers.go:387-404): `os.MkdirAll` then
/// restore the mtime. MkdirAll failures are classified permission ⇒
/// `FilePermission`, else `LocalFile` (Go's `*os.PathError` arm); the mtime
/// restore surfaces its raw error for the top-level switch (Go returned it raw).
fn make_local_directory(filename: &str, mod_time: Option<SystemTime>) -> Result<(), DlError> {
    if let Err(e) = std::fs::create_dir_all(filename) {
        // helpers.go:390-400. (All std fs errors are io::Error ≈ *os.PathError; Go's
        // `default` non-PathError arm is unreachable here, so every error is
        // classified into the two mtpx variants.)
        return Err(DlError::Vfs(
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                VfsError::FilePermission(e.to_string())
            } else {
                VfsError::LocalFile(e.to_string())
            },
        ));
    }
    restore_local_file_timestamp(filename, mod_time)
}

/// Port of go-mtpx `restoreLocalFileTimestamp` (helpers.go:563-572): set the access
/// time to now and the modification time to `mod_time` (Go's `os.Chtimes(dest, now,
/// modTime)`).
///
/// `mod_time` is `Option` because `FileInfo.mod_time` is (`None` == Go's zero
/// `time.Time`); a `None` degrades to the Unix epoch rather than Go's year-1 zero
/// time — a harmless difference that only surfaces for objects the device reports
/// with no modification date.
///
/// Go's `os.Chtimes` operates on the path; std has no path-based equivalent, so
/// keel opens the target and uses [`File::set_times`]. This works on directories
/// too (as `makeLocalDirectory` needs) on Unix — see the returned open-issues list.
/// A failure surfaces as [`DlError::Io`], matching Go's raw `*os.PathError`.
fn restore_local_file_timestamp(path: &str, mod_time: Option<SystemTime>) -> Result<(), DlError> {
    let modified = mod_time.unwrap_or(SystemTime::UNIX_EPOCH);
    let times = FileTimes::new()
        .set_accessed(SystemTime::now())
        .set_modified(modified);
    let f = File::options().read(true).open(path).map_err(DlError::Io)?;
    f.set_times(times).map_err(DlError::Io)
}

/// Port of go-mtpx `processDownloadFilesError` (helpers.go:537-560). Classifies the
/// terminal error; the counts Go returned with it are dropped (discarded on error
/// by keel-ffi). A `Cancelled` short-circuit sits in front (keel keeps the distinct
/// variant instead of Go's FileTransferError-wrap-plus-substring-match).
fn process_download_files_error(e: DlError) -> VfsError {
    match e {
        DlError::Vfs(VfsError::Cancelled) => VfsError::Cancelled,
        // helpers.go:540-541 — `case InvalidPathError`: passed through.
        DlError::Vfs(v @ VfsError::InvalidPath(_)) => v,
        // helpers.go:543-552 — `case *os.PathError`: permission ⇒ FilePermission,
        // NotExist ⇒ InvalidPath, else LocalFile.
        DlError::Io(io) => {
            if io.kind() == std::io::ErrorKind::PermissionDenied {
                VfsError::FilePermission(io.to_string())
            } else if io.kind() == std::io::ErrorKind::NotFound {
                VfsError::InvalidPath(io.to_string())
            } else {
                VfsError::LocalFile(io.to_string())
            }
        }
        // helpers.go:553-555 default arm.
        DlError::Vfs(v) => {
            VfsError::FileTransfer(format!("an error occured while downloading the files. {v}"))
        }
    }
}

/// go-mtpx `fileExistsLocal` (utils.go:136-140): `!os.IsNotExist(err)` — true when
/// the path exists OR the stat failed for a reason other than "not found".
fn file_exists_local(filename: &str) -> bool {
    match std::fs::metadata(filename) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// go-mtpx `mapSourcePathToDestinationPath` (utils.go:217-224). Duplicated from
/// upload.rs (shared, unported by any sibling); the gate agent may hoist both
/// copies into `path.rs`.
fn map_source_path_to_destination_path(
    source_path: &str,
    source_parent_path: &str,
    destination_path: &str,
) -> (String, String) {
    let trimmed = source_path
        .strip_prefix(source_parent_path)
        .unwrap_or(source_path);
    let full_path = get_full_path(destination_path, trimmed);
    (go_filepath_dir(&full_path), full_path)
}

/// Unix `filepath.Dir` for a `fixSlash`'d path — `sourceParentPath =
/// filepath.Dir(_source)` (main.go:600/677). Duplicated from `upload.rs`
/// deliberately so each transfer module stands alone.
fn go_filepath_dir(p: &str) -> String {
    match p.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_exists_local_semantics() {
        assert!(file_exists_local("/")); // root always exists
        assert!(!file_exists_local(
            "/this/path/should/not/exist/keel-vfs-test"
        ));
    }

    #[test]
    fn go_dir_matches_filepath_dir() {
        assert_eq!(
            go_filepath_dir("/mtp-test-files/mock_dir1"),
            "/mtp-test-files"
        );
        assert_eq!(go_filepath_dir("/a"), "/");
        assert_eq!(go_filepath_dir("/"), "/");
    }

    #[test]
    fn map_source_to_destination_download() {
        // Download: device path /mtp/mock_dir1/a.txt under parent /mtp → dest/mock_dir1/a.txt
        let (parent, file) =
            map_source_path_to_destination_path("/mtp/mock_dir1/a.txt", "/mtp", "/local-dest");
        assert_eq!(file, "/local-dest/mock_dir1/a.txt");
        assert_eq!(parent, "/local-dest/mock_dir1");
    }
}

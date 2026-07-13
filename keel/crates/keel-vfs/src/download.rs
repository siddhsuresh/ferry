//! `download_files` — device → local disk: enumerate objects under the sources,
//! recreate the tree on disk, and stream each object into a local file.
//!
//! Operates on a generic `&mut MtpSession<T>` like the rest of keel-vfs; the FFI
//! reaches it via `Device::session_mut()`.
//!
//! Load-bearing behaviours:
//!   * each object is fetched whole via GetObject and streamed straight to
//!     `File::create` — a **silent** local overwrite;
//!   * the device modification time is restored onto the local file after the
//!     write;
//!   * local parent directories are created on demand;
//!   * `.DS_Store` and the test sentinel are filtered out;
//!   * the **final `sent = total` fix-up tick**: GetObject's progress under-reports
//!     by the first data packet's bytes (see keel-mtp's `bulk_read`), so when the
//!     last reported `sent` is short of the file size, one extra progress tick with
//!     `sent == total` is emitted. Required so the wire contract's
//!     `active_file_size.sent == .total` invariant holds at completion.
//!
//! Deterministic order: with pre-processing on, walked objects are collected into
//! a [`BTreeMap`] keyed by destination path — last-write-wins de-duplication with
//! a stable, sorted iteration order, so the transfer (and progress-event) sequence
//! is reproducible.
//!
//! Collect-then-transfer (forced by the borrow checker): we cannot hold
//! `&mut MtpSession` in both `walk` and its callback, so the no-preprocess path
//! collects the walked `FileInfo`s first (the callback touches only a `Vec`), then
//! transfers them. The device-DFS transfer order is preserved, so the file
//! sequence and every progress value are unchanged; only the (unobservable)
//! enumeration/transfer interleaving differs.
//!
//! Cancellation (see upload.rs): `should_cancel` is polled once per preprocessed
//! file and once per progress tick; a fire aborts with the distinct
//! `VfsError::Cancelled`.

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

/// Download preprocess callback, invoked once per file during the counting pass.
pub type MtpPreprocessCb<'a> = &'a mut dyn FnMut(&FileInfo) -> Result<(), VfsError>;

/// Progress callback (same shape as upload), invoked on every per-chunk tick.
pub type ProgressCb<'a> = &'a mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>;

/// Mutable transfer bookkeeping threaded through `process_download_files`.
struct DownloadProps {
    destination_file_parent_path: String,
    destination_file_path: String,
    source_parent_path: String,
    bulk_files_sent: i64,
    bulk_size_sent: i64,
    total_files: i64,
    total_size: i64,
}

/// A single walked object staged for transfer, keyed by destination path.
struct DownloadCacheEntry {
    file_info: FileInfo,
    source_parent_path: String,
    destination_file_parent_path: String,
    destination_file_path: String,
}

/// Splits the two error origins the terminal classifier cares about: a raw
/// local-fs error ([`Self::Io`], from `File::create`/set-times) versus everything
/// else ([`Self::Vfs`] — an already-typed vfs error, an mtp error, or
/// `Cancelled`).
enum DlError {
    Vfs(VfsError),
    Io(std::io::Error),
}

/// A per-chunk progress sink (see upload.rs): `FnMut(total, sent, object_id)`.
type SizeProgressCb<'a> = &'a mut dyn FnMut(i64, i64, u32) -> Result<(), VfsError>;

/// Transfer files from the device to the local disk.
///
/// Returns `(bulk_files_sent, bulk_size_sent)`. As with `upload_files`, the
/// partial counts are dropped on error (keel-ffi discards them on error anyway).
#[allow(clippy::too_many_arguments)]
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

    // The preprocess object cache. A BTreeMap gives a deterministic, sorted
    // iteration order.
    let mut cache: BTreeMap<String, DownloadCacheEntry> = BTreeMap::new();

    // --- optional preprocess pass ---
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

                    // Cache EVERY walked object (dirs included).
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
                    // Cancel per preprocessed file (reached only for real files).
                    if should_cancel() {
                        return Err(VfsError::Cancelled);
                    }
                    preprocess_cb(fi)?;
                    total_size += fi.size;
                    Ok(())
                },
            );

            // A walk failure returns raw.
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
        // --- preprocessed path: transfer from the cache in the BTreeMap's
        // deterministic key order.
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
        // --- no-preprocess path. See the module note: collect, then transfer, to
        // avoid holding `&mut MtpSession` in both `walk` and its callback.
        for source in sources {
            let _source = fix_slash(source);

            // Validate the source path; a failure returns raw.
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

    // --- final Completed tick ---
    pinfo.status = TransferStatus::Completed;
    if should_cancel() {
        return Err(VfsError::Cancelled);
    }
    progress_cb(&pinfo)?;

    Ok((dfprops.bulk_files_sent, dfprops.bulk_size_sent))
}

/// Download a single walked object (dir → local mkdir; file → GetObject +
/// progress).
fn process_download_files<T: Transport>(
    session: &mut MtpSession<T>,
    pinfo: &mut ProgressInfo,
    fi: &FileInfo,
    progress_cb: ProgressCb<'_>,
    dfprops: &mut DownloadProps,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), DlError> {
    if is_disallowed_files(&fi.name) {
        return Ok(());
    }

    // A directory becomes a local directory, carrying its device modification time.
    if fi.is_dir {
        make_local_directory(&dfprops.destination_file_path, fi.mod_time)?;
        return Ok(());
    }

    // Create the local parent on demand (with "now" as its mtime, since we lack
    // the parent's real one here).
    if !file_exists_local(&dfprops.destination_file_parent_path) {
        make_local_directory(
            &dfprops.destination_file_parent_path,
            Some(SystemTime::now()),
        )?;
    }

    // Counted BEFORE the transfer.
    dfprops.bulk_files_sent += 1;
    pinfo.latest_sent_time = SystemTime::now();
    pinfo.file_info = fi.clone();

    // Pulled out so the progress closure can borrow `dfprops` (for bulk_size_sent)
    // without aliasing the `destination_file_path` handed to handle_make_local_file.
    let destination_file_path = dfprops.destination_file_path.clone();
    let total_size = dfprops.total_size;

    let transfer_result = {
        let mut prev_sent_size: i64 = 0;
        // The per-chunk progress closure.
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

    // Update the post-transfer bookkeeping.
    pinfo.files_sent = dfprops.bulk_files_sent;
    pinfo.files_sent_progress = percent(dfprops.bulk_files_sent as f32, dfprops.total_files as f32);

    Ok(())
}

/// Create (silently overwrite) the local file, stream the object into it, emit
/// the sent=total fix-up tick if progress fell short, then restore the device
/// mtime.
fn handle_make_local_file<T: Transport>(
    session: &mut MtpSession<T>,
    fi: &FileInfo,
    destination: &str,
    should_cancel: &dyn Fn() -> bool,
    size_progress_cb: SizeProgressCb<'_>,
) -> Result<(), DlError> {
    // Create or truncate the local file (silent overwrite).
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
                    total_sent = sent as i64;
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
    // A GetObject failure returns raw (an mtp error; the caller lands it in the
    // FileTransfer default arm).
    res.map_err(|e| DlError::Vfs(VfsError::Mtp(e)))?;

    // The sent=total fix-up tick. GetObject's progress omits the first data
    // packet's bytes (see keel-mtp bulk_read), so a successful transfer usually
    // ends with total_sent < fi.size; emit one final tick at 100 %.
    if total_sent < fi.size {
        if should_cancel() {
            return Err(DlError::Vfs(VfsError::Cancelled));
        }
        size_progress_cb(fi.size, fi.size, fi.object_id).map_err(DlError::Vfs)?;
    }

    // Restore the device modification time onto the local file.
    restore_local_file_timestamp(destination, fi.mod_time)
}

/// Create the directory (and parents), then restore the mtime. Create failures
/// are classified permission ⇒ `FilePermission`, else `LocalFile`; the mtime
/// restore surfaces its raw error for the top-level classifier.
fn make_local_directory(filename: &str, mod_time: Option<SystemTime>) -> Result<(), DlError> {
    if let Err(e) = std::fs::create_dir_all(filename) {
        // Every create-dir failure is an io::Error, classified into one of the two
        // vfs variants.
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

/// Set the access time to now and the modification time to `mod_time`.
///
/// `mod_time` is `Option` because `FileInfo.mod_time` is; a `None` degrades to the
/// Unix epoch, which only surfaces for objects the device reports with no
/// modification date.
///
/// std has no path-based set-times, so we open the target and use
/// [`File::set_times`]. This works on directories too (as `make_local_directory`
/// needs) on Unix. A failure surfaces as [`DlError::Io`].
fn restore_local_file_timestamp(path: &str, mod_time: Option<SystemTime>) -> Result<(), DlError> {
    let modified = mod_time.unwrap_or(SystemTime::UNIX_EPOCH);
    let times = FileTimes::new()
        .set_accessed(SystemTime::now())
        .set_modified(modified);
    let f = File::options().read(true).open(path).map_err(DlError::Io)?;
    f.set_times(times).map_err(DlError::Io)
}

/// Classify the terminal download error; the partial counts are dropped
/// (discarded on error by keel-ffi). A `Cancelled` short-circuit sits in front.
fn process_download_files_error(e: DlError) -> VfsError {
    match e {
        DlError::Vfs(VfsError::Cancelled) => VfsError::Cancelled,
        // An `InvalidPath` passes through.
        DlError::Vfs(v @ VfsError::InvalidPath(_)) => v,
        // A raw local-fs error: permission ⇒ FilePermission, NotExist ⇒
        // InvalidPath, else LocalFile.
        DlError::Io(io) => {
            if io.kind() == std::io::ErrorKind::PermissionDenied {
                VfsError::FilePermission(io.to_string())
            } else if io.kind() == std::io::ErrorKind::NotFound {
                VfsError::InvalidPath(io.to_string())
            } else {
                VfsError::LocalFile(io.to_string())
            }
        }
        // Default: everything else becomes a FileTransfer error.
        DlError::Vfs(v) => {
            VfsError::FileTransfer(format!("an error occured while downloading the files. {v}"))
        }
    }
}

/// True when the path exists OR the stat failed for a reason other than "not
/// found".
fn file_exists_local(filename: &str) -> bool {
    match std::fs::metadata(filename) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Duplicated from `upload.rs` so each transfer module stands alone. Both copies
/// may be hoisted into `path.rs`.
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

/// The parent directory of a `fix_slash`'d path. Duplicated from `upload.rs`
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

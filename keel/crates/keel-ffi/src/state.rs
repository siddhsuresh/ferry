//! Global device container + the session-verify / operation helper layer.
//!
//! Holds the process-global device + device-info, and implements the initialize /
//! fetch-device-info / fetch-storages / make-directory / file-exists / delete-file /
//! rename-file / walk / upload / download / dispose helpers plus `verify_mtp_session`
//! / [`lock_mtp`], including the Samsung EOF storage-access workaround. Each `abi.rs`
//! export calls one of these functions.
//!
//! Errors are returned as [`KernelError`] (defined in `errors.rs`, which owns error
//! classification). `KernelError::Vfs` carries a typed vfs error; `KernelError::Message`
//! carries the bare-string sentinels (the two `verify_mtp_session` sentinels, the EOF
//! wrap, the unmarshal failures).
//!
//! ## Two locks, deliberately distinct
//!
//! There is one process-global container, and its `locked` flag is a documented no-op
//! (see [`lock_mtp`]). A mutable global cannot be shared without a real lock, so
//! [`CONTAINER`] is a genuine `Mutex` **purely for memory safety** (plan §2 /
//! keel-ffi/state). That mutex is orthogonal to the `locked` flag: it serializes
//! concurrent access to the container's memory, which Ferry never actually triggers
//! (it drives every export from one dispatch queue), so it is observably invisible.
//! The `locked`-flag no-op is reproduced separately by [`lock_mtp`].

use std::fs::Metadata;
use std::sync::{Mutex, MutexGuard, PoisonError};

use keel_mtp::MtpSession;
use keel_mtp::session::UsbInfo;
use keel_proto::DeviceInfo;
use keel_usb::UsbTransport;
use keel_vfs::device::{Init, StorageData};
use keel_vfs::path::FileProp;
use keel_vfs::{Device, FileInfo, ProgressInfo, VfsError};

use crate::errors::{self, KernelError};

/// The concrete session type the vfs path operations drive. `Device::session_mut`
/// hands one out; `with_session` threads it into each op.
type Session = MtpSession<UsbTransport>;

/// The `"ErrorMtpDetectFailed"` sentinel — returned whenever the container has no
/// device. Factored so every `c.dev` miss produces byte-identical text, which error
/// classification maps to `ErrorMtpDetectFailed` by string equality.
fn detect_failed() -> KernelError {
    KernelError::Message("ErrorMtpDetectFailed".to_string())
}

// ---------------------------------------------------------------------------
// The global container
// ---------------------------------------------------------------------------

/// The process-global device container, minus the `locked` field (see [`lock_mtp`] —
/// it is a no-op, so the field is omitted and the no-op reproduced directly).
struct DeviceContainer {
    /// The connected device, if any.
    dev: Option<Device>,
    /// The cached device info. Present after a successful device-info fetch; its
    /// `serial_number` is what the device-change check compares against.
    device_info: Option<DeviceInfo>,
}

/// The single process-global device container. See the module doc for why this is
/// `Mutex`-wrapped: memory safety only, not the (no-op) `locked` flag.
static CONTAINER: Mutex<DeviceContainer> = Mutex::new(DeviceContainer {
    dev: None,
    device_info: None,
});

/// Lock the container, recovering from poisoning. A panic inside an export (caught by
/// abi.rs's `catch_unwind`) can poison this mutex; we recover the inner value so the
/// next export still works.
fn lock_container() -> MutexGuard<'static, DeviceContainer> {
    CONTAINER.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// lock_mtp — the documented no-op
// ---------------------------------------------------------------------------

/// **Intentional no-op — a preserved quirk (plan §3.5).** The original lock set a
/// `locked` flag, then immediately released it on return via a deferred reset — so the
/// flag was cleared again before the caller did any work. The lock was therefore never
/// actually held across an operation; it could only ever observe `locked == false` and
/// return `Ok`. Every export calls it first, but the `ErrorMtpLockExists` path is dead
/// code.
///
/// A *real* lock here would change observable behaviour (it would serialize or reject
/// concurrent exports), which the plan forbids. So this reproduces the no-op: always
/// `Ok`. Container memory safety is handled separately by [`CONTAINER`]'s mutex, not by
/// this.
pub(crate) fn lock_mtp() -> Result<(), KernelError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// verify_mtp_session + dispose
// ---------------------------------------------------------------------------

/// Verify the session before an operation.
///
/// `skip_device_change_check` is `true` only for `initialize` / `fetch_device_info`,
/// `false` for every other op. When `false` (and a device info is on record), it
/// re-fetches device info and compares serials; a mismatch is `ErrorDeviceChanged`, and
/// a fetch failure drops the stored info, disposes the device (the Samsung usb-timeout
/// mitigation), and surfaces the underlying error.
fn verify_mtp_session(
    c: &mut DeviceContainer,
    skip_device_change_check: bool,
) -> Result<(), KernelError> {
    // No device ⇒ ErrorMtpDetectFailed.
    if c.dev.is_none() {
        return Err(detect_failed());
    }

    // The change check runs only when not skipped AND a prior device info exists to
    // compare against.
    if skip_device_change_check || c.device_info.is_none() {
        return Ok(());
    }

    // Re-fetch device info. Compute the result first so the `&mut` borrow of `c.dev`
    // ends before we mutate `c.device_info` / dispose below.
    let fetched = match c.dev.as_mut() {
        Some(d) => d.fetch_device_info(),
        None => return Err(detect_failed()),
    };

    match fetched {
        Ok(dinfo) => {
            // Serial compare.
            let changed = c
                .device_info
                .as_ref()
                .map(|stored| stored.serial_number != dinfo.serial_number)
                .unwrap_or(false);
            if changed {
                // Update the stored info, report the change.
                c.device_info = Some(dinfo);
                Err(KernelError::Message("ErrorDeviceChanged".to_string()))
            } else {
                Ok(())
            }
        }
        Err(e) => {
            // Drop stored info, dispose, surface the raw error.
            c.device_info = None;
            dispose(c);
            Err(KernelError::Vfs(e))
        }
    }
}

/// Dispose the device.
///
/// `Device::dispose` **consumes** the device (there is no `&mut self` close-in-place),
/// so this `take()`s it out — leaving `c.dev == None`. A subsequent op issued after a
/// verify-time device-info failure therefore finds `None` and returns
/// `ErrorMtpDetectFailed` rather than a device error, but either way an error is
/// surfaced.
fn dispose(c: &mut DeviceContainer) {
    if let Some(dev) = c.dev.take() {
        dev.dispose();
    }
}

// ---------------------------------------------------------------------------
// The verify-then-run harness for the path operations
// ---------------------------------------------------------------------------

/// Lock the container, run `verify_mtp_session`, then hand the live session to `op`.
/// Every path-level helper (make_directory / file_exists / delete_file / rename_file /
/// walk / upload / download) is this shape: verify, then a single vfs call whose error
/// is wrapped `KernelError::Vfs`.
///
/// The container mutex is held for the whole `op` — including a multi-GB transfer.
/// That is intentional and harmless: exports block by contract, Ferry serializes
/// them, and `CancelTransfer` (an atomic) and the 500 ms sampler thread never touch
/// this mutex, so nothing deadlocks against a long-running `op`.
fn with_session<R>(
    skip_device_change_check: bool,
    op: impl FnOnce(&mut Session) -> Result<R, VfsError>,
) -> Result<R, KernelError> {
    let mut c = lock_container();
    verify_mtp_session(&mut c, skip_device_change_check)?;
    let dev = match c.dev.as_mut() {
        Some(d) => d,
        None => return Err(detect_failed()),
    };
    op(dev.session_mut()).map_err(KernelError::Vfs)
}

// ---------------------------------------------------------------------------
// Device lifecycle
// ---------------------------------------------------------------------------

/// Open the device, fetch its info, then read its USB descriptor. Returns the device
/// info and USB descriptor for abi.rs to serialise.
pub(crate) fn initialize() -> Result<(DeviceInfo, UsbInfo), KernelError> {
    let mut c = lock_container();

    // Open the device → container.dev = d. Any prior device is dropped here, and closed
    // on drop.
    let dev = Device::initialize(Init { debug_mode: false }).map_err(KernelError::Vfs)?;
    c.dev = Some(dev);

    // Fetch device info, skipping the device-change check.
    let dinfo = fetch_device_info_inner(&mut c)?;

    // The USB descriptor captured at discovery lives on the session (device.rs installs
    // it at configure).
    let usb = match c.dev.as_ref() {
        Some(d) => d.session().usb_info().clone(),
        None => return Err(detect_failed()),
    };
    Ok((dinfo, usb))
}

/// Fetch the device info, then read its USB descriptor.
pub(crate) fn fetch_device_info() -> Result<(DeviceInfo, UsbInfo), KernelError> {
    let mut c = lock_container();
    let dinfo = fetch_device_info_inner(&mut c)?;
    let usb = match c.dev.as_ref() {
        Some(d) => d.session().usb_info().clone(),
        None => return Err(detect_failed()),
    };
    Ok((dinfo, usb))
}

/// Verify with the device-change check skipped, fetch, and cache the result in the
/// container (or drop the cache on failure).
fn fetch_device_info_inner(c: &mut DeviceContainer) -> Result<DeviceInfo, KernelError> {
    verify_mtp_session(c, true)?;

    let fetched = match c.dev.as_mut() {
        Some(d) => d.fetch_device_info(),
        None => return Err(detect_failed()),
    };
    match fetched {
        Ok(dinfo) => {
            // Cache the fetched info.
            c.device_info = Some(dinfo.clone());
            Ok(dinfo)
        }
        Err(e) => {
            // Drop the cached info and return the error (no dispose).
            c.device_info = None;
            Err(KernelError::Vfs(e))
        }
    }
}

/// Fetch the device's storages, wrapped in the Samsung EOF workaround.
///
/// On any storage error, when both a device AND a device info are on record and the
/// error text contains `"EOF"`, the error is rewritten to `"error allow storage
/// access. …"` and the device is disposed — a mitigation to stop Samsung phones
/// spewing USB timeouts once storage access has been denied. The rewritten message's
/// `"allow storage access"` substring is what error classification keys on to map it to
/// `ErrorAllowStorageAccess`.
///
/// The `"EOF"` substring test depends on keel-usb / keel-mtp surfacing a
/// `"EOF"`-containing error string when a Samsung device denies storage access. If that
/// transport error renders differently, this workaround will not fire.
pub(crate) fn fetch_storages() -> Result<Vec<StorageData>, KernelError> {
    let mut c = lock_container();
    verify_mtp_session(&mut c, false)?;

    let fetched = match c.dev.as_mut() {
        Some(d) => d.fetch_storages(),
        None => return Err(detect_failed()),
    };

    match fetched {
        Ok(storages) => Ok(storages),
        Err(e) => {
            // dev present && deviceInfo present && err contains "EOF".
            if c.dev.is_some() && c.device_info.is_some() && e.to_string().contains("EOF") {
                // The verbatim sentinel is built by errors.rs (which owns its exact
                // text, since classification keys on its substring).
                let msg = errors::allow_storage_access_sentinel(&e.to_string());
                // Dispose to stop the Samsung usb-timeout storm.
                dispose(&mut c);
                Err(KernelError::Message(msg))
            } else {
                Err(KernelError::Vfs(e))
            }
        }
    }
}

/// Dispose the device, then clear both container fields.
pub(crate) fn dispose_export() -> Result<(), KernelError> {
    let mut c = lock_container();
    // Already clears c.dev (via take).
    dispose(&mut c);
    // Clear the cached device info.
    c.device_info = None;
    Ok(())
}

// ---------------------------------------------------------------------------
// Path operations (make_directory / file_exists / delete_file / rename_file /
// walk / upload_files / download_files)
// ---------------------------------------------------------------------------

/// The device-generated thumbnail for an object, via MTP `GetThumb`. `Ok(None)` when
/// the object has none (folders, documents, unsupported) so the UI falls back to a
/// glyph.
pub(crate) fn fetch_thumbnail(
    storage_id: u32,
    full_path: &str,
) -> Result<Option<Vec<u8>>, KernelError> {
    with_session(false, |s| keel_vfs::object::thumbnail(s, storage_id, full_path))
}

/// Create a directory at `full_path`. The returned leaf object id is discarded.
pub(crate) fn make_directory(storage_id: u32, full_path: &str) -> Result<(), KernelError> {
    with_session(false, |s| {
        keel_vfs::dirops::make_directory(s, storage_id, full_path).map(|_| ())
    })
}

/// Test each input path for existence. Each path becomes a `FileProp` with `object_id`
/// left 0 so vfs resolves by path. Returns the per-input `exists` flags in order; on the
/// batch-abort quirk the vec is empty (fewer than the inputs), which abi.rs then pairs
/// against the inputs positionally.
pub(crate) fn file_exists(storage_id: u32, files: &[String]) -> Result<Vec<bool>, KernelError> {
    let fprops = to_file_props(files);
    with_session(false, |s| {
        let fc = keel_vfs::dirops::file_exists(s, storage_id, &fprops)?;
        Ok(fc.into_iter().map(|c| c.exists).collect())
    })
}

/// Delete each input path.
pub(crate) fn delete_file(storage_id: u32, files: &[String]) -> Result<(), KernelError> {
    let fprops = to_file_props(files);
    with_session(false, |s| keel_vfs::dirops::delete_file(s, storage_id, &fprops))
}

/// Rename an object. The returned (unchanged) object id is discarded.
pub(crate) fn rename_file(
    storage_id: u32,
    full_path: &str,
    new_file_name: &str,
) -> Result<(), KernelError> {
    let fp = FileProp {
        object_id: 0,
        full_path: full_path.to_string(),
    };
    with_session(false, |s| {
        keel_vfs::dirops::rename_file(s, storage_id, &fp, new_file_name).map(|_| ())
    })
}

/// Walk a directory tree. The callback accumulates the walked `FileInfo`s into a vec.
pub(crate) fn walk(
    storage_id: u32,
    full_path: &str,
    recursive: bool,
    skip_disallowed_files: bool,
    skip_hidden_files: bool,
) -> Result<Vec<FileInfo>, KernelError> {
    with_session(false, |s| {
        let mut files: Vec<FileInfo> = Vec::new();
        keel_vfs::walk::walk(
            s,
            storage_id,
            full_path,
            recursive,
            skip_disallowed_files,
            skip_hidden_files,
            &mut |_object_id, fi| {
                files.push(fi.clone());
                Ok(())
            },
        )?;
        Ok(files)
    })
}

/// Upload files to the device. The preprocess/progress callbacks and the cancel seam
/// are built by abi.rs (they drive the 500 ms sampler); this just verifies the session
/// and forwards to the vfs upload. The returned counts are discarded.
#[allow(clippy::too_many_arguments)] // the transfer signature genuinely needs all of these.
pub(crate) fn upload_files(
    storage_id: u32,
    sources: &[String],
    destination: &str,
    preprocess_files: bool,
    preprocess_cb: &mut dyn FnMut(&Metadata, &str) -> Result<(), VfsError>,
    progress_cb: &mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), KernelError> {
    with_session(false, |s| {
        keel_vfs::upload::upload_files(
            s,
            storage_id,
            sources,
            destination,
            preprocess_files,
            preprocess_cb,
            progress_cb,
            should_cancel,
        )
        .map(|_| ())
    })
}

/// Download files from the device.
#[allow(clippy::too_many_arguments)] // the transfer signature genuinely needs all of these.
pub(crate) fn download_files(
    storage_id: u32,
    sources: &[String],
    destination: &str,
    preprocess_files: bool,
    preprocess_cb: &mut dyn FnMut(&FileInfo) -> Result<(), VfsError>,
    progress_cb: &mut dyn FnMut(&ProgressInfo) -> Result<(), VfsError>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), KernelError> {
    with_session(false, |s| {
        keel_vfs::download::download_files(
            s,
            storage_id,
            sources,
            destination,
            preprocess_files,
            preprocess_cb,
            progress_cb,
            should_cancel,
        )
        .map(|_| ())
    })
}

/// Build one `FileProp` per input path. `object_id` is left 0 so keel-vfs resolves each
/// entry by path (`get_object_from_object_id_or_path` treats 0 as "resolve by
/// full_path").
fn to_file_props(files: &[String]) -> Vec<FileProp> {
    files
        .iter()
        .map(|f| FileProp {
            object_id: 0,
            full_path: f.clone(),
        })
        .collect()
}

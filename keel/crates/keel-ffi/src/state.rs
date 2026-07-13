//! Global device container + the `verifyMtpSession` / `_xxx` helper layer.
//!
//! This module ports two Go files at once:
//!   * `ferry/kernel/structs.go`'s `deviceContainer` (the process-global device +
//!     device-info + `locked` flag), and
//!   * `ferry/kernel/helpers.go`'s `_initialize` / `_fetchDeviceInfo` /
//!     `_fetchStorages` / `_makeDirectory` / `_fileExists` / `_deleteFile` /
//!     `_renameFile` / `_walk` / `_uploadFiles` / `_downloadFiles` / `_dispose` /
//!     `verifyMtpSession` / `lockMtp`, plus `the legacy kernel`'s `_sendFetchStorages`
//!     Samsung EOF workaround.
//!
//! The `abi.rs` exports map 1:1 to `the legacy kernel`'s exported functions; these `state`
//! functions map 1:1 to the `helpers.go` `_xxx` functions each export calls.
//!
//! Errors are returned as [`KernelError`] (defined in `errors.rs`, which owns the
//! `processError` port) — the keel analogue of Go's untyped `error` reaching
//! `send_to_js.processError`. `KernelError::Vfs` carries a typed vfs error (the
//! type-switch operand); `KernelError::Message` carries the bare-string sentinels
//! Go fabricates via `fmt.Errorf` (the two `verifyMtpSession` sentinels, the EOF
//! wrap, the unmarshal failures).
//!
//! ## Two locks, deliberately distinct
//!
//! Go had ONE global (`var container deviceContainer`) with NO real mutex — the
//! `locked` flag was a no-op (see [`lock_mtp`]). keel cannot share a mutable global
//! without a real lock, so [`CONTAINER`] is a genuine `Mutex` **purely for memory
//! safety** (plan §2 / keel-ffi/state). That mutex is orthogonal to Go's `locked`
//! flag: it serializes concurrent access to the container's memory, which Ferry
//! never actually triggers (it drives every export from one dispatch queue), so it
//! is observably invisible. The `locked`-flag no-op is reproduced separately by
//! [`lock_mtp`] so the port stays line-by-line faithful to `the legacy kernel`.

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

/// The `"ErrorMtpDetectFailed"` sentinel — Go returns this bare string whenever the
/// container has no device (verifyMtpSession, helpers.go:11-13). Factored so every
/// `c.dev` miss produces byte-identical text (and routes through `processError`'s
/// string-equality fallthrough to `ErrorMtpDetectFailed`).
fn detect_failed() -> KernelError {
    KernelError::Message("ErrorMtpDetectFailed".to_string())
}

// ---------------------------------------------------------------------------
// The global container
// ---------------------------------------------------------------------------

/// go-mtpx kernel `deviceContainer` (structs.go:13-17), minus the `locked` field
/// (see [`lock_mtp`] — it was a no-op, so keel omits the field and reproduces the
/// no-op directly).
struct DeviceContainer {
    /// Go's `container.dev *mtp.Device`.
    dev: Option<Device>,
    /// Go's `container.deviceInfo *mtp.DeviceInfo`. Present after a successful
    /// device-info fetch; its `serial_number` is what the device-change check
    /// compares against.
    device_info: Option<DeviceInfo>,
}

/// The single process-global device container. See the module doc for why this is
/// `Mutex`-wrapped even though Go used a bare global: memory safety only, not Go's
/// (no-op) `locked` flag.
static CONTAINER: Mutex<DeviceContainer> = Mutex::new(DeviceContainer {
    dev: None,
    device_info: None,
});

/// Lock the container, recovering from poisoning. A panic inside an export (caught
/// by abi.rs's `catch_unwind`) can poison this mutex; we recover the inner value so
/// the next export still works, matching Go's global staying usable after a
/// recovered error.
fn lock_container() -> MutexGuard<'static, DeviceContainer> {
    CONTAINER.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// lockMtp — the documented no-op
// ---------------------------------------------------------------------------

/// go-mtpx kernel `lockMtp` (helpers.go:191-203).
///
/// **Intentional no-op — a preserved Go bug (plan §3.5).** Go's `lockMtp` does:
///
/// ```go
/// if container.locked { return errors.New("ErrorMtpLockExists") }
/// container.locked = true
/// defer func() { container.locked = false }()   // runs at lockMtp's OWN return
/// return nil
/// ```
///
/// The `defer` fires when `lockMtp` returns — i.e. *before the caller does any
/// work* — so `container.locked` is reset to `false` the instant it is set. The
/// lock is therefore never actually held across an operation; `lockMtp` can only
/// ever observe `locked == false` and return `Ok`. Every export calls it first,
/// but the `ErrorMtpLockExists` path is dead code.
///
/// A *real* lock here would change observable behaviour (it would serialize or
/// reject concurrent exports), which the plan forbids. So keel reproduces the
/// no-op: always `Ok`. Container memory safety is handled separately by
/// [`CONTAINER`]'s mutex, not by this.
pub(crate) fn lock_mtp() -> Result<(), KernelError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// verifyMtpSession + dispose
// ---------------------------------------------------------------------------

/// go-mtpx kernel `verifyMtpSession` (helpers.go:10-33).
///
/// `skip_device_change_check` is Go's `verifyMtpSessionMode.skipDeviceChangeCheck`
/// — `true` only for `_initialize` / `_fetchDeviceInfo`, `false` for every other
/// op. When `false` (and a device info is on record), it re-fetches device info and
/// compares serials; a mismatch is `ErrorDeviceChanged`, and a fetch failure drops
/// the stored info, disposes the device (the Samsung usb-timeout mitigation), and
/// surfaces the underlying error.
fn verify_mtp_session(
    c: &mut DeviceContainer,
    skip_device_change_check: bool,
) -> Result<(), KernelError> {
    // helpers.go:11-13 — no device ⇒ ErrorMtpDetectFailed.
    if c.dev.is_none() {
        return Err(detect_failed());
    }

    // helpers.go:15 — the change check runs only when not skipped AND a prior
    // device info exists to compare against.
    if skip_device_change_check || c.device_info.is_none() {
        return Ok(());
    }

    // helpers.go:16 — re-fetch device info. Compute the result first so the `&mut`
    // borrow of `c.dev` ends before we mutate `c.device_info` / dispose below.
    let fetched = match c.dev.as_mut() {
        Some(d) => d.fetch_device_info(),
        None => return Err(detect_failed()),
    };

    match fetched {
        Ok(dinfo) => {
            // helpers.go:25 — serial compare.
            let changed = c
                .device_info
                .as_ref()
                .map(|stored| stored.serial_number != dinfo.serial_number)
                .unwrap_or(false);
            if changed {
                // helpers.go:26-28 — update the stored info, report the change.
                c.device_info = Some(dinfo);
                Err(KernelError::Message("ErrorDeviceChanged".to_string()))
            } else {
                Ok(())
            }
        }
        Err(e) => {
            // helpers.go:18-22 — drop stored info, dispose, surface the raw error.
            c.device_info = None;
            dispose(c);
            Err(KernelError::Vfs(e))
        }
    }
}

/// go-mtpx kernel `_dispose` (helpers.go:181-189): `mtpx.Dispose(container.dev)`.
///
/// DEVIATION (documented for the gate): Go's `_dispose` closed the device but left
/// the (now-disposed) `*mtp.Device` pointer in `container.dev`; only the `Dispose`
/// *export* nil'd it. keel's `Device::dispose` **consumes** the device (there is no
/// `&mut self` close-in-place), so keel `take()`s it out — leaving `c.dev == None`.
/// The only observable divergence is on a *subsequent* op issued after a
/// verify-time device-info failure: Go would hit the disposed handle and fail again
/// with a device error, whereas keel finds `None` and returns `ErrorMtpDetectFailed`.
/// Both still surface an error; only the `errorType` differs on that follow-up call.
fn dispose(c: &mut DeviceContainer) {
    if let Some(dev) = c.dev.take() {
        dev.dispose();
    }
}

// ---------------------------------------------------------------------------
// The verify-then-run harness for the path operations
// ---------------------------------------------------------------------------

/// Lock the container, run `verifyMtpSession`, then hand the live session to `op`.
/// Every path-level `_xxx` helper (make_directory / file_exists / delete_file /
/// rename_file / walk / upload / download) is this shape: verify, then a single
/// `mtpx.*` call whose error is wrapped `KernelError::Vfs`.
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

/// go-mtpx kernel `Initialize` body (legacy kernel L36-67 minus the `SendInitialize`):
/// `_initialize` then `_fetchDeviceInfo` then `GetUsbInfo`. Returns the device info
/// and USB descriptor for abi.rs to serialise.
pub(crate) fn initialize() -> Result<(DeviceInfo, UsbInfo), KernelError> {
    let mut c = lock_container();

    // _initialize (helpers.go:35-44): mtpx.Initialize → container.dev = d. (Any
    // prior device is dropped here; keel closes it on drop, where Go leaked it.)
    let dev = Device::initialize(Init { debug_mode: false }).map_err(KernelError::Vfs)?;
    c.dev = Some(dev);

    // _fetchDeviceInfo (helpers.go:46-67), skipDeviceChangeCheck = true.
    let dinfo = fetch_device_info_inner(&mut c)?;

    // legacy kernel L59 — container.dev.GetUsbInfo(). keel: the USB descriptor captured
    // at discovery lives on the session (device.rs installs it at configure).
    let usb = match c.dev.as_ref() {
        Some(d) => d.session().usb_info().clone(),
        None => return Err(detect_failed()),
    };
    Ok((dinfo, usb))
}

/// go-mtpx kernel `FetchDeviceInfo` body (legacy kernel L70-93 minus `SendDeviceInfo`):
/// `_fetchDeviceInfo` then `GetUsbInfo`.
pub(crate) fn fetch_device_info() -> Result<(DeviceInfo, UsbInfo), KernelError> {
    let mut c = lock_container();
    let dinfo = fetch_device_info_inner(&mut c)?;
    let usb = match c.dev.as_ref() {
        Some(d) => d.session().usb_info().clone(),
        None => return Err(detect_failed()),
    };
    Ok((dinfo, usb))
}

/// go-mtpx kernel `_fetchDeviceInfo` (helpers.go:46-67). Verify with
/// `skipDeviceChangeCheck = true`, fetch, and cache the result in the container
/// (or drop the cache on failure).
fn fetch_device_info_inner(c: &mut DeviceContainer) -> Result<DeviceInfo, KernelError> {
    verify_mtp_session(c, true)?;

    let fetched = match c.dev.as_mut() {
        Some(d) => d.fetch_device_info(),
        None => return Err(detect_failed()),
    };
    match fetched {
        Ok(dinfo) => {
            // helpers.go:64 — container.deviceInfo = dInfo.
            c.device_info = Some(dinfo.clone());
            Ok(dinfo)
        }
        Err(e) => {
            // helpers.go:58-61 — container.deviceInfo = nil, return err (no dispose).
            c.device_info = None;
            Err(KernelError::Vfs(e))
        }
    }
}

/// go-mtpx kernel `_fetchStorages` (helpers.go:69-80) wrapped in the
/// `_sendFetchStorages` Samsung EOF workaround (legacy kernel L109-128).
///
/// On any storage error, when both a device AND a device info are on record and the
/// error text contains `"EOF"`, Go rewrites it to `"error allow storage access. …"`
/// and disposes the device — a mitigation to stop Samsung phones spewing USB
/// timeouts once storage access has been denied. The rewritten message's
/// `"allow storage access"` substring is what `processError` keys on to classify it
/// `ErrorAllowStorageAccess`.
///
/// (Go's `_sendFetchStorages(retry bool, …)` takes a `retry` parameter that is
/// never referenced in the body — there is no actual retry loop to port. It is
/// noted here for completeness; keel drops the dead parameter.)
///
/// GATE NOTE: the `"EOF"` substring test depends on keel-usb / keel-mtp surfacing a
/// `"EOF"`-containing error string when a Samsung device denies storage access
/// (parity with Go's `io.EOF` bubbling up from the transport). If that transport
/// error renders differently, this workaround will not fire — flagged as an open
/// issue for the transport authors.
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
            // legacy kernel L113-120 — dev != nil && deviceInfo != nil && err contains "EOF".
            if c.dev.is_some() && c.device_info.is_some() && e.to_string().contains("EOF") {
                // legacy kernel L115 — the verbatim sentinel is built by errors.rs (which
                // owns its exact text, since processError keys on its substring).
                let msg = errors::allow_storage_access_sentinel(&e.to_string());
                // legacy kernel L118 — dispose to stop the Samsung usb-timeout storm.
                dispose(&mut c);
                Err(KernelError::Message(msg))
            } else {
                Err(KernelError::Vfs(e))
            }
        }
    }
}

/// go-mtpx kernel `Dispose` export body (legacy kernel L493-512 minus `SendDispose`):
/// `_dispose()`, then nil both container fields.
pub(crate) fn dispose_export() -> Result<(), KernelError> {
    let mut c = lock_container();
    // _dispose() — already clears c.dev (via take), matching legacy kernel L508.
    dispose(&mut c);
    // legacy kernel L509 — container.deviceInfo = nil.
    c.device_info = None;
    Ok(())
}

// ---------------------------------------------------------------------------
// Path operations (helpers.go _makeDirectory / _fileExists / _deleteFile /
// _renameFile / _walk / _uploadFiles / _downloadFiles)
// ---------------------------------------------------------------------------

/// Ferry extension (no legacy-kernel counterpart): the device-generated
/// thumbnail for an object, via MTP `GetThumb`. `Ok(None)` when the object has
/// none (folders, documents, unsupported) so the UI falls back to a glyph.
pub(crate) fn fetch_thumbnail(
    storage_id: u32,
    full_path: &str,
) -> Result<Option<Vec<u8>>, KernelError> {
    with_session(false, |s| keel_vfs::object::thumbnail(s, storage_id, full_path))
}

/// go-mtpx kernel `_makeDirectory` (helpers.go:82-93). The returned leaf object id
/// is discarded, exactly like Go (`_, err := mtpx.MakeDirectory(...)`).
pub(crate) fn make_directory(storage_id: u32, full_path: &str) -> Result<(), KernelError> {
    with_session(false, |s| {
        keel_vfs::dirops::make_directory(s, storage_id, full_path).map(|_| ())
    })
}

/// go-mtpx kernel `_fileExists` (helpers.go:95-106), plus the legacy kernel L179-184
/// `FileProp` construction (each input path → `FileProp{FullPath: f}`; ObjectId
/// left 0 so vfs resolves by path). Returns the per-input `exists` flags in order;
/// on the FileExists batch-abort quirk the vec is empty (fewer than the inputs),
/// which abi.rs then pairs against the inputs exactly as Go's `SendFileExists`
/// ranges the result and indexes the inputs.
pub(crate) fn file_exists(storage_id: u32, files: &[String]) -> Result<Vec<bool>, KernelError> {
    let fprops = to_file_props(files);
    with_session(false, |s| {
        let fc = keel_vfs::dirops::file_exists(s, storage_id, &fprops)?;
        Ok(fc.into_iter().map(|c| c.exists).collect())
    })
}

/// go-mtpx kernel `_deleteFile` (helpers.go:108-119) + the legacy kernel L216-221
/// `FileProp` construction.
pub(crate) fn delete_file(storage_id: u32, files: &[String]) -> Result<(), KernelError> {
    let fprops = to_file_props(files);
    with_session(false, |s| keel_vfs::dirops::delete_file(s, storage_id, &fprops))
}

/// go-mtpx kernel `_renameFile` (helpers.go:121-132) + the legacy kernel L253-255 single
/// `FileProp`. The returned (unchanged) object id is discarded, like Go.
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

/// go-mtpx kernel `_walk` (helpers.go:134-153). The callback simply accumulates the
/// walked `FileInfo`s into a vec — Go's `func(objectId, fi, err) { files = append(…) }`.
/// Go's always-nil `err` callback parameter is dropped (keel-vfs never passes one).
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

/// go-mtpx kernel `_uploadFiles` (helpers.go:155-166). The preprocess/progress
/// callbacks and the cancel seam are built by abi.rs (they drive the 500 ms
/// sampler); this just verifies the session and forwards to `mtpx.UploadFiles`.
/// The returned counts are discarded, like Go (`_, _, _, err := …`).
#[allow(clippy::too_many_arguments)] // 1:1 with the Go signature (helpers.go:155).
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

/// go-mtpx kernel `_downloadFiles` (helpers.go:168-179).
#[allow(clippy::too_many_arguments)] // 1:1 with the Go signature (helpers.go:168).
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

/// the legacy kernel's per-input `mtpx.FileProp{FullPath: f}` construction (legacy kernel L179-184
/// / 216-221). ObjectId is left 0 so keel-vfs resolves each entry by path (Go's
/// zero-value `ObjectId`; keel-vfs `get_object_from_object_id_or_path` treats 0 as
/// "resolve by full_path").
fn to_file_props(files: &[String]) -> Vec<FileProp> {
    files
        .iter()
        .map(|f| FileProp {
            object_id: 0,
            full_path: f.clone(),
        })
        .collect()
}

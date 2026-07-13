//! The 12 exported C symbols — the drop-in dylib ABI.
//!
//! The Swift `KeelLibrary` dlsym loop resolves these exact names, so the symbol set,
//! the parameter shapes, and the blocking-until-done-callback contract are fixed
//! (docs/CONTRACTS.md keel-ffi).
//!
//! ## Division of labour with `json.rs` / `errors.rs` (sibling modules)
//!
//! `json.rs` owns the whole JSON contract: the serde structs, the wire-compatible
//! serializer, the case-insensitive input decode ([`json::decode_input`]), AND the
//! outbound payload builders ([`json::initialize_json`], `walk_json`, `progress_json`,
//! `error_json`, …) — so the domain→wire mapping (the empty-slice `null` quirk, the
//! literal-`Z` `dateAdded`, lossy floats, elapsed-time) lives there and is
//! golden-fixture tested there. `errors.rs` owns error classification
//! ([`errors::process_error`]) and the [`errors::KernelError`] taxonomy `state`
//! returns and this file maps.
//!
//! This file is the orchestrator: read input → call `state` → hand the result to the
//! matching `json` builder → fire the callback.
//!
//! ## The callback ABI (the load-bearing pointer quirk)
//!
//! The C header declares the callback parameter as `on_cb_result_t*` — a *pointer to*
//! the `void(*)(char*)` typedef — but the Swift side passes the callback **function
//! pointer itself as that value**, and the callee casts the pointer straight to the
//! function type and calls it. keel models the parameter directly as a nullable
//! function pointer, [`OnCbResult`], and [`emit`] null-checks then calls it — **never
//! dereferences**.
//!
//! ## Blocking + threading
//!
//! Each export blocks its calling thread until the done callback has fired (Swift's
//! continuation model depends on it). Progress/preprocess callbacks fire from the
//! 500 ms sampler thread; the done/error callback fires from this (the export's)
//! thread. The terminal progress snapshot is also re-emitted from the export thread
//! after the sampler stops, so fast transfers cannot lose their final counters.
//!
//! ## Panic policy
//!
//! `panic = "abort"` is forbidden (plan §3.7); unwinding across the C ABI is UB.
//! Every export wraps its body in [`catch_unwind`] via [`guard`] and, on a caught
//! panic, emits an `ErrorGeneral` envelope through the done callback instead of
//! unwinding.

// The 11 `pub unsafe extern "C"` exports share one safety contract — the C caller
// must uphold the dylib ABI (each callback slot is null or a valid Swift callback
// fn pointer; the JSON input is null or a valid NUL-terminated C string) — which the
// module doc's "callback ABI" section documents in full. A per-export `# Safety`
// stanza would repeat that 11 times, so the lint is allowed module-wide here.
#![allow(clippy::missing_safety_doc)]

use std::any::Any;
use std::ffi::{CStr, c_char};
use std::fs::Metadata;
use std::panic::{AssertUnwindSafe, catch_unwind};

use keel_vfs::{FileInfo, ProgressInfo, VfsError};

use crate::cancel;
use crate::errors::{self, KernelError};
use crate::json;
use crate::sampler::{Sample, Sampler};
use crate::state;

/// The callback type. The `on_cb_result_t*` slot carries the Swift callback
/// **function pointer as its value** (see the module doc). `None` == a null pointer.
/// Function pointers are `Copy`, so this threads through by value.
pub(crate) type OnCbResult = Option<unsafe extern "C" fn(*mut c_char)>;

// ===========================================================================
// Exports, in source order
// ===========================================================================

/// Open the device and emit its info + USB descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Initialize(on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        match state::initialize() {
            Ok((dinfo, usb)) => send(on_done, &json::initialize_json(&dinfo, &usb)),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Re-fetch the device info and emit it with the USB descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FetchDeviceInfo(on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        match state::fetch_device_info() {
            Ok((dinfo, usb)) => send(on_done, &json::device_info_json(&dinfo, &usb)),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Fetch the device's storages, including the Samsung EOF workaround (in
/// [`state::fetch_storages`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FetchStorages(on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        match state::fetch_storages() {
            Ok(storages) => send(on_done, &json::storages_json(&storages)),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Create a directory at the requested path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn MakeDirectory(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::MakeDirectoryInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "MakeDirectory", &e),
        };
        match state::make_directory(parsed.storage_id, &parsed.full_path) {
            Ok(()) => send(on_done, &json::make_directory_json()),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Test each input path for existence.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FileExists(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::FileExistsInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "FileExists", &e),
        };
        match state::file_exists(parsed.storage_id, &parsed.files) {
            // json.rs pairs each input path with its exists flag positionally and
            // does the empty-slice `null` handling.
            Ok(exists) => send(on_done, &json::file_exists_json(&exists, &parsed.files)),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Fetch a device thumbnail via MTP GetThumb. `data` is a base64 string, or `null`
/// when the object has none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FetchThumbnail(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::FetchThumbnailInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "FetchThumbnail", &e),
        };
        match state::fetch_thumbnail(parsed.storage_id, &parsed.full_path) {
            Ok(bytes) => send(on_done, &json::thumbnail_json(bytes.as_deref())),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Delete each input path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DeleteFile(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::DeleteFileInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "DeleteFile", &e),
        };
        match state::delete_file(parsed.storage_id, &parsed.files) {
            Ok(()) => send(on_done, &json::delete_file_json()),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Rename an object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RenameFile(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::RenameFileInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "RenameFile", &e),
        };
        match state::rename_file(parsed.storage_id, &parsed.full_path, &parsed.new_file_name) {
            Ok(()) => send(on_done, &json::rename_file_json()),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Walk a directory tree and emit the collected file infos.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Walk(input: *mut c_char, on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::WalkInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "Walk", &e),
        };
        match state::walk(
            parsed.storage_id,
            &parsed.full_path,
            parsed.recursive,
            parsed.skip_disallowed_files,
            parsed.skip_hidden_files,
        ) {
            Ok(files) => send(on_done, &json::walk_json(&files)),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Streaming walk (Ferry extension): identical inputs to `Walk`, but entries are
/// delivered to `on_batch` in chunks as they are read, so a large directory
/// paints progressively instead of blocking on one final payload. `on_batch`
/// receives a JSON array of the same entry shape `Walk` returns; `on_done` fires
/// once at the end (`data: true` on success, or an error envelope). MTP is
/// serial, so this doesn't read any faster — it just surfaces results as they
/// arrive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn WalkStream(
    input: *mut c_char,
    on_batch: OnCbResult,
    on_done: OnCbResult,
) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::WalkInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "WalkStream", &e),
        };

        // Deliver ~32 entries per callback: small enough that the first tiles
        // appear almost immediately, large enough to keep callback overhead low.
        const BATCH: usize = 32;
        let mut batch: Vec<keel_vfs::FileInfo> = Vec::with_capacity(BATCH);
        let emit = |b: &[keel_vfs::FileInfo]| {
            if !b.is_empty() {
                send(on_batch, &json::walk_json(b));
            }
        };

        let result = {
            let mut on_entry = |fi: &keel_vfs::FileInfo| {
                batch.push(fi.clone());
                if batch.len() >= BATCH {
                    emit(&batch);
                    batch.clear();
                }
            };
            state::walk_streaming(
                parsed.storage_id,
                &parsed.full_path,
                parsed.recursive,
                parsed.skip_disallowed_files,
                parsed.skip_hidden_files,
                &mut on_entry,
            )
        };
        emit(&batch); // flush the trailing partial batch

        // Fires `on_done` exactly once. Nothing fallible may run after this
        // match: a panic past here would re-fire `on_done` via `guard`.
        match result {
            Ok(()) => send(on_done, &json::walk_done_json()),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// Upload files to the device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn UploadFiles(
    input: *mut c_char,
    on_preprocess: OnCbResult,
    on_progress: OnCbResult,
    on_done: OnCbResult,
) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::UploadFilesInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "UploadFiles", &e),
        };

        // Spawn the 500 ms sampler and clear the cancel flag.
        let sampler = Sampler::start(on_preprocess, on_progress);
        cancel::reset();

        // Run the transfer. The callbacks only WRITE the sampler slot; the sampler
        // thread does the marshalling + emitting. Scoped so their borrows of
        // `sampler` end before we `stop()` (which moves it).
        let res = {
            let mut preprocess_cb = |meta: &Metadata, full_path: &str| -> Result<(), VfsError> {
                sampler.set(Sample::UploadPreprocess {
                    full_path: full_path.to_string(),
                    // The preprocess payload needs the base name; Metadata carries no
                    // name, so derive it from the path.
                    name: base_name(full_path),
                    size: meta.len() as i64,
                });
                Ok(())
            };
            let mut progress_cb = |p: &ProgressInfo| -> Result<(), VfsError> {
                sampler.set(Sample::Progress(Box::new(p.clone())));
                Ok(())
            };
            let should_cancel = || cancel::is_cancelled();

            state::upload_files(
                parsed.storage_id,
                &parsed.sources,
                &parsed.destination,
                parsed.preprocess_files,
                &mut preprocess_cb,
                &mut progress_cb,
                &should_cancel,
            )
        };

        finish_transfer(sampler, on_preprocess, on_progress, on_done, res);
    });
}

/// Download files from the device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DownloadFiles(
    input: *mut c_char,
    on_preprocess: OnCbResult,
    on_progress: OnCbResult,
    on_done: OnCbResult,
) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        let raw = unsafe { read_input(input) };
        let parsed: json::DownloadFilesInput = match json::decode_input(&raw) {
            Ok(v) => v,
            Err(e) => return unmarshal_error(on_done, "DownloadFiles", &e),
        };

        let sampler = Sampler::start(on_preprocess, on_progress);
        cancel::reset();

        let res = {
            let mut preprocess_cb = |fi: &FileInfo| -> Result<(), VfsError> {
                // The preprocess payload reads straight off the FileInfo.
                sampler.set(Sample::DownloadPreprocess {
                    full_path: fi.full_path.clone(),
                    name: fi.name.clone(),
                    size: fi.size,
                });
                Ok(())
            };
            let mut progress_cb = |p: &ProgressInfo| -> Result<(), VfsError> {
                sampler.set(Sample::Progress(Box::new(p.clone())));
                Ok(())
            };
            let should_cancel = || cancel::is_cancelled();

            state::download_files(
                parsed.storage_id,
                &parsed.sources,
                &parsed.destination,
                parsed.preprocess_files,
                &mut preprocess_cb,
                &mut progress_cb,
                &should_cancel,
            )
        };

        finish_transfer(sampler, on_preprocess, on_progress, on_done, res);
    });
}

/// Close the device and clear the container.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Dispose(on_done: OnCbResult) {
    guard(on_done, || {
        if let Err(e) = state::lock_mtp() {
            emit_error(on_done, &e);
            return;
        }
        match state::dispose_export() {
            Ok(()) => send(on_done, &json::dispose_json()),
            Err(e) => emit_error(on_done, &e),
        }
    });
}

/// The success/error tail shared by Upload/DownloadFiles. Success stops the poller,
/// re-emits its latest progress snapshot, then fires done; error fires done BEFORE
/// stopping the poller (so one stale progress event may follow — Swift tolerates it).
/// See the sampler module doc for the ≤ 500 ms cadence details.
fn finish_transfer(
    sampler: Sampler,
    on_preprocess: OnCbResult,
    on_progress: OnCbResult,
    on_done: OnCbResult,
    res: Result<(), KernelError>,
) {
    match res {
        Ok(()) => {
            sampler.stop_and_emit_latest(on_preprocess, on_progress);
            send(on_done, &json::transfer_done_json());
        }
        Err(e) => {
            emit_error(on_done, &e);
            // INVARIANT (audit A3.4): nothing fallible may run after the `done` fire
            // above. `guard` re-fires `done` on any escaping panic, so a panic here
            // would double-fire `done`. `sampler.stop()` is infallible — an atomic
            // store plus a `join` whose `Err` is swallowed (sampler.rs) — so the
            // invariant holds; preserve it if this arm ever grows.
            sampler.stop();
        }
    }
}

// ===========================================================================
// Callback / panic / input plumbing
// ===========================================================================

/// Run an export body under `catch_unwind`; on a caught panic emit an
/// `ErrorGeneral` envelope instead of unwinding across the C ABI (plan §3.7).
///
/// `AssertUnwindSafe` is justified: the only shared state touched is the container
/// mutex, which recovers from poisoning (state.rs), and the callback pointer is
/// `Copy`. The panic path is ALWAYS `ErrorGeneral` (not routed through
/// `process_error`), so a panic message that happens to contain a magic substring
/// can't be reclassified.
fn guard<F: FnOnce()>(on_done: OnCbResult, body: F) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(body)) {
        // Build + fire the ErrorGeneral envelope for the caught panic. This recovery
        // emit must ALSO never unwind across the C ABI (plan §3.7): the in-body
        // `send`s are inside the `catch_unwind` above, but this one is not, so if it
        // panics — most reachably a foreign `on_done` callback that itself unwinds
        // into `emit` — a second unwind would escape the export = UB (audit A3.2).
        // Contain it with a second `catch_unwind`; on a double panic, `abort` rather
        // than let the unwind cross the boundary.
        let recovered = catch_unwind(AssertUnwindSafe(|| {
            send(
                on_done,
                &json::error_json(errors::error_type::GENERAL, panic_message(payload)),
            );
        }));
        if recovered.is_err() {
            std::process::abort();
        }
    }
}

/// Best-effort extraction of a panic's message (the `&str` / `String` a `panic!`
/// carries), for the `ErrorGeneral` envelope. Unknown payloads degrade to a fixed
/// string.
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "internal error".to_string()
    }
}

/// Map a [`KernelError`] to an envelope and fire it: classify the error →
/// `{errorType, error, data:null}` → callback.
fn emit_error(cb: OnCbResult, err: &KernelError) {
    let (error_type, error_msg) = errors::process_error(err);
    send(cb, &json::error_json(error_type, error_msg));
}

/// Emit the JSON-unmarshal failure envelope. The verbatim message text (including the
/// `"occured"` typo required by the wire contract) lives in
/// `json::unmarshalling_error`; it classifies as `ErrorGeneral`.
fn unmarshal_error(cb: OnCbResult, op: &str, err: &serde_json::Error) {
    emit_error(cb, &KernelError::Message(json::unmarshalling_error(op, err)));
}

/// Safe wrapper over [`emit`] so the export bodies read cleanly.
fn send(cb: OnCbResult, payload: &str) {
    // SAFETY: `emit` null-checks the pointer and passes a C-allocated,
    // NUL-terminated copy of `payload`; nothing here dereferences `cb`.
    unsafe { emit(cb, payload) };
}

/// The one true callback invocation. Null-checks the function pointer, copies
/// `payload` into a `libc::malloc` buffer (so Swift's `free()` matches the C
/// allocator), and calls it.
///
/// A Rust-allocated `CString` would be freed by Swift with a mismatched allocator, so
/// the malloc copy is load-bearing (plan §3.6). Valid JSON never contains an interior
/// NUL, so the copy is a well-formed C string.
///
/// # Safety
/// The caller must pass a `payload` that outlives this call (a `&str` does) and an
/// `OnCbResult` that is either null or a valid Swift callback function pointer.
pub(crate) unsafe fn emit(cb: OnCbResult, payload: &str) {
    // Fire only on a non-null pointer. Never dereference: the value IS the function
    // pointer.
    let Some(callback) = cb else { return };

    let bytes = payload.as_bytes();
    let len = bytes.len();
    let buf = unsafe { libc::malloc(len + 1) } as *mut u8;
    if buf.is_null() {
        // OOM: we cannot copy the payload into a C buffer, but we MUST still fire the
        // callback. Every export blocks until its `done` callback fires; a bare early
        // return here would leave `done` un-fired and hang Swift's continuation forever
        // (audit A3.1). Swift's `consume` (KeelFFI.swift:85-91) guards a NULL pointer —
        // it decodes NULL as `""` and does NOT `free()` it — so passing NULL safely
        // unblocks the continuation (Swift then treats the empty payload as a decode
        // failure). Firing NULL keeps block-until-done intact rather than crashing.
        unsafe { callback(std::ptr::null_mut()) };
        return;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, len);
        *buf.add(len) = 0;
        callback(buf as *mut c_char);
    }
}

/// Copy a borrowed C-string input into an owned `String` (borrow, copy before use). A
/// null pointer yields `""`.
///
/// # Safety
/// `ptr` must be null or a valid NUL-terminated C string, as guaranteed by the ABI
/// contract.
unsafe fn read_input(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // to_string_lossy replaces invalid UTF-8; input is expected to be UTF-8.
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

/// The base name of a path, used for the upload preprocess payload.
fn base_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

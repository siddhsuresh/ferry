//! `CancelTransfer` — the cooperative cancellation flag.
//!
//! Faithful port of the cancellation machinery in `ferry/kernel/the legacy kernel`
//! (lines 22-33, 348, 356, 372, 446, 454, 470). Go used a package-global
//! `atomic.Bool` polled from inside the Upload/DownloadFiles preprocess and
//! progress callbacks; keel keeps the exact same single-flag design.
//!
//! The wiring differs only in *where* the poll happens. Go polled the flag
//! *inside* the FFI callbacks it handed to go-mtpx (which itself had no cancel
//! concept). keel-vfs's `upload_files`/`download_files` instead take a
//! `should_cancel: &dyn Fn() -> bool` seam and do the poll themselves (see
//! upload.rs / download.rs module docs), returning `VfsError::Cancelled` on a
//! fire. keel-ffi feeds [`is_cancelled`] into that seam, so the observable
//! behaviour is identical: a set flag aborts the in-flight transfer with an
//! `ErrorTransferCancelled` envelope.

use std::sync::atomic::{AtomicBool, Ordering};

/// Go: `var transferCancelled atomic.Bool` (legacy kernel L26).
///
/// Set by [`CancelTransfer`] (called from any thread while an Upload/DownloadFiles
/// export is still blocking the FFI queue on another thread), polled by
/// [`is_cancelled`] via keel-vfs's `should_cancel` seam, and cleared by [`reset`]
/// at the start of each transfer.
static TRANSFER_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Go `CancelTransfer` (legacy kernel L30-33): flag the in-flight transfer for
/// cancellation. Safe to call from any thread; a no-op when nothing is
/// transferring (the flag is simply set and cleared again at the next transfer's
/// start).
///
/// This is the 12th exported symbol the Swift `KeelLibrary` dlsym loop resolves
/// (docs/CONTRACTS.md keel-ffi). It takes no pointers and only stores an atomic,
/// so it never panics and needs no `catch_unwind` guard (Go's export was equally
/// trivial).
///
/// `SeqCst` matches Go's `atomic.Bool`, which is sequentially consistent.
#[unsafe(no_mangle)]
pub extern "C" fn CancelTransfer() {
    TRANSFER_CANCELLED.store(true, Ordering::SeqCst);
}

/// Clear the flag at the start of a transfer. Go: `transferCancelled.Store(false)`
/// immediately before `_uploadFiles`/`_downloadFiles` (legacy kernel L348 / 446).
pub(crate) fn reset() {
    TRANSFER_CANCELLED.store(false, Ordering::SeqCst);
}

/// Poll the flag. Wired into keel-vfs's `should_cancel` seam; mirrors Go's
/// `transferCancelled.Load()` inside the preprocess/progress callbacks
/// (legacy kernel L356 / 372 / 454 / 470).
pub(crate) fn is_cancelled() -> bool {
    TRANSFER_CANCELLED.load(Ordering::SeqCst)
}

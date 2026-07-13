//! `CancelTransfer` — the cooperative cancellation flag.
//!
//! A single process-global atomic flag, polled during Upload/DownloadFiles transfers.
//!
//! keel-vfs's `upload_files`/`download_files` take a `should_cancel: &dyn Fn() -> bool`
//! seam and poll it themselves (see upload.rs / download.rs module docs), returning
//! `VfsError::Cancelled` on a fire. keel-ffi feeds [`is_cancelled`] into that seam, so
//! a set flag aborts the in-flight transfer with an `ErrorTransferCancelled` envelope.

use std::sync::atomic::{AtomicBool, Ordering};

/// The in-flight transfer's cancellation flag.
///
/// Set by [`CancelTransfer`] (called from any thread while an Upload/DownloadFiles
/// export is still blocking the FFI queue on another thread), polled by
/// [`is_cancelled`] via keel-vfs's `should_cancel` seam, and cleared by [`reset`]
/// at the start of each transfer.
static TRANSFER_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Flag the in-flight transfer for cancellation. Safe to call from any thread; a
/// no-op when nothing is transferring (the flag is simply set and cleared again at
/// the next transfer's start).
///
/// This is the 12th exported symbol the Swift `KeelLibrary` dlsym loop resolves
/// (docs/CONTRACTS.md keel-ffi). It takes no pointers and only stores an atomic, so it
/// never panics and needs no `catch_unwind` guard.
///
/// `SeqCst` gives the flag sequentially-consistent ordering.
#[unsafe(no_mangle)]
pub extern "C" fn CancelTransfer() {
    TRANSFER_CANCELLED.store(true, Ordering::SeqCst);
}

/// Clear the flag at the start of a transfer, immediately before the upload/download
/// begins.
pub(crate) fn reset() {
    TRANSFER_CANCELLED.store(false, Ordering::SeqCst);
}

/// Poll the flag. Wired into keel-vfs's `should_cancel` seam, checked from the
/// preprocess/progress callbacks during a transfer.
pub(crate) fn is_cancelled() -> bool {
    TRANSFER_CANCELLED.load(Ordering::SeqCst)
}

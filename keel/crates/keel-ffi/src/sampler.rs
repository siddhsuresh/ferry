//! The 500 ms latest-value progress sampler.
//!
//! Faithful port of the sampling goroutine in `ferry/kernel/the legacy kernel`
//! `UploadFiles` (319-346) and `DownloadFiles` (419-444). Both exports spawn a
//! goroutine that, every 500 ms, reads a single shared slot (`pInterface`) and, if
//! it holds an event, marshals + fires it through the appropriate callback. The
//! transfer's own preprocess/progress callbacks only ever *write* that slot
//! (overwriting the previous value); the goroutine samples it.
//!
//! The load-bearing properties, all preserved:
//!   * **latest-value sampling** — one slot shared by preprocess AND progress;
//!     intermediate events that land between two ticks are silently dropped;
//!   * **re-send on every tick** — the slot is never cleared, so if no newer event
//!     arrives, the same one is re-emitted each tick (Go never nil'd `pInterface`);
//!   * **success = stop-poller-then-final-progress-then-done** — Go's `ch <- true`
//!     (which blocks until the goroutine stops) precedes `SendTransferFilesDone`.
//!     keel additionally re-emits the latest progress snapshot after the join so
//!     a sub-500 ms transfer still exposes its terminal counters;
//!   * **error = done-then-stop** — Go's `SendError` precedes `ch <- true`, so one
//!     stale progress event may still slip out after the error; Swift tolerates it
//!     (legacy kernel L383-387).
//!
//! keel maps Go's unbuffered stop-channel to an `AtomicBool` + `JoinHandle`:
//! [`Sampler::stop`] sets the flag and joins (the "blocks until stopped" guarantee),
//! and [`Drop`] does the same so a panic unwinding through a transfer (abi.rs's
//! `catch_unwind`) never orphans the poller thread — a case Go never had to handle
//! because a Go panic would simply crash the process.
//!
//! One deliberate difference from Go: Go's goroutine read the *live* `*ProgressInfo`
//! pointer at tick time (a benign data race — it could observe a half-updated
//! struct while the transfer thread mutated it). keel instead stores a **clone**
//! per write, so the poller always reads a consistent snapshot. The sampled cadence
//! and values are equivalent; keel just avoids the race.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(test)]
use std::ffi::CString;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use keel_vfs::ProgressInfo;

use crate::abi::{OnCbResult, emit};
use crate::json;

/// Go: `time.Sleep(time.Millisecond * 500)` (legacy kernel L343 / 441).
const TICK: Duration = Duration::from_millis(500);

/// The single shared slot's payload — Go's `pInterface interface{}`, which held one
/// of `UploadPreprocessContainer`, `DownloadPreprocessContainer`, or
/// `ProgressContainer` (the legacy kernel structs.go:62-73). Cloned into the slot on every
/// write so the poller reads a consistent snapshot (see the module doc).
#[derive(Clone)]
pub(crate) enum Sample {
    /// Go `UploadPreprocessContainer` (legacy kernel L331-333 → SendUploadFilesPreprocess).
    /// keel-vfs's upload preprocess callback yields `(&Metadata, full_path)`; abi.rs
    /// derives `name` (the path base) and `size` here.
    UploadPreprocess {
        full_path: String,
        name: String,
        size: i64,
    },
    /// Go `DownloadPreprocessContainer` (legacy kernel L430-431 → SendDownloadFilesPreprocess).
    DownloadPreprocess {
        full_path: String,
        name: String,
        size: i64,
    },
    /// Go `ProgressContainer` (legacy kernel L335-336 / 433-434 → SendTransferFilesProgress).
    /// Boxed so this (much larger) variant doesn't bloat every slot write; a progress
    /// tick already clones the `ProgressInfo`, so the extra box allocation is noise
    /// against the USB-bound transfer.
    Progress(Box<ProgressInfo>),
}

/// The running poller: the shared slot, the stop flag, and the join handle. Created
/// by [`start`](Sampler::start) at the top of an Upload/DownloadFiles export and torn
/// down by [`stop`](Sampler::stop) (or [`Drop`] on the panic path).
pub(crate) struct Sampler {
    slot: Arc<Mutex<Option<Sample>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Spawn the 500 ms poller. `on_preprocess` / `on_progress` are the Swift
    /// callback function pointers (function pointers are `Send + Copy`, so moving
    /// them into the thread is sound). The thread runs until [`stop`](Self::stop),
    /// which the export always calls before returning — so the poller never outlives
    /// the transfer, and the callback pointers stay valid for its whole life.
    pub(crate) fn start(on_preprocess: OnCbResult, on_progress: OnCbResult) -> Sampler {
        let slot: Arc<Mutex<Option<Sample>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        let slot_thread = Arc::clone(&slot);
        let stop_thread = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            // Go's `for { select { case <-ch: return; default: … } }` — the stop
            // signal is checked FIRST each iteration (legacy kernel L323-345).
            loop {
                if stop_thread.load(Ordering::SeqCst) {
                    break;
                }

                // Snapshot the latest sample OUT of the lock, so the foreign
                // callback is never invoked while holding the slot mutex.
                let current: Option<Sample> = {
                    let guard = slot_thread.lock().unwrap_or_else(PoisonError::into_inner);
                    (*guard).clone()
                };
                if let Some(sample) = current {
                    emit_sample(&sample, on_preprocess, on_progress);
                }

                thread::sleep(TICK);
            }
        });

        Sampler {
            slot,
            stop,
            handle: Some(handle),
        }
    }

    /// Write the latest event into the slot (overwriting any unsent one). Called by
    /// abi.rs's transfer callbacks — Go's `pInterface = …` assignment.
    pub(crate) fn set(&self, sample: Sample) {
        let mut guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(sample);
    }

    /// Stop the poller and wait for it to finish. This is Go's `ch <- true` on an
    /// unbuffered channel — it blocks until the goroutine has stopped. The join
    /// waits at most one `TICK` (the current sleep) to elapse, which is the ≤ 500 ms
    /// "done may lag" window on the success path.
    pub(crate) fn stop(mut self) {
        self.stop_and_join();
    }

    /// Stop the sampler and deliver the latest progress snapshot once more.
    ///
    /// A fast transfer can finish before the first 500 ms sampling tick. The
    /// transfer code still writes its final `Completed` progress value into
    /// the slot, so emit that snapshot synchronously before the done callback
    /// to make terminal counts observable to callers.
    pub(crate) fn stop_and_emit_latest(
        mut self,
        on_preprocess: OnCbResult,
        on_progress: OnCbResult,
    ) {
        self.stop_and_join();
        let latest = {
            let guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            (*guard).clone()
        };
        if let Some(Sample::Progress(progress)) = latest {
            emit_sample(
                &Sample::Progress(progress),
                on_preprocess,
                on_progress,
            );
        }
    }

    /// Idempotent stop+join, shared by [`stop`](Self::stop) and [`Drop`].
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // A panicking poller thread (e.g. a JSON build panic) would make `join`
            // return `Err`; ignore it — we are tearing down regardless.
            let _ = handle.join();
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        // Reached on the panic/unwind path (abi.rs's `catch_unwind`), where `stop`
        // was never called. Ensures the poller thread is stopped and joined before
        // the export returns, so it can't fire a callback into freed Swift state.
        self.stop_and_join();
    }
}

/// Marshal one sample and fire it through the matching callback — Go's `switch v :=
/// pInterface.(type)` dispatch (legacy kernel L331-340 / 429-438). Upload/Download
/// preprocess samples go through `on_preprocess`; progress samples through
/// `on_progress`.
fn emit_sample(sample: &Sample, on_preprocess: OnCbResult, on_progress: OnCbResult) {
    // The domain→wire mapping (elapsed-time, the shared preprocess shape, float
    // formatting) lives in json.rs's Send* builders; the sampler just picks the
    // matching builder + callback. SAFETY on every `emit`: it null-checks the
    // pointer and passes a C-allocated NUL-terminated copy; see abi::emit.
    match sample {
        Sample::UploadPreprocess {
            full_path,
            name,
            size,
        } => {
            let payload = json::upload_preprocess_json(full_path, name, *size);
            unsafe { emit(on_preprocess, &payload) };
        }
        Sample::DownloadPreprocess {
            full_path,
            name,
            size,
        } => {
            let payload = json::download_preprocess_json(full_path, name, *size);
            unsafe { emit(on_preprocess, &payload) };
        }
        Sample::Progress(p) => {
            let payload = json::progress_json(p);
            unsafe { emit(on_progress, &payload) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_vfs::ProgressInfo;

    static PROGRESS_CALLBACKS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn count_progress(payload: *mut std::ffi::c_char) {
        PROGRESS_CALLBACKS.fetch_add(1, Ordering::SeqCst);
        if !payload.is_null() {
            unsafe { drop(CString::from_raw(payload)); }
        }
    }

    #[test]
    fn fast_transfer_gets_terminal_progress_before_done() {
        PROGRESS_CALLBACKS.store(0, Ordering::SeqCst);
        let sampler = Sampler::start(None, Some(count_progress));
        sampler.set(Sample::Progress(Box::new(ProgressInfo::new())));
        sampler.stop_and_emit_latest(None, Some(count_progress));

        assert!(
            PROGRESS_CALLBACKS.load(Ordering::SeqCst) >= 1,
            "the final progress snapshot must be delivered even before the first sampler tick"
        );
    }
}

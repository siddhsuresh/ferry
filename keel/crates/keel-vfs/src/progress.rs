//! Transfer progress model: the `SizeInfo` / `ProgressInfo` / `TransferStatus`
//! types plus the two progress arithmetic helpers (`percent`, `transfer_rate`).
//!
//! These types are what `upload_files` / `download_files` mutate in place and
//! hand (by shared reference) to the caller's progress callback on every tick.
//! keel-ffi's 500 ms sampler snapshots the latest one and serialises it, so the
//! field set and the exact arithmetic (percent-of-zero == 0, speed rounded to
//! 2 dp) are load-bearing for the wire contract.
//!
//! Naming: `SizeInfo` is exported under that name to match the crate's `lib.rs`
//! re-export (`pub use progress::{ProgressInfo, SizeInfo, TransferStatus}`).

use std::time::SystemTime;

use crate::object::FileInfo;

/// The transfer status carried in each progress tick. [`Self::as_str`] gives the
/// wire spelling keel-ffi serialises (`"InProgress"` / `"Completed"`), which the
/// wire contract fixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    /// Emitted on every per-chunk tick while a file is transferring.
    InProgress,
    /// Emitted once, as the final progress tick, after every file is done:
    /// `upload_files`/`download_files` set `status = Completed` and call the
    /// progress callback one last time.
    Completed,
}

impl TransferStatus {
    /// The exact wire spelling for each status; keel-ffi's JSON must reproduce
    /// these.
    pub fn as_str(self) -> &'static str {
        match self {
            TransferStatus::InProgress => "InProgress",
            TransferStatus::Completed => "Completed",
        }
    }
}

/// Size accounting for either the currently-active file or the whole bulk
/// session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeInfo {
    /// Total size to transfer. **0 when pre-processing was not requested** — that
    /// path never computes a total, so the bulk `total` stays 0 and every bulk
    /// percentage is `percent(x, 0) == 0`.
    pub total: i64,
    /// Total size transferred so far.
    pub sent: i64,
    /// Progress as a percentage in `[0, 100]` (`percent(sent, total)`).
    pub progress: f32,
}

/// One mutable instance lives for the whole `upload_files`/`download_files` call;
/// it is mutated in place and passed by reference to the caller's progress
/// callback each tick.
#[derive(Clone, Debug)]
pub struct ProgressInfo {
    /// The file currently being transferred. Owned and default-constructed until
    /// the first file starts.
    pub file_info: FileInfo,

    /// Wall-clock time the transfer session started.
    pub start_time: SystemTime,

    /// Wall-clock time of the most recent tick. Reset to "now" *after* each
    /// progress callback returns, so it feeds the next tick's [`transfer_rate`] as
    /// the interval start.
    pub latest_sent_time: SystemTime,

    /// Instantaneous transfer rate for the last chunk (see [`transfer_rate`]).
    pub speed: f64,

    /// Total files to transfer. **0 when pre-processing was not requested.**
    pub total_files: i64,

    /// Total directories to transfer. **0 when pre-processing was not requested.**
    pub total_directories: i64,

    /// Files fully transferred so far. Updated *after* each file completes, so
    /// during a file's own ticks this still reflects the count *before* it — an
    /// off-by-one the wire contract depends on.
    pub files_sent: i64,

    /// `percent(files_sent, total_files)`. `0` for the whole run when
    /// pre-processing was off (`total_files == 0`).
    pub files_sent_progress: f32,

    /// Size accounting for the active file.
    pub active_file_size: SizeInfo,

    /// Size accounting for the whole bulk session.
    pub bulk_file_size: SizeInfo,

    /// `InProgress` for every chunk tick; `Completed` for the final tick.
    pub status: TransferStatus,
}

impl ProgressInfo {
    /// The initial `ProgressInfo` built at the top of `upload_files` /
    /// `download_files`: zeroed counters, `start_time`/`latest_sent_time` = now,
    /// empty `FileInfo`, `status = InProgress`.
    pub fn new() -> Self {
        let now = SystemTime::now();
        ProgressInfo {
            file_info: FileInfo::default(),
            start_time: now,
            latest_sent_time: now,
            speed: 0.0,
            total_files: 0,
            total_directories: 0,
            files_sent: 0,
            files_sent_progress: 0.0,
            active_file_size: SizeInfo::default(),
            bulk_file_size: SizeInfo::default(),
            status: TransferStatus::InProgress,
        }
    }
}

impl Default for ProgressInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Percentage of `partial` over `total`, with a **`total <= 0 => 0`** guard —
/// that guard is why every bulk percentage is `0` when pre-processing is disabled
/// (`total == 0`).
pub fn percent(partial: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    (partial / total) * 100.0
}

/// Instantaneous rate for one chunk: `bytes / elapsed_ns * 1000`, rounded to two
/// decimals, `0` when no time has elapsed.
///
/// The interval is measured on the wall clock via [`SystemTime::duration_since`],
/// treating a backwards clock (Err) as "no time elapsed" (the `elapsed <= 0 => 0`
/// outcome). Speed is a behavioural-compatible-only field — the conformance
/// harness normalises it — so the wall-clock measurement is not a parity break.
pub fn transfer_rate(size: i64, last_sent_time: SystemTime) -> f64 {
    let elapsed_ns: i128 = SystemTime::now()
        .duration_since(last_sent_time)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);
    if elapsed_ns <= 0 {
        return 0.0;
    }
    let rate = size as f64 / elapsed_ns as f64 * 1000.0;
    (rate * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_of_zero_total_is_zero() {
        // The load-bearing guard: bulk percentages when preprocessing is off.
        assert_eq!(percent(5.0, 0.0), 0.0);
        assert_eq!(percent(0.0, 0.0), 0.0);
        assert_eq!(percent(10.0, -1.0), 0.0);
    }

    #[test]
    fn percent_normal() {
        assert_eq!(percent(1.0, 4.0), 25.0);
        assert_eq!(percent(4.0, 4.0), 100.0);
    }

    #[test]
    fn transfer_rate_zero_when_no_time_elapsed() {
        // A time in the future => duration_since Err => 0 (elapsed <= 0).
        let future = SystemTime::now() + std::time::Duration::from_secs(3600);
        assert_eq!(transfer_rate(1_000_000, future), 0.0);
    }

    #[test]
    fn transfer_rate_rounds_to_two_decimals() {
        let past = SystemTime::now() - std::time::Duration::from_millis(10);
        let r = transfer_rate(1234, past);
        // Rounded to 2 dp: the value times 100 must be integral.
        assert_eq!((r * 100.0).fract(), 0.0);
    }

    #[test]
    fn status_wire_spellings() {
        assert_eq!(TransferStatus::InProgress.as_str(), "InProgress");
        assert_eq!(TransferStatus::Completed.as_str(), "Completed");
    }
}

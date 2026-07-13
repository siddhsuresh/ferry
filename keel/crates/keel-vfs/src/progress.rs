//! Transfer progress model — a faithful port of go-mtpx `structs.go`
//! (`TransferSizeInfo`, `ProgressInfo`), `enums.go` (`TransferStatus`) and the
//! two progress arithmetic helpers from `utils.go` (`Percent`, `transferRate`).
//!
//! These types are what `upload_files` / `download_files` mutate in place and
//! hand (by shared reference) to the caller's progress callback on every tick.
//! keel-ffi's 500 ms sampler snapshots the latest one and serialises it, so the
//! field set and the exact arithmetic (percent-of-zero == 0, speed rounded to
//! 2 dp) are load-bearing for JSON parity.
//!
//! Naming: Go's `TransferSizeInfo` is exported here as [`SizeInfo`] to match the
//! crate's `lib.rs` re-export (`pub use progress::{ProgressInfo, SizeInfo,
//! TransferStatus}`). Field names are the Go names, snake_cased.

use std::time::SystemTime;

use crate::object::FileInfo;

/// go-mtpx `TransferStatus` (enums.go:3-8). A Go `type TransferStatus string`
/// with exactly two values; modelled as an enum here. [`Self::as_str`] gives the
/// wire spelling keel-ffi serialises (`"InProgress"` / `"Completed"`), preserved
/// verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    /// Emitted on every per-chunk tick while a file is transferring.
    InProgress,
    /// Emitted once, as the final progress tick, after every file is done
    /// (UploadFiles/DownloadFiles set `pInfo.Status = Completed` then call the
    /// progress callback one last time — main.go:542-545 / 694-697).
    Completed,
}

impl TransferStatus {
    /// The exact string Go's `TransferStatus` carried (enums.go:6-7). keel-ffi's
    /// JSON must reproduce these spellings.
    pub fn as_str(self) -> &'static str {
        match self {
            TransferStatus::InProgress => "InProgress",
            TransferStatus::Completed => "Completed",
        }
    }
}

/// go-mtpx `TransferSizeInfo` (structs.go:36-46). Size accounting for either the
/// currently-active file or the whole bulk session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeInfo {
    /// Total size to transfer. **0 when pre-processing was not requested**
    /// (go-mtpx never fills it in that case — the `preprocessFiles == false`
    /// path leaves `totalSize == 0`, so the bulk `Total` stays 0 and every
    /// bulk percentage is `Percent(x, 0) == 0`).
    pub total: i64,
    /// Total size transferred so far.
    pub sent: i64,
    /// Progress as a percentage in `[0, 100]` (`Percent(sent, total)`).
    pub progress: f32,
}

/// go-mtpx `ProgressInfo` (structs.go:48-81). One mutable instance lives for the
/// whole `upload_files`/`download_files` call; it is mutated in place and passed
/// by reference to the caller's progress callback each tick.
#[derive(Clone, Debug)]
pub struct ProgressInfo {
    /// The file currently being transferred. Go held a `*FileInfo` initialised
    /// to `&FileInfo{}`; here it is owned and default-constructed.
    pub file_info: FileInfo,

    /// Wall-clock time the transfer session started (`time.Now()`, structs.go:52).
    pub start_time: SystemTime,

    /// Wall-clock time of the most recent tick. Reset to "now" *after* each
    /// progress callback returns (main.go:497 / helpers.go:522), so it feeds the
    /// next tick's [`transfer_rate`] as the interval start.
    pub latest_sent_time: SystemTime,

    /// Instantaneous transfer rate for the last chunk (see [`transfer_rate`]).
    pub speed: f64,

    /// Total files to transfer. **0 when pre-processing was not requested.**
    pub total_files: i64,

    /// Total directories to transfer. **0 when pre-processing was not requested.**
    pub total_directories: i64,

    /// Files fully transferred so far. Updated *after* each file completes
    /// (main.go:508 / helpers.go:531), so during a file's own ticks this still
    /// reflects the count *before* it — the exact off-by-one the go-mtpx tests
    /// pin (`So(fi.FilesSent, ShouldEqual, prevFilesSent)`).
    pub files_sent: i64,

    /// `Percent(files_sent, total_files)`. `0` for the whole run when
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
    /// The initial `ProgressInfo` go-mtpx builds at the top of `UploadFiles` /
    /// `DownloadFiles` (main.go:277-289 / main.go:560-572): zeroed counters,
    /// `StartTime`/`LatestSentTime` = now, empty `FileInfo`, `Status = InProgress`.
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

/// go-mtpx `Percent` (utils.go:177-183). Percentage of `partial` over `total`,
/// with the **`total <= 0 => 0`** guard preserved verbatim — that guard is why
/// every bulk percentage is `0` when pre-processing is disabled (`total == 0`),
/// which several go-mtpx transfer tests assert.
pub fn percent(partial: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    (partial / total) * 100.0
}

/// go-mtpx `transferRate` (utils.go:247-256). Instantaneous rate for one chunk:
/// `bytes / elapsed_ns * 1000`, rounded to two decimals, `0` when no time has
/// elapsed.
///
/// The arithmetic is reproduced exactly (magic `* 1000`, `Round(rate*100)/100`).
/// Go used `time.Since(lastSentTime)` off a monotonic clock; keel measures the
/// wall-clock interval via [`SystemTime::duration_since`], treating a backwards
/// clock (Err) as "no time elapsed" — the same `elapsedTime <= 0 => 0` outcome
/// Go produced. Speed is a plan §3.4 "behavioural-compatible only" field (the
/// conformance harness normalises it), so the monotonic-vs-wall difference is
/// not a parity break.
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
        // A time in the future => duration_since Err => 0 (Go's elapsed <= 0).
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

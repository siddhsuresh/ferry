//! Shared helpers for both probe modes (direct keel-vfs and `--via-ffi`).
//!
//! Zero external deps by design (the crate keeps `rand` etc. out of its
//! dependency set, per the task): the PRNG is a hand-rolled xorshift, the
//! byte-size formatter is ad-hoc, and the golden source tree is built with
//! `std::fs`. The golden tree mirrors the Swift `Probe.runGoldenSession` fixture
//! (Sources/FerryProbe/Probe.swift:177-188) byte-for-byte so a Rust golden run
//! reproduces the same device-side layout the Go-kernel capture used.

use std::cell::Cell;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// A tiny xorshift64* PRNG. No `rand` dependency (task: keep deps zero); the
/// soak cancel-injection only needs cheap, non-cryptographic randomness.
pub struct Rng(u64);

impl Rng {
    /// Seed from the wall clock; force odd/non-zero (xorshift dies on a 0 state).
    pub fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng(nanos | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        // xorshift64* finalizer.
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `[0, n)`; `0` when `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

/// A cancel trigger for the soak torture test. Implements the `should_cancel`
/// seam keel-vfs's `upload_files`/`download_files` poll (`&dyn Fn() -> bool`):
/// it fires (returns `true`) once the callback has been polled `fire_at` times.
///
/// Uses `Cell` interior mutability because the vfs seam is `Fn`, not `FnMut`
/// (upload.rs:104 / download.rs:111 take `should_cancel: &dyn Fn() -> bool`).
pub struct CancelInjector {
    count: Cell<u64>,
    fire_at: Option<u64>,
}

impl CancelInjector {
    /// `fire_at = None` ⇒ never cancels; `Some(t)` ⇒ cancels on the `t`-th poll.
    pub fn new(fire_at: Option<u64>) -> Self {
        Self {
            count: Cell::new(0),
            fire_at,
        }
    }

    pub fn should(&self) -> bool {
        let c = self.count.get() + 1;
        self.count.set(c);
        matches!(self.fire_at, Some(t) if c >= t)
    }

    /// True if this injector is armed (will fire at some point).
    pub fn armed(&self) -> bool {
        self.fire_at.is_some()
    }
}

/// ~50% chance to arm a cancel, firing after a random 1..=40 progress ticks.
/// Used for the direct-mode soak (cancel is polled per progress tick, so the
/// unit is ticks).
pub fn maybe_cancel_ticks(rng: &mut Rng) -> Option<u64> {
    if rng.below(2) == 0 {
        Some(1 + rng.below(40))
    } else {
        None
    }
}

/// ~50% chance to arm a cancel, firing after a random 20..=419 ms. Used for the
/// `--via-ffi` soak, where cancellation crosses threads via `CancelTransfer`
/// (an atomic store) while the transfer blocks the calling thread — so the unit
/// is a wall-clock delay, not a tick count.
pub fn maybe_cancel_ms(rng: &mut Rng) -> Option<u64> {
    if rng.below(2) == 0 {
        Some(20 + rng.below(400))
    } else {
        None
    }
}

/// Human-readable byte count (base-1000, matching Swift's `.file` ByteCount
/// style closely enough for a dev tool — not load-bearing for parity).
pub fn human_bytes(n: i64) -> String {
    if n < 0 {
        return "?".to_string();
    }
    let n = n as f64;
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if n < 1000.0 {
        return format!("{} B", n as i64);
    }
    let mut v = n;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{:.2} {}", v, UNITS[i])
}

/// Build the golden upload source tree — a faithful port of the Swift capture
/// (Probe.swift:177-188): a 1.5 MB 0xA5 blob, a UTF-8 note with an emoji in both
/// its name and body (exercises the UCS-2→UTF-16 surrogate fix), and a 300 KB
/// 0x5A blob one level down under `sub/`.
pub fn create_golden_src_tree(local: &Path) -> io::Result<()> {
    let sub = local.join("sub");
    fs::create_dir_all(&sub)?;
    // Probe.swift:182 — Data(repeating: 0xA5, count: 1_500_000).
    fs::write(local.join("blob-1.5mb.bin"), vec![0xA5u8; 1_500_000])?;
    // Probe.swift:184 — the emoji filename + body.
    fs::write(
        local.join("note-🛳️.txt"),
        "hello from keel golden capture — émoji: 🛳️\n".as_bytes(),
    )?;
    // Probe.swift:186 — Data(repeating: 0x5A, count: 300_000) under sub/.
    fs::write(sub.join("nested.bin"), vec![0x5Au8; 300_000])?;
    Ok(())
}

/// The base name of a path (Go `filepath.Base` for the soak remote staging dir).
pub fn base_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

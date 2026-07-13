//! Android MTP vendor extensions (OC 0x95C1–0x95C5), as `impl MtpSession` blocks.
//!
//! Faithful port of go-mtpfs `mtp/android.go`, with the plan §3.5 fix to the
//! `GetPartialObject` opcode/param mismatch folded in. These operations are not
//! on Ferry's current parity path — they back post-parity features (ranged /
//! resumable transfers, in-place edits) — but are ported now, including the
//! single most obscure load-bearing quirk: forced separate-header framing around
//! `SendPartialObject`.
//!
//! The AOSP operation being spoken to is implemented in
//! `frameworks/av/media/mtp/MtpServer.cpp` (`doGetPartialObject` /
//! `doSendPartialObject`). Its parameter layout — not the (buggy) ops.go
//! `GetPartialObject` layout — is authoritative here.

use std::io::{Read, Write};

use keel_proto::{Container, OpCode};

use crate::error::MtpError;
use crate::session::MtpSession;
use crate::transport::Transport;

impl<T: Transport> MtpSession<T> {
    /// `ANDROID_GET_PARTIAL_OBJECT64` (0x95C1) — read a byte range of an object.
    ///
    /// PLAN §3.5 FIX. go-mtpfs shipped two entry points for this one opcode:
    ///   * `android.go:33` `AndroidGetPartialObject64` sent the correct 4-param
    ///     64-bit form `{handle, offsetLo, offsetHi, size}`;
    ///   * `ops.go:215` `GetPartialObject` sent `{handle, offset32, size}` — a
    ///     32-bit offset with only 3 params, but under the *64-bit* opcode. On a
    ///     real device that shifts `size` into the offset-hi slot and drops the
    ///     length, corrupting the read.
    ///
    /// keel keeps only the correct form. Layout matches AOSP MtpServer.cpp
    /// `doGetPartialObject`: `param2|(param3<<32)` = 64-bit offset, `param4` =
    /// 32-bit max length. (The size is a single 32-bit parameter — AOSP's
    /// GET_PARTIAL_OBJECT_64 has no 64-bit length; callers page through in
    /// ≤4 GiB windows.) Streams the data phase to `sink`; no progress callback
    /// (Go used `EmptyProgressFunc`, android.go:37).
    pub fn get_partial_object_64(
        &mut self,
        handle: u32,
        sink: &mut dyn Write,
        offset: u64,
        size: u32,
    ) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::ANDROID_GET_PARTIAL_OBJECT64.0,
            params: vec![
                handle,
                (offset & 0xFFFF_FFFF) as u32, // offset low 32
                (offset >> 32) as u32,         // offset high 32
                size,                          // max length (32-bit)
            ],
            ..Default::default()
        };
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, Some(sink), None, 0, &mut noprog)?;
        Ok(())
    }

    /// `ANDROID_BEGIN_EDIT_OBJECT` (0x95C4) — open a file for writing. Must
    /// precede `SendPartialObject`/`TruncateObject` (android.go:41-47).
    pub fn begin_edit_object(&mut self, handle: u32) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::ANDROID_BEGIN_EDIT_OBJECT.0,
            params: vec![handle],
            ..Default::default()
        };
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, None, None, 0, &mut noprog)?;
        Ok(())
    }

    /// `ANDROID_TRUNCATE_OBJECT` (0x95C3) — truncate a file to a 64-bit length
    /// (offset split lo/hi, android.go:50-56).
    pub fn truncate_object(&mut self, handle: u32, offset: u64) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::ANDROID_TRUNCATE_OBJECT.0,
            params: vec![
                handle,
                (offset & 0xFFFF_FFFF) as u32,
                (offset >> 32) as u32,
            ],
            ..Default::default()
        };
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, None, None, 0, &mut noprog)?;
        Ok(())
    }

    /// `ANDROID_SEND_PARTIAL_OBJECT` (0x95C2) — write a byte range of an object.
    /// Params `{handle, offsetLo, offsetHi, size}`, then a `size`-byte data phase
    /// from `source` (android.go:59-72).
    ///
    /// FORCED SEPARATE-HEADER — the single most obscure load-bearing quirk in the
    /// stack (android.go:64-71). AOSP `MtpServer.cpp::doSendPartialObject` copies
    /// any bytes that arrive *in the same USB packet as the 12-byte container
    /// header* using `write(fd, …)` — which appends at the file descriptor's
    /// current position — instead of `pwrite(fd, …, offset)`. When the host
    /// coalesces the header and the first data bytes into one packet, those bytes
    /// land at the wrong file offset and the write is silently corrupted. Forcing
    /// the header into its own packet (so the data starts in a fresh packet, at a
    /// clean offset) sidesteps the bug. Go set the mutable `d.SeparateHeader`
    /// flag around the call and cleared it afterward unconditionally; keel does
    /// the same via `set_separate_header`, restoring it even on error so an early
    /// return cannot leave the whole session stuck in split-header mode.
    pub fn send_partial_object(
        &mut self,
        handle: u32,
        offset: u64,
        size: u32,
        source: &mut dyn Read,
    ) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::ANDROID_SEND_PARTIAL_OBJECT.0,
            params: vec![
                handle,
                (offset & 0xFFFF_FFFF) as u32,
                (offset >> 32) as u32,
                size,
            ],
            ..Default::default()
        };
        // android.go:68 — force header/data into separate writes for THIS
        // transaction only.
        self.set_separate_header(true);
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        let res = self.run_transaction(req, None, Some(source), size as u64, &mut noprog);
        // android.go:70 — always cleared, success or failure.
        self.set_separate_header(false);
        res.map(|_| ())
    }

    /// `ANDROID_END_EDIT_OBJECT` (0x95C5) — close a file opened for write
    /// (android.go:75-81).
    pub fn end_edit_object(&mut self, handle: u32) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::ANDROID_END_EDIT_OBJECT.0,
            params: vec![handle],
            ..Default::default()
        };
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, None, None, 0, &mut noprog)?;
        Ok(())
    }
}

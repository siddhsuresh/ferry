//! Android MTP vendor extensions (OC 0x95C1–0x95C5), as `impl MtpSession` blocks.
//!
//! These operations are not on Ferry's current parity path — they back
//! post-parity features (ranged / resumable transfers, in-place edits) — but are
//! implemented now, including the single most obscure load-bearing quirk: forced
//! separate-header framing around `SendPartialObject`.
//!
//! The AOSP operation being spoken to is implemented in
//! `frameworks/av/media/mtp/MtpServer.cpp` (`doGetPartialObject` /
//! `doSendPartialObject`); its parameter layout is authoritative here.

use std::io::{Read, Write};

use keel_proto::{Container, OpCode};

use crate::error::MtpError;
use crate::session::MtpSession;
use crate::transport::Transport;

impl<T: Transport> MtpSession<T> {
    /// `ANDROID_GET_PARTIAL_OBJECT64` (0x95C1) — read a byte range of an object.
    ///
    /// This is the 64-bit form: 4 params `{handle, offsetLo, offsetHi, size}`.
    /// A 3-param variant `{handle, offset32, size}` under this same opcode is a
    /// trap — on a real device the missing offset-hi slot shifts `size` into it
    /// and drops the length, corrupting the read — so only the 4-param form is
    /// used. Layout matches AOSP MtpServer.cpp `doGetPartialObject`:
    /// `param2|(param3<<32)` = 64-bit offset, `param4` = 32-bit max length. (The
    /// size is a single 32-bit parameter — GET_PARTIAL_OBJECT_64 has no 64-bit
    /// length; callers page through in ≤4 GiB windows.) Streams the data phase to
    /// `sink`; no progress callback.
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
    /// precede `SendPartialObject`/`TruncateObject`.
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
    /// (offset split lo/hi).
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
    /// from `source`.
    ///
    /// FORCED SEPARATE-HEADER — the single most obscure load-bearing quirk here.
    /// AOSP `MtpServer.cpp::doSendPartialObject` copies any bytes that arrive *in
    /// the same USB packet as the 12-byte container header* using `write(fd, …)`
    /// — which appends at the file descriptor's current position — instead of
    /// `pwrite(fd, …, offset)`. When the host coalesces the header and the first
    /// data bytes into one packet, those bytes land at the wrong file offset and
    /// the write is silently corrupted. Forcing the header into its own packet
    /// (so the data starts in a fresh packet, at a clean offset) sidesteps the
    /// bug. `set_separate_header` sets the flag for this one transaction and
    /// restores it even on error, so an early return cannot leave the whole
    /// session stuck in split-header mode.
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
        // Force header/data into separate writes for THIS transaction only.
        self.set_separate_header(true);
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        let res = self.run_transaction(req, None, Some(source), size as u64, &mut noprog);
        // Always cleared, success or failure.
        self.set_separate_header(false);
        res.map(|_| ())
    }

    /// `ANDROID_END_EDIT_OBJECT` (0x95C5) — close a file opened for write.
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

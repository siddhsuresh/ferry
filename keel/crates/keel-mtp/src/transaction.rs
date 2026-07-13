//! The MTP transaction engine: command → optional data-out → data-in →
//! response, with tid stamping, the SeparateHeader (MTP 1.1 App. H) state
//! machine, the XHCI response-in-place-of-ZLP tolerance, and 16 KiB chunk
//! streaming with progress callbacks.
//!
//! The pieces:
//!   * [`MtpSession::run_transaction`] — the wrapper that poisons the
//!     connection on fatal errors
//!   * [`MtpSession::run_transaction_raw`] — the actual work, no sanity checks
//!   * [`MtpSession::send_req`] — encode and write the command
//!   * [`MtpSession::fetch_packet`] — read one packet and split off the header
//!   * [`decode_rep`] — validate and parse the response
//!   * [`MtpSession::bulk_write`] — stream the data-out phase
//!   * [`MtpSession::bulk_read`] — stream the data-in phase
//!
//! Progress: the transaction-level callback is `FnMut(u64) -> Result<(),
//! MtpError>` (bytes transferred). The ops layer wraps it to add the object
//! handle the contract's `FnMut(u64, u32)` shape wants.

use std::io::{Read, Write};
use std::time::Duration;

use keel_proto::{Container, ContainerKind, RcError, RespCode, HDR_LEN};

use crate::error::MtpError;
use crate::session::MtpSession;
use crate::transport::Transport;

/// The Linux USB stack can send 16 KiB per call, per libusb — the chunk size
/// for streaming reads and writes.
const RW_BUF_SIZE: usize = 0x4000;

/// Timeout for the terminal short/zero-length packet — a hard 250 ms deadline
/// regardless of the session timeout.
const ZLP_TIMEOUT: Duration = Duration::from_millis(250);

impl<T: Transport> MtpSession<T> {
    /// The guarded, connection-poisoning wrapper the ops layer calls. Returns
    /// [`MtpError::Closed`] up front if the session is already dead, then runs
    /// the raw transaction and, on a *fatal* error (see [`MtpError::poisons`]),
    /// closes the connection so every later op fails fast.
    ///
    /// `req` is consumed and the decoded response [`Container`] is returned;
    /// `dest` is the device→host data-in sink (GetData / GetObject), `src` the
    /// host→device data-out source (SendData / SendObject).
    pub(crate) fn run_transaction(
        &mut self,
        req: Container,
        dest: Option<&mut dyn Write>,
        src: Option<&mut dyn Read>,
        write_size: u64,
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> Result<Container, MtpError> {
        if self.closed {
            return Err(MtpError::Closed);
        }

        match self.run_transaction_raw(req, dest, src, write_size, progress) {
            Ok(rep) => Ok(rep),
            Err(e) => {
                if e.poisons() {
                    // Fatal error: close the connection. shutdown() is close()'s body.
                    log::warn!("fatal error {e}; closing connection.");
                    self.shutdown();
                }
                Err(e)
            }
        }
    }

    /// The actual transaction work, with **no** sanity checking before/after.
    /// Used directly by `OpenSession` and `close` so a failure does not trigger
    /// the auto-close in [`Self::run_transaction`]. Returns the decoded response
    /// container.
    pub(crate) fn run_transaction_raw(
        &mut self,
        mut req: Container,
        dest: Option<&mut dyn Write>,
        src: Option<&mut dyn Read>,
        write_size: u64,
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> Result<Container, MtpError> {
        // tid stamping. Only when a session is open: the pre-session
        // GetDeviceInfo and OpenSession itself go out with tid = 0.
        if let Some(sess) = self.session.as_mut() {
            req.transaction_id = sess.tid;
            sess.tid = sess.tid.wrapping_add(1); // u32 wraparound.
        }

        // Command phase.
        self.send_req(&req)?;

        // Optional data-out phase.
        if let Some(src) = src {
            self.bulk_write(req.code, req.transaction_id, src, write_size, progress)?;
        }

        // Read the first packet after the command/data-out.
        let mps = self.transport.max_packet_size();
        let mut data = vec![0u8; mps];
        let (resp_header, resp_length, first_n, first_rest) = self.fetch_packet(&mut data)?;

        // If it is a DATA container, drain the data-in phase and then read the
        // response; otherwise the first packet already IS the response.
        let (final_header, final_length, final_rest, unexpected_data) =
            if resp_header.kind == ContainerKind::Data {
                self.read_data_phase(dest, first_n, resp_length, &first_rest, &mut data, progress)?
            } else {
                (resp_header, resp_length, first_rest, false)
            };

        // decode_rep + the post-checks. Note the ordering: the unexpected-data
        // SyncError is returned *before* decode_rep's result is consulted.
        let mut rep = Container::default();
        let decode_result = decode_rep(&final_header, final_length, &final_rest, &mut rep);

        if unexpected_data {
            // The RC-name map is deliberately looked up with the *operation*
            // code here; it almost always misses and prints "0x%x".
            return Err(MtpError::Sync(format!(
                "unexpected data for code {}",
                rc_or_hex(req.code)
            )));
        }

        decode_result?;

        // tid mismatch — only meaningful with a session open.
        if self.session.is_some() && rep.transaction_id != req.transaction_id {
            return Err(MtpError::Sync(format!(
                "transaction ID mismatch got {:x} want {:x}",
                rep.transaction_id, req.transaction_id
            )));
        }

        Ok(rep)
    }

    /// Encode the command header (`Type=COMMAND`, `Length = 12 + 4*nparam`) +
    /// parameter words and write one bulk-OUT.
    fn send_req(&mut self, req: &Container) -> Result<(), MtpError> {
        let payload_len = (req.params.len() * 4) as u64;
        let mut buf = Vec::with_capacity(HDR_LEN as usize + req.params.len() * 4);

        // The command header is always Type=COMMAND regardless of the passed
        // container; build a COMMAND-typed header from req's code/tid.
        let cmd = Container {
            kind: ContainerKind::Command,
            code: req.code,
            transaction_id: req.transaction_id,
            params: Vec::new(),
        };
        buf.extend_from_slice(&cmd.encode_header(payload_len));
        for &p in &req.params {
            buf.extend_from_slice(&p.to_le_bytes());
        }

        self.transport
            .bulk_out(&buf, self.timeout)
            .map_err(MtpError::Transport)?;
        Ok(())
    }

    /// Read one max-packet-size bulk-IN and split off the 12-byte header.
    /// Returns the header container, the raw `Length` field, `n` (bytes read),
    /// and the payload bytes after the header.
    ///
    /// Trust-nothing hardening: a read shorter than a header is a
    /// [`MtpError::Proto`] (non-fatal); a container `Type` outside 1..=4 is
    /// mapped straight to a fatal [`MtpError::Sync`] rather than being deferred
    /// to `decode_rep`.
    fn fetch_packet(
        &mut self,
        data: &mut [u8],
    ) -> Result<(Container, u32, usize, Vec<u8>), MtpError> {
        let mps = self.transport.max_packet_size();
        let end = mps.min(data.len());
        let n = self
            .transport
            .bulk_in(&mut data[..end], self.timeout)
            .map_err(MtpError::Transport)?;

        if n < HDR_LEN as usize {
            return Err(MtpError::Proto(keel_proto::ProtoError::Truncated {
                need: HDR_LEN as usize,
                have: n,
            }));
        }

        let (header, length) = match Container::decode_header(&data[..n]) {
            Ok(x) => x,
            Err(keel_proto::ProtoError::BadContainerType(t)) => {
                return Err(MtpError::Sync(format!(
                    "got type {t} ({}) in packet, want a valid container type.",
                    usb_container_name(t)
                )));
            }
            Err(e) => return Err(MtpError::Proto(e)),
        };

        let rest = data[HDR_LEN as usize..n].to_vec();
        Ok((header, length, n, rest))
    }

    /// The DATA-container arm of the transaction: write the first packet's
    /// payload to `dest`, decide whether more packets follow (running the
    /// SeparateHeader detection while we do), drain the rest via
    /// [`Self::bulk_read`], then obtain the response container — reusing
    /// `bulk_read`'s trailing packet (the XHCI case) or fetching a fresh one.
    ///
    /// Returns the response `(header, length, rest, unexpected_data)`.
    fn read_data_phase(
        &mut self,
        dest: Option<&mut dyn Write>,
        first_n: usize,
        first_length: u32,
        first_rest: &[u8],
        data: &mut [u8],
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> Result<(Container, u32, Vec<u8>, bool), MtpError> {
        let mut null_sink = NullSink;
        let mut unexpected = false;
        // No sink means the op expected no data; discard it and flag the
        // transaction as a desync afterwards.
        let dest_ref: &mut dyn Write = match dest {
            Some(d) => d,
            None => {
                unexpected = true;
                &mut null_sink
            }
        };

        // Write the first packet's payload; the sink error is intentionally ignored.
        let _ = dest_ref.write_all(first_rest);

        let mps = self.transport.max_packet_size();
        // Continue reading? A full first packet, or the device declared more
        // than we have received.
        if first_rest.len() + HDR_LEN as usize == mps || (first_n as u32) < first_length {
            // SeparateHeader detection.
            if first_n == HDR_LEN as usize && first_rest.is_empty() && (first_n as u32) < first_length
            {
                self.separate_header.detect_separate();
            } else {
                // Multi-packet but not header-only ⇒ a coalesced device.
                self.separate_header.detect_coalesced();
            }

            // Drain remaining data; final_packet may be the response (XHCI).
            let (_, final_packet, res) = self.bulk_read(dest_ref, progress);
            res?;

            // Response container.
            if !final_packet.is_empty() {
                match Container::decode_header(&final_packet) {
                    Ok((hdr, len)) => {
                        let rest = final_packet[HDR_LEN as usize..].to_vec();
                        Ok((hdr, len, rest, unexpected))
                    }
                    // A malformed final packet is a fatal desync: the response
                    // header failed to decode.
                    Err(_) => Err(MtpError::Sync(
                        "malformed final response packet".into(),
                    )),
                }
            } else {
                let (hdr, len, _n, rest) = self.fetch_packet(data)?;
                Ok((hdr, len, rest, unexpected))
            }
        } else {
            // Small single-packet data phase: the whole payload was in the first
            // packet; read a fresh packet for the response.
            let (hdr, len, _n, rest) = self.fetch_packet(data)?;
            Ok((hdr, len, rest, unexpected))
        }
    }

    /// Stream the data-out phase. Writes the DATA header (its own 12-byte packet
    /// when SeparateHeader, else header + first chunk), then 16 KiB chunks, then
    /// a terminal zero-length packet when the last transfer was
    /// max-packet-aligned. Returns non-header bytes written.
    fn bulk_write(
        &mut self,
        code: u16,
        tid: u32,
        src: &mut dyn Read,
        size: u64,
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> Result<u64, MtpError> {
        let total_size = size;
        let mut remaining = size;
        let packet_size = self.transport.max_packet_size();
        let mut n: u64 = 0;

        // --- header + first chunk ---
        // Length = HDR_LEN + size, saturated to 0xFFFFFFFF —
        // Container::encode_header does exactly this.
        let hdr = Container {
            kind: ContainerKind::Data,
            code,
            transaction_id: tid,
            params: Vec::new(),
        };
        let header_bytes = hdr.encode_header(size);

        let packet_len = if self.separate_header.is_separate() {
            HDR_LEN as usize // header goes out alone
        } else {
            packet_size
        };
        let mut buf: Vec<u8> = Vec::with_capacity(packet_len.max(HDR_LEN as usize));
        buf.extend_from_slice(&header_bytes);

        let cp_size = (packet_len.saturating_sub(HDR_LEN as usize) as u64).min(remaining);
        copy_n(src, cp_size, &mut buf);

        self.transport
            .bulk_out(&buf, self.timeout)
            .map_err(MtpError::Transport)?;
        remaining -= cp_size;
        n += cp_size;
        progress(total_size - remaining)?;

        // --- main loop, 16 KiB chunks ---
        let mut chunk = [0u8; RW_BUF_SIZE];
        let mut last_transfer: usize = 0;
        while remaining > 0 {
            let want = (RW_BUF_SIZE as u64).min(remaining) as usize;
            let m = match src.read(&mut chunk[..want]) {
                Ok(0) => break,       // EOF
                Ok(m) => m,
                Err(_) => break,      // read error ends the phase
            };
            remaining -= m as u64;
            last_transfer = self
                .transport
                .bulk_out(&chunk[..m], self.timeout)
                .map_err(MtpError::Transport)?;
            n += last_transfer as u64;
            if last_transfer == 0 {
                break;
            }
            progress(total_size - remaining)?;
        }

        // Terminal ZLP "just to be sure" when the last transfer filled a packet.
        // NB last_transfer starts at 0, so a data phase that fit entirely in the
        // first (short) packet still emits it (0 % packet_size == 0). Requested
        // as an empty bulk-OUT; keel-usb's Transport turns an empty write into
        // the wire ZLP.
        if packet_size != 0 && last_transfer % packet_size == 0 {
            let _ = self.transport.bulk_out(&[], ZLP_TIMEOUT);
        }

        Ok(n)
    }

    /// Read the data-in phase into `w` in 16 KiB chunks until a short read, then
    /// — if the last read filled a packet — perform the expected null-packet
    /// read. On Linux + XHCI that "null" read is actually the `CONTAINER_OK`
    /// response, so we return it for the caller to inspect.
    ///
    /// Returns `(bytes_written, trailing_packet, result)`. `trailing_packet` is
    /// empty unless the data ended on a packet boundary.
    fn bulk_read(
        &mut self,
        w: &mut dyn Write,
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> (u64, Vec<u8>, Result<(), MtpError>) {
        let mut buf = [0u8; RW_BUF_SIZE];
        let mut n: u64 = 0;
        let mut last_read: usize = 0;
        let mut result: Result<(), MtpError> = Ok(());

        loop {
            match self.transport.bulk_in(&mut buf, self.timeout) {
                Ok(r) => last_read = r,
                Err(e) => {
                    result = Err(MtpError::Transport(e));
                    break;
                }
            }

            if last_read > 0 {
                match w.write(&buf[..last_read]) {
                    Ok(written) => n += written as u64,
                    // A sink write error is deliberately swallowed here: the loop
                    // just breaks, and the returned result stays whatever the last
                    // bulk-IN was (i.e. Ok). Preserved intentionally to keep
                    // conformance parity with the wire contract.
                    Err(_) => break,
                }
            }

            match progress(n) {
                Ok(()) => {}
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }

            if last_read < buf.len() {
                break; // short read → data phase done
            }
        }

        let packet_size = self.transport.max_packet_size();
        if packet_size != 0 && last_read % packet_size == 0 {
            // Expected null packet — on XHCI it is the response instead.
            let null_read_size = match self.transport.bulk_in(&mut buf, self.timeout) {
                // A successful null-packet read overwrites any prior progress
                // error, clearing result back to Ok.
                Ok(r) => {
                    result = Ok(());
                    r
                }
                Err(e) => {
                    result = Err(MtpError::Transport(e));
                    0
                }
            };
            // Call progress(n) again and return *its* error if any, else the
            // (overwritten) result.
            if let Err(e) = progress(n) {
                return (n, buf[..null_read_size].to_vec(), Err(e));
            }
            return (n, buf[..null_read_size].to_vec(), result);
        }

        (n, Vec::new(), result)
    }
}

/// Validate the response header and parse its parameter words into `rep`. A
/// non-RESPONSE type is a fatal desync ([`MtpError::Sync`]); an over-long
/// declared length is a non-fatal decode error; a non-OK code becomes an
/// [`MtpError::Rc`]. Parameters are parsed *before* the OK check, so error
/// responses still expose any params they carry.
fn decode_rep(
    header: &Container,
    length: u32,
    rest: &[u8],
    rep: &mut Container,
) -> Result<(), MtpError> {
    if header.kind != ContainerKind::Response {
        return Err(MtpError::Sync(format!(
            "got type {} ({}) in response, want CONTAINER_RESPONSE.",
            header.kind as u16,
            usb_container_name(header.kind as u16)
        )));
    }

    rep.kind = ContainerKind::Response;
    rep.code = header.code;
    rep.transaction_id = header.transaction_id;
    rep.params.clear();

    // Signed subtraction: a Length < 12 yields a negative rest_len, which then
    // produces 0 params (the loop below no-ops).
    let rest_len = length as i64 - HDR_LEN as i64;
    if rest_len > rest.len() as i64 {
        // Non-fatal: the declared length overruns what we actually read. We
        // surface it as Truncated; the exact message text is not matched anywhere.
        return Err(MtpError::Proto(keel_proto::ProtoError::Truncated {
            need: rest_len.max(0) as usize,
            have: rest.len(),
        }));
    }

    if rest_len > 0 {
        let n_param = (rest_len / 4) as usize;
        for i in 0..n_param {
            let off = 4 * i;
            // In-bounds: 4*n_param <= rest_len <= rest.len().
            let p = u32::from_le_bytes([rest[off], rest[off + 1], rest[off + 2], rest[off + 3]]);
            rep.params.push(p);
        }
    }

    if rep.code != RespCode::OK.0 {
        // Non-OK response code → RC error. Non-fatal.
        return Err(MtpError::Rc(RcError(RespCode(rep.code))));
    }
    Ok(())
}

/// The RC spec name for `code`, or on a miss `"0x%x"` (lowercase, no padding).
/// Deliberately used with an *op* code for the unexpected-data message.
fn rc_or_hex(code: u16) -> String {
    match RespCode(code).name() {
        Some(n) => n.to_string(),
        None => format!("0x{code:x}"),
    }
}

/// Container-type name for the desync messages. Values are USB-IF spec facts.
fn usb_container_name(ty: u16) -> String {
    match ty {
        0x0000 => "CONTAINER_UNDEFINED".to_string(),
        0x0001 => "CONTAINER_COMMAND".to_string(),
        0x0002 => "CONTAINER_DATA".to_string(),
        0x0003 => "CONTAINER_RESPONSE".to_string(),
        0x0004 => "CONTAINER_EVENT".to_string(),
        512 => "BULK_HS_MAX_PACKET_LEN_READ".to_string(),
        other => format!("0x{other:x}"),
    }
}

/// Copy up to `n` bytes from `src` into `out`; a short read or read error ends
/// the copy (the error is discarded). Never panics.
fn copy_n(src: &mut dyn Read, n: u64, out: &mut Vec<u8>) {
    let mut remaining = n;
    let mut tmp = [0u8; RW_BUF_SIZE];
    while remaining > 0 {
        let want = (RW_BUF_SIZE as u64).min(remaining) as usize;
        match src.read(&mut tmp[..want]) {
            Ok(0) => break,
            Ok(m) => {
                out.extend_from_slice(&tmp[..m]);
                remaining -= m as u64;
            }
            Err(_) => break,
        }
    }
}

/// A sink that accepts and discards everything, used when a data phase arrives
/// for an op that expected none.
struct NullSink;

impl Write for NullSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

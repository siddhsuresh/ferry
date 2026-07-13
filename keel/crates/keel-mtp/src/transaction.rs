//! The MTP transaction engine: command → optional data-out → data-in →
//! response, with tid stamping, the SeparateHeader (MTP 1.1 App. H) state
//! machine, the XHCI response-in-place-of-ZLP tolerance, and 16 KiB chunk
//! streaming with progress callbacks.
//!
//! Port of go-mtpfs `mtp/mtp.go`:
//!   * `RunTransaction`  (381-397)  → [`MtpSession::run_transaction`]  (the
//!     wrapper that poisons the connection on fatal errors)
//!   * `runTransaction`  (401-510)  → [`MtpSession::run_transaction_raw`]
//!   * `sendReq`         (291-318)  → [`MtpSession::send_req`]
//!   * `fetchPacket`     (322-337)  → [`MtpSession::fetch_packet`]
//!   * `decodeRep`       (339-361)  → [`decode_rep`]
//!   * `bulkWrite`       (529-603)  → [`MtpSession::bulk_write`]
//!   * `bulkRead`        (605-657)  → [`MtpSession::bulk_read`]
//!
//! Progress: the transaction-level callback is `FnMut(u64) -> Result<(),
//! MtpError>` (Go's `ProgressFunc func(sent int64) error`, types.go:170). The
//! ops layer wraps it to add the object handle the contract's `FnMut(u64, u32)`
//! shape wants.

use std::io::{Read, Write};
use std::time::Duration;

use keel_proto::{Container, ContainerKind, RcError, RespCode, HDR_LEN};

use crate::error::MtpError;
use crate::session::MtpSession;
use crate::transport::Transport;

/// "The linux usb stack can send 16kb per call, according to libusb."
/// go-mtpfs `rwBufSize` (mtp.go:526).
const RW_BUF_SIZE: usize = 0x4000;

/// Timeout for the terminal short/zero-length packet — go-mtpfs writes it with a
/// hard 250 ms deadline regardless of the session timeout (mtp.go:599).
const ZLP_TIMEOUT: Duration = Duration::from_millis(250);

impl<T: Transport> MtpSession<T> {
    /// `RunTransaction` (mtp.go:381-397): the guarded, connection-poisoning
    /// wrapper the ops layer calls. Returns [`MtpError::Closed`] up front if the
    /// session is already dead (Go's `if d.h == nil { … "device is not open" }`,
    /// mtp.go:383-385), then runs the raw transaction and, on a *fatal* error
    /// (`SyncError`/`usb.Error` — see [`MtpError::poisons`]), closes the
    /// connection so every later op fails fast.
    ///
    /// `req` is consumed and the decoded response [`Container`] is returned (Go's
    /// out-parameter `rep`); `dest` is the device→host data-in sink (GetData /
    /// GetObject), `src` the host→device data-out source (SendData / SendObject).
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
                    // mtp.go:391-392 — "fatal error %v; closing connection." then
                    // d.Close(). shutdown() is Close()'s body.
                    log::warn!("fatal error {e}; closing connection.");
                    self.shutdown();
                }
                Err(e)
            }
        }
    }

    /// `runTransaction` (mtp.go:401-510): the actual work, with **no** sanity
    /// checking before/after. Used directly by `OpenSession` and `Close` so a
    /// failure does not trigger the auto-close in [`Self::run_transaction`].
    /// Returns the decoded response container.
    pub(crate) fn run_transaction_raw(
        &mut self,
        mut req: Container,
        dest: Option<&mut dyn Write>,
        src: Option<&mut dyn Read>,
        write_size: u64,
        progress: &mut dyn FnMut(u64) -> Result<(), MtpError>,
    ) -> Result<Container, MtpError> {
        // tid/sid stamping (mtp.go:404-408). Only when a session is open: the
        // pre-session GetDeviceInfo and OpenSession itself go out with tid = 0.
        if let Some(sess) = self.session.as_mut() {
            req.transaction_id = sess.tid;
            sess.tid = sess.tid.wrapping_add(1); // uint32 wraparound, like Go.
        }

        // Command phase (mtp.go:414).
        self.send_req(&req)?;

        // Optional data-out phase (mtp.go:421-433).
        if let Some(src) = src {
            self.bulk_write(req.code, req.transaction_id, src, write_size, progress)?;
        }

        // Read the first packet after the command/data-out (mtp.go:434-440).
        let mps = self.transport.max_packet_size();
        let mut data = vec![0u8; mps];
        let (resp_header, resp_length, first_n, first_rest) = self.fetch_packet(&mut data)?;

        // If it is a DATA container, drain the data-in phase and then read the
        // response; otherwise the first packet already IS the response
        // (mtp.go:442-491).
        let (final_header, final_length, final_rest, unexpected_data) =
            if resp_header.kind == ContainerKind::Data {
                self.read_data_phase(dest, first_n, resp_length, &first_rest, &mut data, progress)?
            } else {
                (resp_header, resp_length, first_rest, false)
            };

        // decodeRep + the post-checks (mtp.go:493-509). Note the ordering: the
        // unexpected-data SyncError is returned *before* decodeRep's result is
        // consulted (mtp.go:497-499).
        let mut rep = Container::default();
        let decode_result = decode_rep(&final_header, final_length, &final_rest, &mut rep);

        if unexpected_data {
            // mtp.go:498 — getName(RC_names, req.Code) (yes, the RC map is
            // deliberately looked up with the operation code; it almost always
            // misses and prints "0x%x").
            return Err(MtpError::Sync(format!(
                "unexpected data for code {}",
                rc_or_hex(req.code)
            )));
        }

        decode_result?;

        // tid mismatch — only meaningful with a session open (mtp.go:504-507).
        if self.session.is_some() && rep.transaction_id != req.transaction_id {
            return Err(MtpError::Sync(format!(
                "transaction ID mismatch got {:x} want {:x}",
                rep.transaction_id, req.transaction_id
            )));
        }

        Ok(rep)
    }

    /// `sendReq` (mtp.go:291-318): encode the command header (`Type=COMMAND`,
    /// `Length = 12 + 4*nparam`) + parameter words and write one bulk-OUT.
    fn send_req(&mut self, req: &Container) -> Result<(), MtpError> {
        let payload_len = (req.params.len() * 4) as u64;
        let mut buf = Vec::with_capacity(HDR_LEN as usize + req.params.len() * 4);

        // Go's sendReq hardcodes Type=COMMAND regardless of the passed container;
        // build a COMMAND-typed header from req's code/tid so we do the same.
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

    /// `fetchPacket` (mtp.go:322-337): read one max-packet-size bulk-IN and split
    /// off the 12-byte header. Returns the header container, the raw `Length`
    /// field, `n` (bytes read), and the payload bytes after the header.
    ///
    /// Trust-nothing hardening (the Go code trusted the device): a read shorter
    /// than a header is a [`MtpError::Proto`] (non-fatal, like Go's
    /// `io.ErrUnexpectedEOF`); a container `Type` outside 1..=4 — which Go read
    /// raw and only rejected later in `decodeRep` as a `SyncError` — is mapped
    /// straight to a fatal [`MtpError::Sync`].
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

    /// The DATA-container arm of `runTransaction` (mtp.go:442-491): write the
    /// first packet's payload to `dest`, decide whether more packets follow
    /// (running the SeparateHeader detection while we do), drain the rest via
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
        // mtp.go:443-449 — no sink means the op expected no data; discard it and
        // flag the transaction as a desync afterwards.
        let dest_ref: &mut dyn Write = match dest {
            Some(d) => d,
            None => {
                unexpected = true;
                &mut null_sink
            }
        };

        // mtp.go:454 — dest.Write(rest); the error is ignored in Go.
        let _ = dest_ref.write_all(first_rest);

        let mps = self.transport.max_packet_size();
        // "continue reading?" (mtp.go:456): a full first packet, or the device
        // declared more than we have received.
        if first_rest.len() + HDR_LEN as usize == mps || (first_n as u32) < first_length {
            // SeparateHeader detection (mtp.go:464-469).
            if first_n == HDR_LEN as usize && first_rest.is_empty() && (first_n as u32) < first_length
            {
                self.separate_header.detect_separate();
            } else {
                // Multi-packet but not header-only ⇒ a coalesced device.
                self.separate_header.detect_coalesced();
            }

            // Drain remaining data; final_packet may be the response (XHCI).
            let (_, final_packet, res) = self.bulk_read(dest_ref, progress);
            res?; // mtp.go:475-477

            // Response container (mtp.go:480-491).
            if !final_packet.is_empty() {
                match Container::decode_header(&final_packet) {
                    Ok((hdr, len)) => {
                        let rest = final_packet[HDR_LEN as usize..].to_vec();
                        Ok((hdr, len, rest, unexpected))
                    }
                    // Go feeds a zeroed header to decodeRep here (its binary.Read
                    // error is discarded), which then fails the Type check as a
                    // SyncError; we produce the equivalent fatal desync.
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
            // packet; read a fresh packet for the response (mtp.go:489).
            let (hdr, len, _n, rest) = self.fetch_packet(data)?;
            Ok((hdr, len, rest, unexpected))
        }
    }

    /// `bulkWrite` (mtp.go:529-603): stream the data-out phase. Writes the DATA
    /// header (its own 12-byte packet when SeparateHeader, else header + first
    /// chunk), then 16 KiB chunks, then a terminal zero-length packet when the
    /// last transfer was max-packet-aligned. Returns non-header bytes written.
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

        // --- header + first chunk (mtp.go:532-566) ---
        // Length = HDR_LEN + size, saturated to 0xFFFFFFFF (mtp.go:533-537) —
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

        // --- main loop, 16 KiB chunks (mtp.go:568-595) ---
        let mut chunk = [0u8; RW_BUF_SIZE];
        let mut last_transfer: usize = 0;
        while remaining > 0 {
            let want = (RW_BUF_SIZE as u64).min(remaining) as usize;
            let m = match src.read(&mut chunk[..want]) {
                Ok(0) => break,       // EOF — Go: r.Read err → break
                Ok(m) => m,
                Err(_) => break,      // Go: if err != nil { break }
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

        // Terminal ZLP "just to be sure" when the last transfer filled a packet
        // (mtp.go:597-600). NB last_transfer starts at 0, so a data phase that
        // fit entirely in the first (short) packet still emits it, exactly like
        // Go (0 % packet_size == 0). Requested as an empty bulk-OUT; keel-usb's
        // Transport turns an empty write into the wire ZLP (see returned issues).
        if packet_size != 0 && last_transfer % packet_size == 0 {
            let _ = self.transport.bulk_out(&[], ZLP_TIMEOUT);
        }

        Ok(n)
    }

    /// `bulkRead` (mtp.go:605-657): read the data-in phase into `w` in 16 KiB
    /// chunks until a short read, then — if the last read filled a packet —
    /// perform the expected null-packet read. On Linux + XHCI that "null" read
    /// is actually the `CONTAINER_OK` response, so we return it for the caller to
    /// inspect (mtp.go:638-654).
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
                    // go-mtpfs mtp.go:619-622 shadows `err` here, so a sink write
                    // error is swallowed and the loop just breaks (the outer
                    // `err` — returned — stays whatever the last bulk-IN was,
                    // i.e. nil). Preserved: the plan's fix list (§3.5) does not
                    // include this, and matching it keeps conformance parity.
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
                // Go: `nullReadSize, err = BulkTransfer(...)` overwrites err (so a
                // successful read clears a prior progress error to nil).
                Ok(r) => {
                    result = Ok(());
                    r
                }
                Err(e) => {
                    result = Err(MtpError::Transport(e));
                    0
                }
            };
            // Go calls progressCb(n) again and returns *its* error if any
            // (mtp.go:649-651), else the (overwritten) err.
            if let Err(e) = progress(n) {
                return (n, buf[..null_read_size].to_vec(), Err(e));
            }
            return (n, buf[..null_read_size].to_vec(), result);
        }

        (n, Vec::new(), result)
    }
}

/// `decodeRep` (mtp.go:339-361): validate the response header and parse its
/// parameter words into `rep`. A non-RESPONSE type is a fatal desync
/// (`SyncError`); an over-long declared length is a non-fatal decode error; a
/// non-OK code becomes an [`MtpError::Rc`]. Parameters are parsed *before* the
/// OK check, so error responses still expose any params they carry.
fn decode_rep(
    header: &Container,
    length: u32,
    rest: &[u8],
    rep: &mut Container,
) -> Result<(), MtpError> {
    if header.kind != ContainerKind::Response {
        // mtp.go:341.
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

    // restLen := int(h.Length) - usbHdrLen (mtp.go:347). Signed: a Length < 12
    // yields a negative restLen → 0 params (Go's `for i:=0; i<nParam` no-ops).
    let rest_len = length as i64 - HDR_LEN as i64;
    if rest_len > rest.len() as i64 {
        // mtp.go:348-350 — a plain fmt.Errorf (non-fatal). We reuse Truncated;
        // its Display differs from Go's "header specified …" string, but that
        // text is not matched anywhere.
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
        // mtp.go:357-359 — RCError(rep.Code). Non-fatal.
        return Err(MtpError::Rc(RcError(RespCode(rep.code))));
    }
    Ok(())
}

/// go-mtpfs `getName(RC_names, code)` (print.go:39-45): the RC spec name or, on
/// a miss, `"0x%x"` (lowercase, no padding). Deliberately used with an *op* code
/// for the unexpected-data message, matching mtp.go:498.
fn rc_or_hex(code: u16) -> String {
    match RespCode(code).name() {
        Some(n) => n.to_string(),
        None => format!("0x{code:x}"),
    }
}

/// go-mtpfs `USB_names` (const.go:1937-1943) via `getName` — container-type name
/// for the desync messages. Values are USB-IF spec facts.
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

/// `io.CopyN(out, src, n)` semantics (mtp.go:554): copy up to `n` bytes; a short
/// read or read error ends the copy (Go discards the error). Never panics.
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

/// go-mtpfs `NullWriter` (nullreader.go): a sink that accepts and discards
/// everything, used when a data phase arrives for an op that expected none.
struct NullSink;

impl Write for NullSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

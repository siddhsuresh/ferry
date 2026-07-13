//! `FakeDevice`: a scripted [`Transport`] that replays byte-for-byte USB
//! exchanges, driving the real [`MtpSession`] transaction engine end-to-end
//! against pinned wire scenarios.
//!
//! Each `bulk_in` call returns exactly one scripted device→host transfer (one
//! "packet"), and every `bulk_out` call is recorded verbatim (including the
//! empty terminal-ZLP request keel-mtp emits, matching keel-usb's `pipes.rs` ZLP
//! handling). That one-transfer-per-call model gives each test full control over
//! packet boundaries — which is what the SeparateHeader / XHCI-null / saturation
//! quirks are actually about.
//!
//! Scenarios pinned here:
//!   1. `separate_header_autodetect_then_mirrored_writes`
//!   2. `coalesced_header_device`
//!   3. `xhci_response_in_place_of_null_packet`
//!   4. `session_already_opened_blind_close_then_retry`
//!   5. `garbage_tid_syncerror_poisons_session`
//!   6. `four_gib_length_saturation_on_get_object`

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keel_mtp::{MtpError, MtpSession, Transport, TransportError};
use keel_proto::{Container, ContainerKind, OpCode, RespCode, HDR_LEN};

// ---------------------------------------------------------------------------
// The fake transport.
// ---------------------------------------------------------------------------

/// Shared, `Send`-able state so a test can keep inspecting the fake after it has
/// been moved into `MtpSession::configure`. (`Transport: Send`, so no
/// `Rc`/`RefCell` — `Arc<Mutex<_>>`.)
#[derive(Default)]
struct Shared {
    /// Scripted device→host transfers, one per `bulk_in`, in order.
    in_queue: VecDeque<Vec<u8>>,
    /// Every `bulk_out` payload, verbatim (empty vec == a wire ZLP request).
    writes: Vec<Vec<u8>>,
    /// How many times `reset()` was invoked (session recovery ladder).
    reset_count: usize,
    /// Set once `close()` fired (poison/teardown).
    closed: bool,
}

struct FakeDevice {
    mps: usize,
    shared: Arc<Mutex<Shared>>,
}

fn fake(mps: usize, ins: Vec<Vec<u8>>) -> (FakeDevice, Arc<Mutex<Shared>>) {
    let shared = Arc::new(Mutex::new(Shared {
        in_queue: ins.into(),
        ..Default::default()
    }));
    (
        FakeDevice {
            mps,
            shared: Arc::clone(&shared),
        },
        shared,
    )
}

impl Transport for FakeDevice {
    fn bulk_out(&mut self, data: &[u8], _timeout: Duration) -> Result<usize, TransportError> {
        let mut s = self.shared.lock().unwrap();
        if s.closed {
            return Err(TransportError::Io("fake transport closed".into()));
        }
        s.writes.push(data.to_vec());
        Ok(data.len())
    }

    fn bulk_in(&mut self, buf: &mut [u8], _timeout: Duration) -> Result<usize, TransportError> {
        // Pop under the lock, then release it before any panic so a scripting
        // bug surfaces its own message rather than a poisoned-mutex one.
        let pkt = {
            let mut s = self.shared.lock().unwrap();
            if s.closed {
                return Err(TransportError::Io("fake transport closed".into()));
            }
            s.in_queue.pop_front()
        };
        let pkt = pkt.expect(
            "FakeDevice: bulk_in past end of script — the engine read more packets than were \
             scripted (an extra fetch/read the scenario did not expect)",
        );
        assert!(
            pkt.len() <= buf.len(),
            "FakeDevice: scripted IN packet ({} B) exceeds the engine's read buffer ({} B)",
            pkt.len(),
            buf.len()
        );
        buf[..pkt.len()].copy_from_slice(&pkt);
        Ok(pkt.len())
    }

    fn reset(&mut self) -> Result<(), TransportError> {
        self.shared.lock().unwrap().reset_count += 1;
        Ok(())
    }

    fn max_packet_size(&self) -> usize {
        self.mps
    }

    fn close(&mut self) {
        self.shared.lock().unwrap().closed = true;
    }
}

// ---------------------------------------------------------------------------
// Wire builders (device→host packets) and parsers (recorded host→device writes).
// ---------------------------------------------------------------------------

const MPS: usize = 512; // USB2 high-speed bulk MPS — the confirmed real hardware.

/// A 12-byte little-endian bulk header with an arbitrary declared `length`
/// (so DATA packets can advertise more bytes than the first packet carries).
fn header(length: u32, kind: ContainerKind, code: u16, tid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(HDR_LEN as usize);
    v.extend_from_slice(&length.to_le_bytes());
    v.extend_from_slice(&(kind as u16).to_le_bytes());
    v.extend_from_slice(&code.to_le_bytes());
    v.extend_from_slice(&tid.to_le_bytes());
    v
}

/// A RESPONSE container with no parameters (`Length = 12`).
fn response(code: u16, tid: u32) -> Vec<u8> {
    header(HDR_LEN, ContainerKind::Response, code, tid)
}

/// A DATA container header advertising `declared_len` total bytes, followed by
/// whatever `payload` is coalesced into this first packet.
fn data_packet(declared_len: u32, code: u16, tid: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = header(declared_len, ContainerKind::Data, code, tid);
    v.extend_from_slice(payload);
    v
}

/// Decode a recorded write's header → (kind, code, transaction_id, length).
fn decode_write(w: &[u8]) -> (ContainerKind, u16, u32, u32) {
    let (c, len) = Container::decode_header(w).expect("recorded write is a valid container header");
    (c.kind, c.code, c.transaction_id, len)
}

/// The `u32` parameter words after a recorded command's header.
fn write_params(w: &[u8]) -> Vec<u32> {
    w[HDR_LEN as usize..]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// The raw bytes after a recorded write's 12-byte header (a DATA phase payload).
fn write_payload(w: &[u8]) -> &[u8] {
    &w[HDR_LEN as usize..]
}

/// A no-op op/data-transfer progress callback (`FnMut(u64, u32)`).
fn noprog() -> impl FnMut(u64, u32) -> Result<(), MtpError> {
    |_sent, _handle| Ok(())
}

// ---------------------------------------------------------------------------
// (1) SeparateHeader auto-detect on a 12-byte-first-packet device, then the
//     write path mirrors it (header goes out in its own packet).
// ---------------------------------------------------------------------------

#[test]
fn separate_header_autodetect_then_mirrored_writes() {
    // GetObject data-in whose FIRST data packet is exactly the 12-byte header
    // (payload arrives in a later packet) ⇒ MTP 1.1 App. H split-header device.
    let ins = vec![
        response(RespCode::OK.0, 0),                    // OpenSession reply (tid 0)
        data_packet(HDR_LEN + 2, OpCode::GET_OBJECT.0, 1, &[]), // 12-byte header, declares 14
        vec![0x48, 0x49],                               // "HI" — payload in its own packet
        response(RespCode::OK.0, 1),                    // GetObject response (tid 1)
        response(RespCode::OK.0, 2),                    // SendObject response (tid 2)
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure");

    // --- read: triggers auto-detect (n == 12, no rest, n < declared length) ---
    let mut sink: Vec<u8> = Vec::new();
    let n = session
        .get_object(5, &mut sink, &mut noprog())
        .expect("get_object");
    assert_eq!(n, 2, "streamed the 2 payload bytes");
    assert_eq!(sink, vec![0x48, 0x49]);

    // --- write: SeparateHeader is now latched, so the header must ship alone ---
    let payload = [0xAA, 0xBB, 0xCC, 0xDD];
    session
        .send_object(&mut Cursor::new(payload.to_vec()), payload.len() as u64, &mut noprog())
        .expect("send_object");

    let w = shared.lock().unwrap();
    assert_eq!(w.writes.len(), 5, "openSession, getObject, sendObject cmd + header + data");

    // The mirrored write: the DATA header is its OWN 12-byte packet.
    let hdr = &w.writes[3];
    assert_eq!(hdr.len(), HDR_LEN as usize, "header shipped in a separate packet");
    let (kind, code, tid, len) = decode_write(hdr);
    assert_eq!(kind, ContainerKind::Data);
    assert_eq!(code, OpCode::SEND_OBJECT.0);
    assert_eq!(tid, 2);
    assert_eq!(len, HDR_LEN + 4, "DATA length = header + 4 data bytes");
    assert!(write_payload(hdr).is_empty(), "header packet carries no data");
    // The data follows in the NEXT packet (raw bytes), never coalesced with it.
    assert_eq!(&w.writes[4][..], &payload);
    // A short (4-byte) last transfer is not mps-aligned ⇒ no terminal ZLP.
    assert!(w.writes.iter().all(|x| !x.is_empty()), "no ZLP for this size");

    // sid quirk: rand | 1 — low bit set, never 0x00000000/0xFFFFFFFF.
    let sid = write_params(&w.writes[0])[0];
    assert_eq!(sid & 1, 1);
    assert_ne!(sid, 0xFFFF_FFFF);
}

// ---------------------------------------------------------------------------
// (2) Coalesced-header device: first data packet carries header + data
//     together ⇒ detection stays "not separate" ⇒ writes stay coalesced
//     (header + first chunk in one packet).
// ---------------------------------------------------------------------------

#[test]
fn coalesced_header_device() {
    let ins = vec![
        response(RespCode::OK.0, 0),                                  // OpenSession
        data_packet(HDR_LEN + 4, OpCode::GET_OBJECT.0, 1, &[0x41, 0x42]), // hdr + "AB" (14B), declares 16
        vec![0x43, 0x44],                                             // "CD" remainder
        response(RespCode::OK.0, 1),                                  // GetObject response
        response(RespCode::OK.0, 2),                                  // SendObject response
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure");

    // Read: first packet is 14 bytes (header + data), NOT a bare 12-byte header,
    // so detection must NOT flip to separate mode.
    let mut sink: Vec<u8> = Vec::new();
    let n = session
        .get_object(5, &mut sink, &mut noprog())
        .expect("get_object");
    assert_eq!(n, 4);
    assert_eq!(sink, b"ABCD");

    // Write: header + first data chunk are coalesced into ONE packet.
    let payload = [0xAA, 0xBB, 0xCC, 0xDD];
    session
        .send_object(&mut Cursor::new(payload.to_vec()), payload.len() as u64, &mut noprog())
        .expect("send_object");

    let w = shared.lock().unwrap();
    let data_write = &w.writes[3];
    let (kind, code, _tid, len) = decode_write(data_write);
    assert_eq!(kind, ContainerKind::Data);
    assert_eq!(code, OpCode::SEND_OBJECT.0);
    assert_eq!(len, HDR_LEN + 4);
    assert_eq!(
        data_write.len(),
        HDR_LEN as usize + 4,
        "header and data coalesced in one packet (not 12 bytes)"
    );
    assert_eq!(write_payload(data_write), &payload);

    // ZLP quirk: all data fit the first packet ⇒ lastTransfer stays 0 ⇒
    // 0 % mps == 0 ⇒ a terminal ZLP is still emitted. keel signals it as an
    // empty bulk_out.
    assert_eq!(w.writes.len(), 5);
    assert!(w.writes[4].is_empty(), "terminal ZLP after a packet-aligned data phase");
}

// ---------------------------------------------------------------------------
// (3) XHCI: the "null packet" after a packet-aligned data phase is actually the
//     CONTAINER_OK response. The read returns it; the transaction reuses it
//     instead of fetching again.
// ---------------------------------------------------------------------------

#[test]
fn xhci_response_in_place_of_null_packet() {
    let ins = vec![
        response(RespCode::OK.0, 0), // OpenSession
        // First data packet: a full 512-byte packet (12 header + 500 data),
        // declaring more to come ⇒ keep reading.
        data_packet(1024, OpCode::GET_OBJECT.0, 1, &[0u8; 500]),
        // A packet-aligned (512 B) data transfer ⇒ the read expects a null packet.
        vec![0u8; 512],
        // XHCI: instead of a ZLP, the device sends the RESPONSE right here.
        response(RespCode::OK.0, 1),
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure");

    let mut sink: Vec<u8> = Vec::new();
    let n = session
        .get_object(9, &mut sink, &mut noprog())
        .expect("get_object tolerates response-in-place-of-null");
    assert_eq!(n, 500 + 512, "500 (first packet) + 512 (aligned read)");
    assert_eq!(sink.len(), 1012);

    let w = shared.lock().unwrap();
    // Proof of the tolerance: every scripted IN packet was consumed and NO extra
    // fetch happened — the response arrived as the null read and was reused.
    assert!(
        w.in_queue.is_empty(),
        "response consumed as the null packet; no extra fetch_packet"
    );
    // Only the two command writes (OpenSession, GetObject) — a data-in op writes
    // nothing else.
    assert_eq!(w.writes.len(), 2);
    assert_eq!(w.reset_count, 0);
}

// ---------------------------------------------------------------------------
// (4) RC_SessionAlreadyOpened → blind CloseSession → OpenSession succeeds.
//     No USB reset on this ladder arm.
// ---------------------------------------------------------------------------

#[test]
fn session_already_opened_blind_close_then_retry() {
    let ins = vec![
        response(RespCode::SESSION_ALREADY_OPENED.0, 0), // 1st OpenSession → 0x201E
        response(RespCode::OK.0, 0),                     // blind CloseSession reply
        response(RespCode::OK.0, 0),                     // 2nd OpenSession → OK
        response(RespCode::OK.0, 1),                     // follow-up DeleteObject
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure recovers from 0x201E");

    // The session must be usable and its tid counter must start at 1.
    session.delete_object(9).expect("delete_object after recovery");

    let w = shared.lock().unwrap();
    assert_eq!(w.reset_count, 0, "the 0x201E arm never resets the device");
    assert_eq!(w.writes.len(), 4);

    // Command sequence: OpenSession, CloseSession, OpenSession, DeleteObject.
    assert_eq!(decode_write(&w.writes[0]).1, OpCode::OPEN_SESSION.0);
    assert_eq!(decode_write(&w.writes[1]).1, OpCode::CLOSE_SESSION.0);
    assert_eq!(decode_write(&w.writes[2]).1, OpCode::OPEN_SESSION.0);

    // OpenSession commands go out with tid 0 (no session yet).
    assert_eq!(decode_write(&w.writes[0]).2, 0);
    assert_eq!(decode_write(&w.writes[2]).2, 0);
    // Both sids obey the rand|1 rule; they are independently random.
    assert_eq!(write_params(&w.writes[0])[0] & 1, 1);
    assert_eq!(write_params(&w.writes[2])[0] & 1, 1);

    // DeleteObject is the first real op: tid 1, params {handle, 0} — the explicit
    // trailing 0 that some devices require.
    let (_k, code, tid, _len) = decode_write(&w.writes[3]);
    assert_eq!(code, OpCode::DELETE_OBJECT.0);
    assert_eq!(tid, 1);
    assert_eq!(write_params(&w.writes[3]), vec![9, 0]);
}

// ---------------------------------------------------------------------------
// (5) A response with a mismatched transaction ID ⇒ SyncError ⇒ the connection
//     is poisoned (closed); every later op fails fast with `Closed`.
// ---------------------------------------------------------------------------

#[test]
fn garbage_tid_syncerror_poisons_session() {
    let ins = vec![
        response(RespCode::OK.0, 0),          // OpenSession (tid 0)
        response(RespCode::OK.0, 0xDEAD_BEEF), // GetObject reply, GARBAGE tid (want 1)
        response(RespCode::OK.0, 2),          // shutdown's CloseSession reply (tid 2)
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure");

    let mut sink: Vec<u8> = Vec::new();
    let err = session
        .get_object(7, &mut sink, &mut noprog())
        .expect_err("tid mismatch must fail");
    assert!(
        matches!(err, MtpError::Sync(_)),
        "tid mismatch is a SyncError, got {err:?}"
    );

    // The poison already ran the connection's Close (which fired a CloseSession
    // with the next tid, 2), so a later op short-circuits without touching the
    // bus at all.
    let after = session
        .delete_object(1)
        .expect_err("poisoned session rejects further ops");
    assert!(
        matches!(after, MtpError::Closed),
        "post-poison ops return Closed, got {after:?}"
    );

    let w = shared.lock().unwrap();
    assert!(w.closed, "the transport was closed by the poison path");
    assert_eq!(w.reset_count, 0, "CloseSession succeeded, so no reset");
    // OpenSession, GetObject, then the teardown CloseSession (tid 2).
    assert_eq!(w.writes.len(), 3);
    let (_k, code, tid, _len) = decode_write(&w.writes[2]);
    assert_eq!(code, OpCode::CLOSE_SESSION.0);
    assert_eq!(tid, 2);
    // The `delete_object` after poison wrote nothing.
    assert!(w.in_queue.is_empty());
}

// ---------------------------------------------------------------------------
// (6) 4 GiB length saturation on GetObject streaming: the DATA container
//     advertises the 0xFFFFFFFF >4 GiB sentinel, yet the engine streams to a
//     short packet and reports the TRUE byte count (not ~4 GiB) — the length
//     field (n < Length keeps reading) is never trusted as a bound.
// ---------------------------------------------------------------------------

#[test]
fn four_gib_length_saturation_on_get_object() {
    let ins = vec![
        response(RespCode::OK.0, 0), // OpenSession
        // DATA header declares the saturated >4 GiB sentinel (0xFFFFFFFF),
        // carrying a full first packet.
        data_packet(0xFFFF_FFFF, OpCode::GET_OBJECT.0, 1, &[0u8; 500]),
        // 400 more bytes: a short (< 16 KiB) read ends the phase, NOT the length.
        vec![0u8; 400],
        response(RespCode::OK.0, 1), // response
    ];
    let (dev, shared) = fake(MPS, ins);
    let mut session = MtpSession::configure(dev).expect("configure");

    let mut sink: Vec<u8> = Vec::new();
    let n = session
        .get_object(11, &mut sink, &mut noprog())
        .expect("get_object streams past the saturated length");
    assert_eq!(n, 900, "true bytes streamed (500 + 400), not the 0xFFFFFFFF sentinel");
    assert_eq!(sink.len(), 900);

    let w = shared.lock().unwrap();
    assert!(w.in_queue.is_empty(), "streamed to the short packet, consuming exactly the script");
}

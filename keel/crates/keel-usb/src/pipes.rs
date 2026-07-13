//! `UsbTransport` — the bulk in/out/reset pipe layer implementing
//! `keel_mtp::Transport` over nusb 0.2.4.
//!
//! `bulk_out` writes, `bulk_in` reads, and `reset` issues a SIC class reset (not
//! a USB port reset — see `reset` below). Protocol-level framing (SeparateHeader,
//! the XHCI response-in-place-of-ZLP inspection, transaction IDs) lives one layer
//! up in keel-mtp; this file is purely per-pipe byte movement plus the two
//! per-pipe quirks it owns: the ZLP delimiter and 16 KiB chunking.

use std::time::Duration;

use nusb::transfer::{
    BulkOrInterrupt, Buffer, Bulk, ControlOut, ControlType, EndpointDirection, In, Interrupt, Out,
    Recipient, TransferError,
};
use nusb::{Device, Endpoint, Interface};

use keel_mtp::{Transport, TransportError};

use crate::handle::blocking;

/// Per-call chunk ceiling for bulk transfers. nusb/IOKit imposes no such limit,
/// but 16 KiB chunking is a deliberately preserved quirk (some USB stacks accept
/// only 16 KiB per call) and is harmless — just more `transfer_blocking` calls.
/// The terminal ZLP is emitted once for the whole `bulk_out` call, never between
/// chunks.
const RW_BUF_SIZE: usize = 0x4000;

/// SIC (USB Still Image Class) DEVICE_RESET request code.
const SIC_DEVICE_RESET_REQUEST: u8 = 0x66;

/// USB transport over one claimed MTP interface. Owns its three endpoints
/// exclusively (`&mut self` everywhere), so no locking is needed.
pub struct UsbTransport {
    // Held for lifetime/RAII: dropping `interface` releases it and dropping
    // `device` closes the handle. Kept as fields so `reset` can issue control
    // transfers on the interface and so both outlive the endpoints.
    #[allow(dead_code)]
    device: Device,
    interface: Interface,
    interface_number: u8,
    bulk_in: Endpoint<Bulk, In>,
    bulk_out: Endpoint<Bulk, Out>,
    interrupt_in: Endpoint<Interrupt, In>,
    /// Max packet size of the bulk-IN (fetch) endpoint.
    in_mps: usize,
    /// Bytes from an IN completion that overflowed the caller's buffer, returned
    /// first on the next `bulk_in`. See `bulk_in` for why this can happen.
    residue: Vec<u8>,
    closed: bool,
}

impl UsbTransport {
    /// Open the three MTP endpoints on an already-claimed interface and assemble
    /// the transport. Called by `discover` once a candidate has been selected.
    pub(crate) fn open(
        device: Device,
        interface: Interface,
        interface_number: u8,
        bulk_in_addr: u8,
        bulk_out_addr: u8,
        interrupt_in_addr: u8,
    ) -> Result<Self, TransportError> {
        let bulk_in = interface
            .endpoint::<Bulk, In>(bulk_in_addr)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let bulk_out = interface
            .endpoint::<Bulk, Out>(bulk_out_addr)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let interrupt_in = interface
            .endpoint::<Interrupt, In>(interrupt_in_addr)
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let in_mps = bulk_in.max_packet_size();

        Ok(Self {
            device,
            interface,
            interface_number,
            bulk_in,
            bulk_out,
            interrupt_in,
            in_mps,
            residue: Vec::new(),
            closed: false,
        })
    }

    /// Cancel any in-flight transfer on an endpoint and wait for it to drain.
    /// nusb requires `pending() == 0` before `clear_halt`; used by `reset` and
    /// `close`.
    fn drain<E: BulkOrInterrupt, D: EndpointDirection>(ep: &mut Endpoint<E, D>) {
        if ep.pending() > 0 {
            ep.cancel_all();
            while ep.pending() > 0 {
                // Cancelled transfers return promptly; the 1 s cap only guards
                // against a wedged endpoint so close/reset can't hang forever.
                if ep.wait_next_complete(Duration::from_secs(1)).is_none() {
                    break;
                }
            }
        }
    }
}

/// Round `size` up to a non-zero multiple of `mps`. nusb requires IN transfer
/// buffers to be a non-zero multiple of the endpoint's max packet size
/// (`Endpoint::submit`).
fn align_to_packet_size(size: usize, mps: usize) -> usize {
    if mps == 0 {
        return size.max(1);
    }
    if size == 0 {
        return mps;
    }
    if size % mps == 0 {
        size
    } else {
        (size / mps + 1) * mps
    }
}

/// Map a nusb `TransferError` to a `TransportError`, clearing the endpoint halt
/// on a stall so the pipe is usable again.
///
/// A stalled bulk endpoint stays wedged across process restarts until
/// `clear_halt`; the transfer has already completed (with the stall) so the
/// endpoint is idle, which is what `clear_halt` requires.
fn map_transfer_error<E: BulkOrInterrupt, D: EndpointDirection>(
    ep: &mut Endpoint<E, D>,
    err: TransferError,
) -> TransportError {
    match err {
        // transfer_blocking / wait_next_complete cancel on timeout and report
        // Cancelled — surface as the retryable Timeout.
        TransferError::Cancelled => TransportError::Timeout,
        // Device left the bus. TransportError::DeviceGone's Display carries
        // "LIBUSB_ERROR_NO_DEVICE" so the FFI mapper keeps firing
        // ErrorDeviceChanged — required by the wire contract.
        TransferError::Disconnected => TransportError::DeviceGone,
        TransferError::Stall => {
            let _ = blocking(ep.clear_halt());
            TransportError::Stall
        }
        TransferError::Fault | TransferError::InvalidArgument | TransferError::Unknown(_) => {
            TransportError::Io(err.to_string())
        }
    }
}

impl Transport for UsbTransport {
    /// Write one bulk-OUT transfer.
    ///
    /// The terminal-ZLP decision (whether the final message length is a multiple
    /// of the packet size) belongs to the transaction layer, not here:
    /// `Transport::bulk_out` takes a `&[u8]`, not a `Read`, so a multi-GB data
    /// phase MUST be fed in chunks — each `bulk_out` call is one USB transfer, NOT
    /// one complete MTP message. A per-message ZLP here would fire *between*
    /// data-phase chunks and prematurely terminate the transfer (and double-emit
    /// for the all-in-first-packet case). So keel-mtp owns the decision and signals
    /// a required ZLP by calling `bulk_out(&[])`; this impl turns that empty write
    /// into exactly one wire zero-length packet.
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, TransportError> {
        if self.closed {
            return Err(TransportError::Io("transport is closed".into()));
        }

        // Empty write == keel-mtp's explicit terminal-ZLP request. Emit exactly
        // one zero-length bulk transfer (`Buffer::new(0)` is a wire ZLP). Its
        // result is discarded; the 250 ms deadline is the `timeout` keel-mtp passes
        // on this call.
        if data.is_empty() {
            let _ = self.bulk_out.transfer_blocking(Buffer::new(0), timeout).status;
            return Ok(0);
        }

        // A non-empty logical write. keel-mtp already caps each call at 16 KiB,
        // but keep the chunk loop so an oversized slice is still split into 16 KiB
        // transfers. No ZLP is appended here — that is the transaction layer's call
        // (see above).
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + RW_BUF_SIZE).min(data.len());
            let chunk = &data[offset..end];
            let completion = self.bulk_out.transfer_blocking(Buffer::from(chunk), timeout);
            if let Err(e) = completion.status {
                return Err(map_transfer_error(&mut self.bulk_out, e));
            }
            offset = end;
        }

        Ok(data.len())
    }

    /// Read one bulk-IN transfer into `buf`, returning the byte count.
    ///
    /// Uses nusb's `wait_next_complete(timeout)`: on timeout it returns `None`
    /// **without cancelling** the transfer, so the in-flight data is not lost and
    /// the next call resumes it via `pending() > 0`. This cancel-safe,
    /// resume-on-pending read is realised as a single blocking call because
    /// `Transport::bulk_in` is synchronous.
    ///
    /// `residue`: if a resumed transfer completes with more bytes than the
    /// current `buf` can hold (possible when a large read timed out and left a
    /// big buffer pending, then a caller passes a smaller `buf`), the overflow is
    /// stashed and returned first on the next call. This never truncates device
    /// data; the guard exists because we DON'T trust the caller's buffer sizing to
    /// match the pending transfer's.
    fn bulk_in(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        if self.closed {
            return Err(TransportError::Io("transport is closed".into()));
        }

        // 1. Serve any overflow from a previous oversized completion first.
        if !self.residue.is_empty() {
            let n = buf.len().min(self.residue.len());
            buf[..n].copy_from_slice(&self.residue[..n]);
            self.residue.drain(..n);
            return Ok(n);
        }

        // 2. Submit a fresh transfer only if none is already pending (a pending
        //    one is a timed-out read still in flight — resume it).
        if self.bulk_in.pending() == 0 {
            let size = align_to_packet_size(buf.len(), self.in_mps);
            self.bulk_in.submit(Buffer::new(size));
        }

        // 3. Wait up to `timeout`. None ⇒ leave pending (do NOT cancel).
        match self.bulk_in.wait_next_complete(timeout) {
            Some(completion) => {
                if let Err(e) = completion.status {
                    return Err(map_transfer_error(&mut self.bulk_in, e));
                }
                let data = &completion.buffer[..completion.actual_len];
                let n = buf.len().min(data.len());
                buf[..n].copy_from_slice(&data[..n]);
                if data.len() > n {
                    self.residue.extend_from_slice(&data[n..]);
                }
                Ok(n)
            }
            None => Err(TransportError::Timeout),
        }
    }

    /// Reset the device via the SIC (Still Image Class) DEVICE_RESET control
    /// request — NOT a USB port reset.
    ///
    /// A USB port reset (`libusb_reset_device`-style) re-enumerates the device on
    /// macOS and invalidates our handle; the SIC class reset instead returns the
    /// device to Idle in place. Sequence:
    ///   1. DEVICE_RESET control transfer (bRequest 0x66, no payload).
    ///   2. Resync both bulk endpoints: cancel pending → `clear_halt` (the reset
    ///      cleared the device's data toggles).
    ///   3. Drain stale bulk-IN containers left by the aborted transaction, so
    ///      they don't surface next session as a tid mismatch / wrong container.
    fn reset(&mut self) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Io("transport is closed".into()));
        }

        // Step 1: SIC DEVICE_RESET.
        blocking(self.interface.control_out(
            ControlOut {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: SIC_DEVICE_RESET_REQUEST,
                value: 0,
                index: self.interface_number as u16,
                data: &[],
            },
            Duration::from_secs(1),
        ))
        .map_err(|e| map_transfer_error(&mut self.bulk_out, e))?;

        // Step 2: resync both bulk endpoints.
        Self::drain(&mut self.bulk_out);
        let _ = blocking(self.bulk_out.clear_halt());
        Self::drain(&mut self.bulk_in);
        let _ = blocking(self.bulk_in.clear_halt());

        // Step 3: drain stale bulk-IN data until the pipe is idle (300 ms idle
        // race).
        loop {
            if self.bulk_in.pending() == 0 {
                let size = align_to_packet_size(self.in_mps, self.in_mps);
                self.bulk_in.submit(Buffer::new(size));
            }
            let got_data = match self.bulk_in.wait_next_complete(Duration::from_millis(300)) {
                Some(completion) => completion.status.is_ok(),
                None => false,
            };
            if !got_data {
                Self::drain(&mut self.bulk_in);
                break;
            }
        }

        Ok(())
    }

    /// Max packet size for framing decisions.
    /// keel-mtp's transaction layer compares the first data packet against this
    /// (SeparateHeader detection) and computes the terminal-ZLP condition from it.
    /// In/out mps are equal on every real MTP bulk pair.
    fn max_packet_size(&self) -> usize {
        self.in_mps
    }

    /// Release the interface + close the handle, idempotently.
    ///
    /// Cancels any in-flight transfers so no completion callback fires after the
    /// caller considers the transport closed. The OS-level release happens when
    /// the `interface`/`device` fields are dropped (nusb releases on `Drop`);
    /// the `closed` flag makes repeat calls — and any post-close `bulk_*` — safe.
    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        Self::drain(&mut self.bulk_in);
        Self::drain(&mut self.bulk_out);
        Self::drain(&mut self.interrupt_in);
    }
}

//! The seam between protocol and hardware. `keel-usb` implements this over
//! nusb/IOKit; tests implement `FakeDevice` to replay quirk scenarios.
//! PRE-WRITTEN CONTRACT — coordinate before changing anything here.

use std::fmt;
use std::time::Duration;

pub trait Transport: Send {
    /// Write one bulk-OUT transfer. Implementations own ZLP policy
    /// (zero-length packet after max-packet-aligned writes, 250 ms timeout).
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, TransportError>;

    /// Read one bulk-IN transfer into `buf`, returning bytes read.
    fn bulk_in(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;

    /// USB device reset (session recovery ladder + Close failure path).
    fn reset(&mut self) -> Result<(), TransportError>;

    /// Max packet size of the bulk endpoints (512 for USB2 high speed,
    /// 1024 for USB3) — needed for ZLP decisions.
    fn max_packet_size(&self) -> usize;

    /// Release interface + close handle. Idempotent.
    fn close(&mut self);
}

#[derive(Debug)]
pub enum TransportError {
    /// Transfer timed out (recoverable at the transaction layer's discretion).
    Timeout,
    /// Device left the bus. Display MUST contain "LIBUSB_ERROR_NO_DEVICE" —
    /// the FFI error mapper string-matches it to produce ErrorDeviceChanged,
    /// exactly like the Go/libusb stack did.
    DeviceGone,
    /// Endpoint stalled.
    Stall,
    /// Anything else, with the underlying description.
    Io(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Timeout => write!(f, "usb transfer timeout"),
            TransportError::DeviceGone => write!(f, "LIBUSB_ERROR_NO_DEVICE: device is gone"),
            TransportError::Stall => write!(f, "usb endpoint stall"),
            TransportError::Io(s) => write!(f, "usb i/o error: {s}"),
        }
    }
}

impl std::error::Error for TransportError {}

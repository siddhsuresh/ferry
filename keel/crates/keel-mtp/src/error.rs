//! keel-mtp error taxonomy.
//!
//! Ported from go-mtpfs `mtp/mtp.go` — the `RCError` type (lines 70-79), the
//! `SyncError` type (lines 363-369), and the poisoning rule inside
//! `RunTransaction` (lines 381-397):
//!
//! ```go
//! if err := d.runTransaction(...); err != nil {
//!     _, ok2 := err.(SyncError)   // lost protocol synchronization
//!     _, ok1 := err.(usb.Error)   // any libusb error (BUSY/IO/ACCESS/TIMEOUT/…)
//!     if ok1 || ok2 {
//!         log.Printf("fatal error %v; closing connection.", err)
//!         d.Close()               // <- poisons: subsequent ops see d.h == nil
//!     }
//!     return err
//! }
//! ```
//!
//! So exactly two error classes are *fatal* (they close/poison the connection):
//! [`MtpError::Sync`] (Go's `SyncError`) and [`MtpError::Transport`] (Go's
//! `usb.Error`). `RCError`, decode failures, and the "device is not open" guard
//! do **not** poison — see [`MtpError::poisons`].

use std::fmt;

use keel_proto::{ProtoError, RcError};

use crate::transport::TransportError;

/// The unified error type surfaced by [`crate::MtpSession`].
///
/// Contract taxonomy (docs/CONTRACTS.md keel-mtp/error):
/// `enum MtpError { Rc, Sync, Transport, Proto, Closed }`.
#[derive(Debug)]
pub enum MtpError {
    /// A non-OK MTP response code (Go `RCError`, mtp.go:359). Does **not**
    /// poison. `Display` is the bare `RC_names` value (`"StoreFull"`, …) — the
    /// FFI error mapper substring-matches it (`send_to_js/helpers.go:112,115`),
    /// so it is load-bearing; the exactness lives in `keel_proto::RcError`.
    Rc(RcError),

    /// Lost protocol synchronization (Go `SyncError`, mtp.go:363). **Poisons**
    /// the session: the connection is closed and every later op returns
    /// [`MtpError::Closed`]. Raised on: response container with the wrong
    /// `Type`, a transaction-ID mismatch, or data arriving for an op that
    /// expects none (mtp.go:341/498/505).
    Sync(String),

    /// A USB transport failure (Go `usb.Error`). **Poisons** the session, just
    /// like libusb BUSY/IO/ACCESS/TIMEOUT did in Go. `TransportError::DeviceGone`
    /// carries the `"LIBUSB_ERROR_NO_DEVICE"` marker the FFI mapper keys on.
    Transport(TransportError),

    /// A wire decode/encode failure from `keel-proto`. Does **not** poison —
    /// mirrors Go returning a plain `fmt.Errorf`/`io.EOF` (e.g. the
    /// "header specified 0x%x bytes, but have 0x%x" path, mtp.go:349), which is
    /// neither a `SyncError` nor a `usb.Error`.
    Proto(ProtoError),

    /// The session is already closed/poisoned. Corresponds to Go's
    /// `RunTransaction` guard `if d.h == nil { return fmt.Errorf("mtp: cannot
    /// run operation %v, device is not open", …) }` (mtp.go:383-385). Does not
    /// poison (already closed). `Display` keeps the `"device is not open"`
    /// substring the FFI mapper overrides on (plan keel-ffi/errors).
    Closed,
}

impl MtpError {
    /// Whether this error is *fatal* and must close/poison the connection —
    /// the `ok1 || ok2` test in `RunTransaction` (mtp.go:389-390): only
    /// `SyncError` and `usb.Error` qualify.
    pub(crate) fn poisons(&self) -> bool {
        matches!(self, MtpError::Sync(_) | MtpError::Transport(_))
    }
}

impl fmt::Display for MtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MtpError::Rc(e) => write!(f, "{e}"),
            MtpError::Sync(s) => write!(f, "{s}"),
            MtpError::Transport(e) => write!(f, "{e}"),
            MtpError::Proto(e) => write!(f, "{e}"),
            // Go: "mtp: cannot run operation %v, device is not open" (mtp.go:384).
            // We drop the interpolated op name (Closed is a unit variant per the
            // contract); the load-bearing substring "device is not open" stays.
            MtpError::Closed => write!(f, "mtp: cannot run operation, device is not open"),
        }
    }
}

impl std::error::Error for MtpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MtpError::Rc(e) => Some(e),
            MtpError::Transport(e) => Some(e),
            MtpError::Proto(e) => Some(e),
            MtpError::Sync(_) | MtpError::Closed => None,
        }
    }
}

// `?`-ergonomics for the ops layer and the transaction engine. These wrap only;
// the poison classification is decided in one place ([`MtpError::poisons`]), so
// a `?` that produces `Transport`/`Rc` still gets the right fatal/non-fatal
// treatment when it bubbles through `run_transaction`.
impl From<TransportError> for MtpError {
    fn from(e: TransportError) -> Self {
        MtpError::Transport(e)
    }
}

impl From<ProtoError> for MtpError {
    fn from(e: ProtoError) -> Self {
        MtpError::Proto(e)
    }
}

impl From<RcError> for MtpError {
    fn from(e: RcError) -> Self {
        MtpError::Rc(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_proto::RespCode;

    #[test]
    fn only_sync_and_transport_poison() {
        assert!(MtpError::Sync("desync".into()).poisons());
        assert!(MtpError::Transport(TransportError::Timeout).poisons());
        assert!(MtpError::Transport(TransportError::DeviceGone).poisons());
        assert!(!MtpError::Rc(RcError(RespCode::STORE_FULL)).poisons());
        assert!(!MtpError::Proto(ProtoError::Truncated { need: 12, have: 4 }).poisons());
        assert!(!MtpError::Closed.poisons());
    }

    #[test]
    fn closed_display_keeps_ffi_substring() {
        // The FFI processError override matches "device is not open".
        assert!(MtpError::Closed.to_string().contains("device is not open"));
    }

    #[test]
    fn rc_display_is_bare_name_for_ffi_match() {
        // Load-bearing for send_to_js/helpers.go:112,115.
        assert!(MtpError::Rc(RcError(RespCode::STORE_FULL))
            .to_string()
            .contains("StoreFull"));
        assert!(MtpError::Rc(RcError(RespCode::STORE_NOT_AVAILABLE))
            .to_string()
            .contains("StoreNotAvailable"));
    }

    #[test]
    fn device_gone_display_keeps_libusb_marker() {
        assert!(MtpError::Transport(TransportError::DeviceGone)
            .to_string()
            .contains("LIBUSB_ERROR_NO_DEVICE"));
    }
}

//! keel-mtp error taxonomy.
//!
//! Exactly two error classes are *fatal* — they close and poison the connection,
//! so every later op fails fast: [`MtpError::Sync`] (lost protocol
//! synchronization) and [`MtpError::Transport`] (any USB-level failure). When a
//! transaction returns either, the connection is closed and subsequent ops see a
//! dead handle. Response codes, decode failures, and the "device is not open"
//! guard do **not** poison — see [`MtpError::poisons`].

use std::fmt;

use keel_proto::{ProtoError, RcError};

use crate::transport::TransportError;

/// The unified error type surfaced by [`crate::MtpSession`].
///
/// Contract taxonomy (docs/CONTRACTS.md keel-mtp/error):
/// `enum MtpError { Rc, Sync, Transport, Proto, Closed }`.
#[derive(Debug)]
pub enum MtpError {
    /// A non-OK MTP response code. Does **not** poison. `Display` is the bare
    /// response-code name (`"StoreFull"`, …) — the FFI error mapper
    /// substring-matches it, so it is load-bearing; the exact spelling lives in
    /// `keel_proto::RcError`.
    Rc(RcError),

    /// Lost protocol synchronization. **Poisons** the session: the connection is
    /// closed and every later op returns [`MtpError::Closed`]. Raised on a
    /// response container with the wrong `Type`, a transaction-ID mismatch, or
    /// data arriving for an op that expects none.
    Sync(String),

    /// A USB transport failure (BUSY/IO/ACCESS/TIMEOUT, …). **Poisons** the
    /// session. `TransportError::DeviceGone` carries the
    /// `"LIBUSB_ERROR_NO_DEVICE"` marker the FFI mapper keys on.
    Transport(TransportError),

    /// A wire decode/encode failure from `keel-proto`. Does **not** poison — a
    /// malformed dataset (e.g. a header that claims more bytes than actually
    /// arrived) is neither a sync loss nor a transport failure.
    Proto(ProtoError),

    /// The session is already closed/poisoned. Every operation checks this guard
    /// before touching the transport and bails out here. Does not poison (already
    /// closed). `Display` keeps the `"device is not open"` substring the FFI
    /// mapper overrides on.
    Closed,
}

impl MtpError {
    /// Whether this error is *fatal* and must close/poison the connection: only
    /// sync losses and transport failures qualify.
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
            // Closed is a unit variant, so there is no op name to interpolate;
            // the load-bearing substring "device is not open" stays.
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
        // Load-bearing for the FFI error mapper's substring match.
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

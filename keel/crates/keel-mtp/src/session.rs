//! MTP session lifecycle: the `configure` recovery ladder, `close`, and the
//! `OpenSession`/`CloseSession` primitives those two depend on.
//!
//! The transaction engine these call lives in `transaction.rs`.
//!
//! USB open/claim/probe and the session are split across crates: `keel-usb`
//! opens/claims/probes the device and hands us an already-live [`Transport`],
//! so `configure` never reopens the handle — that happened before this session
//! was constructed.

use std::time::Duration;

use keel_proto::{Container, ContainerKind, OpCode, RespCode};

use crate::error::MtpError;
use crate::transport::Transport;

/// Per-transfer timeout used during keel-usb's pre-session probe (2 s). Never
/// used *inside* a session — [`SESSION_TIMEOUT`] is installed before the session
/// ladder runs — but kept as the documented default the probe phase runs at,
/// following the "2 s default, 15 s after session open" timeout model.
#[allow(dead_code)] // documented constant; the probe phase (keel-usb) is what runs at 2s.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2000);

/// Per-transfer timeout for everything from `configure` onward (15 s). The
/// entire session ladder — including `OpenSession` — runs at 15 s, not the 2 s
/// probe timeout; `configure` installs it at the top.
pub(crate) const SESSION_TIMEOUT: Duration = Duration::from_millis(15000);

/// Per-session state. Present only while a session is open.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionData {
    /// Next transaction ID to stamp; incremented after every stamped op.
    pub(crate) tid: u32,
    /// The randomly-chosen session ID sent in `OpenSession`.
    #[allow(dead_code)] // carried only in the OpenSession params; never on the wire header.
    pub(crate) sid: u32,
}

/// MTP 1.1 Appendix H header/data split state — an explicit tri-state over
/// whether the device splits the container header from its data payload.
///
/// Detection (in `transaction.rs`) runs **only** on a multi-packet data phase,
/// and only ever moves us *toward* [`Separate`]:
///   * a header-only first packet (exactly 12 bytes, more data declared) ⇒
///     the device splits header from data ⇒ [`Separate`];
///   * any other multi-packet first packet ⇒ the device coalesces ⇒
///     [`Coalesced`] (informational).
///
/// For the write path only [`Separate`] matters ([`Self::is_separate`]);
/// [`Unknown`] and [`Coalesced`] both write coalesced. So `Coalesced` is
/// behaviourally identical to `Unknown`; it exists purely to name "we observed
/// a coalesced device" cleanly. Because we never lock in `Coalesced` against a
/// later `Separate` transition, the observable behaviour is stable: once
/// detection fires, `Separate` persists for the rest of the connection.
///
/// [`Separate`]: SeparateHeader::Separate
/// [`Coalesced`]: SeparateHeader::Coalesced
/// [`Unknown`]: SeparateHeader::Unknown
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SeparateHeader {
    /// Not yet determined. Writes coalesced.
    #[default]
    Unknown,
    /// Device splits the 12-byte header into its own packet.
    Separate,
    /// Device coalesces header + data (behaviourally == `Unknown`).
    Coalesced,
}

impl SeparateHeader {
    /// Whether the write path must send the header in a separate transfer.
    pub(crate) fn is_separate(self) -> bool {
        matches!(self, SeparateHeader::Separate)
    }

    /// Detection saw a header-only first packet: the device splits the header.
    /// Sticky; overrides any prior state.
    pub(crate) fn detect_separate(&mut self) {
        *self = SeparateHeader::Separate;
    }

    /// Detection saw a coalesced multi-packet first packet. We record it, but
    /// never downgrade a prior `Separate` decision.
    pub(crate) fn detect_coalesced(&mut self) {
        if matches!(self, SeparateHeader::Unknown) {
            *self = SeparateHeader::Coalesced;
        }
    }

    /// Explicit override used by the Android `SendPartialObject` quirk. `true` →
    /// [`Separate`]; `false` → [`Unknown`] (coalesced-behaving and
    /// re-detectable), clobbering any prior detection.
    ///
    /// [`Separate`]: SeparateHeader::Separate
    /// [`Unknown`]: SeparateHeader::Unknown
    pub(crate) fn force(&mut self, separate: bool) {
        *self = if separate {
            SeparateHeader::Separate
        } else {
            SeparateHeader::Unknown
        };
    }
}

/// USB descriptor info (vid/pid/bcd + strings) surfaced through
/// [`MtpSession::usb_info`]. Defined here (not in keel-usb) because keel-mtp
/// cannot depend on keel-usb — the dependency runs the other way (keel-usb
/// implements [`Transport`]). keel-usb populates it during discovery and hands
/// it in via [`MtpSession::set_usb_info`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsbInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
}

/// An open (or recoverable) MTP session over a [`Transport`].
///
/// Fields are `pub(crate)` so the sibling `transaction.rs`/`ops.rs` modules can
/// reach them (Rust makes struct fields private to the *defining* module).
pub struct MtpSession<T: Transport> {
    pub(crate) transport: T,
    /// `Some` once `OpenSession` has succeeded; `None` before and after close.
    pub(crate) session: Option<SessionData>,
    pub(crate) separate_header: SeparateHeader,
    pub(crate) timeout: Duration,
    pub(crate) usb_info: UsbInfo,
    /// Set once the connection is poisoned/closed. Gates every later op.
    pub(crate) closed: bool,
}

impl<T: Transport> MtpSession<T> {
    /// Robust `OpenSession` with a recovery ladder:
    ///   1. `OpenSession`.
    ///   2. On `RC_SessionAlreadyOpened` → blind `CloseSession` (works without a
    ///      transaction ID on Android) → retry `OpenSession`.
    ///   3. On any remaining error → USB reset, sleep 1 s to let the device
    ///      settle, retry `OpenSession` **once**.
    ///
    /// Step 3 relies on `Transport::reset()` to re-establish the pipes rather
    /// than releasing and reopening the handle: the `Transport` trait exposes no
    /// reopen primitive, so we never call `close()` mid-ladder (it would leave
    /// the transport unusable with no way to reopen).
    pub fn configure(transport: T) -> Result<Self, MtpError> {
        let mut sess = MtpSession {
            transport,
            session: None,
            separate_header: SeparateHeader::Unknown,
            // The session ladder runs at the 15 s session timeout.
            timeout: SESSION_TIMEOUT,
            usb_info: UsbInfo::default(),
            closed: false,
        };

        let mut err = sess.open_session();

        // RC_SessionAlreadyOpened (0x201E) → blind close, retry.
        let already_opened = matches!(
            &err,
            Err(MtpError::Rc(rc)) if rc.code() == RespCode::SESSION_ALREADY_OPENED.0
        );
        if already_opened {
            // The close result is intentionally ignored here.
            let _ = sess.close_session();
            err = sess.open_session();
        }

        // Any remaining failure → reset, rest, reopen once.
        if let Err(e) = err {
            log::warn!("OpenSession failed: {e}; attempting reset");
            // Reset the device; the error is intentionally ignored.
            let _ = sess.transport.reset();
            // reset() must re-establish the connection: there is no release +
            // reopen primitive on Transport (see the ladder note above).
            std::thread::sleep(Duration::from_millis(1000));
            // Return the underlying MtpError so its type (Rc/Transport/…) is
            // preserved for the FFI mapper.
            sess.open_session()?;
        }

        Ok(sess)
    }

    /// Release the interface and close the device. Consumes the session.
    pub fn close(mut self) {
        self.shutdown();
    }

    /// The `&mut self` body of `close`, shared with the poison path in
    /// `run_transaction`. Idempotent — a no-op once the connection is closed.
    pub(crate) fn shutdown(&mut self) {
        if self.closed {
            return;
        }

        if self.session.is_some() {
            // CloseSession goes through the RAW transaction path, not the
            // wrapper: the wrapper would itself run close on a fatal error and
            // recurse. A failed CloseSession triggers a USB reset. We ignore any
            // resulting session state — we are tearing down regardless.
            let req = close_session_req();
            let mut progress = empty_progress();
            if self
                .run_transaction_raw(req, None, None, 0, &mut progress)
                .is_err()
            {
                let _ = self.transport.reset();
            }
        }

        // Transport::close releases the interface and closes the handle; it is
        // idempotent and rolls both into one.
        self.transport.close();
        self.closed = true;
    }

    /// `OpenSession`. Uses the RAW transaction path so a failure does not
    /// auto-close the connection — `configure` needs the handle alive to reset
    /// and retry.
    pub(crate) fn open_session(&mut self) -> Result<(), MtpError> {
        if self.session.is_some() {
            // Precondition guard, not a device desync. Never fires from
            // configure (the session starts as None).
            return Err(MtpError::Sync("session already open".into()));
        }

        // 31 random bits with the low bit forced set — avoids both 0x00000000
        // and 0xFFFFFFFF for the session id.
        let sid: u32 = (rand::random::<u32>() & 0x7FFF_FFFF) | 1;

        let req = Container {
            kind: ContainerKind::Command,
            code: OpCode::OPEN_SESSION.0,
            transaction_id: 0,
            params: vec![sid],
        };
        let mut progress = empty_progress();

        // session is None here, so run_transaction_raw does NOT stamp a tid —
        // the OpenSession command goes out with transaction_id = 0.
        self.run_transaction_raw(req, None, None, 0, &mut progress)?;

        // tid counter starts at 1 for the first real op.
        self.session = Some(SessionData { tid: 1, sid });
        Ok(())
    }

    /// `CloseSession`. Uses the WRAPPER transaction path and clears the session
    /// unconditionally, even on error.
    pub(crate) fn close_session(&mut self) -> Result<(), MtpError> {
        let req = close_session_req();
        let mut progress = empty_progress();
        let res = self.run_transaction(req, None, None, 0, &mut progress);
        // Clear the session unconditionally.
        self.session = None;
        res.map(|_| ())
    }

    /// USB device reset, surfaced for callers that need the recovery /
    /// close-failure primitive directly.
    pub fn reset_device(&mut self) -> Result<(), MtpError> {
        self.transport.reset().map_err(MtpError::Transport)
    }

    /// The USB descriptor info captured for this device.
    pub fn usb_info(&self) -> &UsbInfo {
        &self.usb_info
    }

    /// Install the USB descriptor info. keel-usb obtains vid/pid/bcd + string
    /// descriptors during discovery and passes them here; `configure` only
    /// receives the [`Transport`], so this is how the info reaches the session.
    pub fn set_usb_info(&mut self, info: UsbInfo) {
        self.usb_info = info;
    }

    /// Force the MTP 1.1 Appendix H header/data split framing on or off for
    /// subsequent transactions. Used by the Android `SendPartialObject` quirk
    /// (android.rs), which forces it `true` around the op and clears it after.
    pub(crate) fn set_separate_header(&mut self, separate: bool) {
        self.separate_header.force(separate);
    }
}

/// A fresh `OC_CloseSession` command container (no params).
fn close_session_req() -> Container {
    Container {
        kind: ContainerKind::Command,
        code: OpCode::CLOSE_SESSION.0,
        transaction_id: 0,
        params: Vec::new(),
    }
}

/// A no-op progress callback for transactions with no transfer to report.
pub(crate) fn empty_progress() -> impl FnMut(u64) -> Result<(), MtpError> {
    |_: u64| Ok(())
}

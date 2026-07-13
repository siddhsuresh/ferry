//! MTP session lifecycle: the `Configure` recovery ladder, `Close`, and the
//! `OpenSession`/`CloseSession` primitives those two depend on.
//!
//! Ported from go-mtpfs `mtp/mtp.go` (`Configure`, lines 661-693; `Close`,
//! lines 95-126) and `mtp/ops.go` (`OpenSession`, lines 19-41; `CloseSession`,
//! lines 44-50). The transaction engine those call lives in `transaction.rs`.
//!
//! The `Device` struct in Go owns USB open/claim/probe *and* the session; in
//! keel that is split — `keel-usb` opens/claims/probes and hands us an
//! already-live [`Transport`]. So keel's `configure` does **not** re-run Go's
//! `Open()` (`if d.h == nil { Open() }`, mtp.go:662-666); that happened before
//! we were constructed.

use std::time::Duration;

use keel_proto::{Container, ContainerKind, OpCode, RespCode};

use crate::error::MtpError;
use crate::transport::Transport;

/// Per-transfer timeout used during keel-usb's pre-session probe: go-mtpfs
/// `Open()` sets `d.Timeout = 2000` when unset (mtp.go:153-154). Never actually
/// used *inside* a session (go-mtpx installs [`SESSION_TIMEOUT`] before
/// `Configure`), but kept as the documented default the probe phase runs at, per
/// the task's "default 2s, 15s after session open" timeout model.
#[allow(dead_code)] // documented constant; the probe phase (keel-usb) is what runs at 2s.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2000);

/// Per-transfer timeout for everything from `configure` onward. go-mtpx sets
/// `dev.Timeout = devTimeout` (15000 ms) *before* calling `dev.Configure()`
/// (go-mtpx `main.go:29`, `const.go:12`), so the entire session ladder —
/// including `OpenSession` — runs at 15 s, not 2 s. That is where Go "switches"
/// the timeout, so keel installs it at the top of `configure`.
pub(crate) const SESSION_TIMEOUT: Duration = Duration::from_millis(15000);

/// go-mtpfs `sessionData` (mtp.go:65-68). Present only while a session is open.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionData {
    /// Next transaction ID to stamp; incremented after every stamped op.
    pub(crate) tid: u32,
    /// The randomly-chosen session ID sent in `OpenSession`.
    #[allow(dead_code)] // stamped onto Container.SessionID in Go; not on the wire header.
    pub(crate) sid: u32,
}

/// MTP 1.1 Appendix H header/data split state — an explicit tri-state modelling
/// of go-mtpfs's leaky mutable `Device.SeparateHeader bool` (mtp.go:44).
///
/// Detection (transaction.rs, mirroring mtp.go:456-478) runs **only** on a
/// multi-packet data phase, and only ever moves us *toward* [`Separate`]:
///   * a header-only first packet (exactly 12 bytes, more data declared) ⇒
///     the device splits header from data ⇒ [`Separate`] (Go's `true`);
///   * any other multi-packet first packet ⇒ the device coalesces ⇒
///     [`Coalesced`] (informational).
///
/// For the write path only [`Separate`] matters ([`Self::is_separate`]);
/// [`Unknown`] and [`Coalesced`] both write coalesced — exactly Go's `false`.
/// So `Coalesced` is behaviourally identical to `Unknown`; it exists purely to
/// name "we observed a coalesced device" cleanly. Because we never lock in
/// `Coalesced` against a later `Separate` transition, the observable behaviour
/// (when detection fires, that `Separate` persists per-connection) is
/// byte-identical to Go's flag.
///
/// [`Separate`]: SeparateHeader::Separate
/// [`Coalesced`]: SeparateHeader::Coalesced
/// [`Unknown`]: SeparateHeader::Unknown
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SeparateHeader {
    /// Not yet determined. Writes coalesced (Go's initial `false`).
    #[default]
    Unknown,
    /// Device splits the 12-byte header into its own packet (Go's `true`).
    Separate,
    /// Device coalesces header + data (behaviourally == `Unknown`).
    Coalesced,
}

impl SeparateHeader {
    /// Whether the write path must send the header in a separate transfer.
    pub(crate) fn is_separate(self) -> bool {
        matches!(self, SeparateHeader::Separate)
    }

    /// Detection saw a header-only first packet — Go's `d.SeparateHeader = true`
    /// (mtp.go:465). Sticky; overrides any prior state.
    pub(crate) fn detect_separate(&mut self) {
        *self = SeparateHeader::Separate;
    }

    /// Detection saw a coalesced multi-packet first packet — Go leaves the flag
    /// `false` here; we record it, but never downgrade a `Separate` decision.
    pub(crate) fn detect_coalesced(&mut self) {
        if matches!(self, SeparateHeader::Unknown) {
            *self = SeparateHeader::Coalesced;
        }
    }

    /// Explicit override — Go's direct `d.SeparateHeader = <bool>` assignment
    /// (android.go:68/70, around `SendPartialObject`). `true` → [`Separate`];
    /// `false` → [`Unknown`] (coalesced-behaving and re-detectable), clobbering
    /// any prior detection exactly as Go's `= false` did.
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
/// implements [`Transport`]). Mirrors go-mtpfs `UsbDeviceInfo` (mtp.go:49-63)
/// with snake-cased field names; keel-usb populates it and hands it in via
/// [`MtpSession::set_usb_info`].
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
    /// Set once the connection is poisoned/closed — Go's `d.h == nil` sentinel.
    pub(crate) closed: bool,
}

impl<T: Transport> MtpSession<T> {
    /// Robust `OpenSession`, port of go-mtpfs `Configure` (mtp.go:661-693).
    ///
    /// Ladder:
    ///   1. `OpenSession`.
    ///   2. On `RC_SessionAlreadyOpened` → blind `CloseSession` (works without a
    ///      transaction ID on Android) → retry `OpenSession`.
    ///   3. On any remaining error → USB reset, sleep 1 s ("give the device some
    ///      rest"), retry `OpenSession` **once**.
    ///
    /// Deviation (forced by the `Transport` trait shape — see the module note
    /// and the returned issue list): Go's step 3 also does `d.Close()` +
    /// `d.Open()` (release + reopen the handle). The `Transport` trait exposes
    /// no reopen primitive, so keel relies on `Transport::reset()` to
    /// re-establish the pipes; we do **not** call `close()` mid-ladder (it would
    /// leave the transport unusable with no way to reopen).
    pub fn configure(transport: T) -> Result<Self, MtpError> {
        let mut sess = MtpSession {
            transport,
            session: None,
            separate_header: SeparateHeader::Unknown,
            // go-mtpx installs devTimeout (15 s) before Configure (main.go:29).
            timeout: SESSION_TIMEOUT,
            usb_info: UsbInfo::default(),
            closed: false,
        };

        let mut err = sess.open_session();

        // RC_SessionAlreadyOpened (0x201E) → blind close, retry (mtp.go:669-674).
        let already_opened = matches!(
            &err,
            Err(MtpError::Rc(rc)) if rc.code() == RespCode::SESSION_ALREADY_OPENED.0
        );
        if already_opened {
            // Go ignores CloseSession's result here.
            let _ = sess.close_session();
            err = sess.open_session();
        }

        // Any remaining failure → reset, rest, reopen once (mtp.go:676-691).
        if let Err(e) = err {
            log::warn!("OpenSession failed: {e}; attempting reset");
            // Go: d.h.Reset() — error ignored.
            let _ = sess.transport.reset();
            // Go: d.Close() + d.Open() here; not expressible via Transport (see
            // deviation note above). reset() must re-establish the connection.
            std::thread::sleep(Duration::from_millis(1000));
            // Go wraps this as fmt.Errorf("OpenSession after reset: %v", err);
            // we return the underlying MtpError so its type (Rc/Transport/…) is
            // preserved for the FFI mapper. (The "after reset:" prefix is not
            // substring-matched anywhere.)
            sess.open_session()?;
        }

        Ok(sess)
    }

    /// Release the interface and close the device — port of go-mtpfs `Close`
    /// (mtp.go:95-126). Consumes the session (contract: `close(self)`).
    pub fn close(mut self) {
        self.shutdown();
    }

    /// The `&mut self` body of `Close`, shared with the poison path in
    /// `run_transaction`. Idempotent (Go's `if d.h == nil { return }`).
    pub(crate) fn shutdown(&mut self) {
        if self.closed {
            return;
        }

        if self.session.is_some() {
            // Go runs CloseSession via the RAW runTransaction here, not the
            // wrapper — "RunTransaction runs close, so can't use CloseSession()"
            // (mtp.go:104). A failed CloseSession triggers a USB reset
            // (mtp.go:105-110). We ignore any resulting session state (we are
            // tearing down regardless).
            let req = close_session_req();
            let mut progress = empty_progress();
            if self
                .run_transaction_raw(req, None, None, 0, &mut progress)
                .is_err()
            {
                let _ = self.transport.reset();
            }
        }

        // Go: ReleaseInterface + h.Close() (mtp.go:113-120). Transport::close is
        // idempotent and rolls both into one.
        self.transport.close();
        self.closed = true;
    }

    /// `OpenSession` — port of go-mtpfs `OpenSession` (ops.go:19-41). Uses the
    /// RAW transaction path so a failure does not auto-close (Go wants to keep
    /// the handle so `Configure` can reset it, ops.go:29-31).
    pub(crate) fn open_session(&mut self) -> Result<(), MtpError> {
        if self.session.is_some() {
            // Go: fmt.Errorf("session already open") — a precondition guard, not
            // a device desync. Never fires from configure (session starts None).
            return Err(MtpError::Sync("session already open".into()));
        }

        // sid = uint32(rand.Int31()) | 1 (ops.go:27): 31 random bits with the
        // low bit forced set — avoids both 0x00000000 and 0xFFFFFFFF.
        let sid: u32 = (rand::random::<u32>() & 0x7FFF_FFFF) | 1;

        let req = Container {
            kind: ContainerKind::Command,
            code: OpCode::OPEN_SESSION.0,
            transaction_id: 0,
            params: vec![sid],
        };
        let mut progress = empty_progress();

        // session is None here, so run_transaction_raw does NOT stamp a tid —
        // the OpenSession command goes out with transaction_id = 0, matching Go.
        self.run_transaction_raw(req, None, None, 0, &mut progress)?;

        // ops.go:36-40 — tid counter starts at 1 for the first real op.
        self.session = Some(SessionData { tid: 1, sid });
        Ok(())
    }

    /// `CloseSession` — port of go-mtpfs `CloseSession` (ops.go:44-50). Uses the
    /// WRAPPER transaction path (Go uses `d.RunTransaction`, ops.go:47) and
    /// clears the session unconditionally, even on error.
    pub(crate) fn close_session(&mut self) -> Result<(), MtpError> {
        let req = close_session_req();
        let mut progress = empty_progress();
        let res = self.run_transaction(req, None, None, 0, &mut progress);
        // Go: d.session = nil (ops.go:48) — always.
        self.session = None;
        res.map(|_| ())
    }

    /// USB device reset (Go's `d.h.Reset()`), surfaced for callers that need the
    /// recovery/close-failure primitive directly.
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
    /// (android.rs), which forces it `true` around the op and clears it after —
    /// go-mtpfs's mutable `d.SeparateHeader` assignment (android.go:68/70).
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

/// go-mtpfs `EmptyProgressFunc` (mtp.go:90) — a no-op progress callback for
/// transactions with no transfer to report.
pub(crate) fn empty_progress() -> impl FnMut(u64) -> Result<(), MtpError> {
    |_: u64| Ok(())
}

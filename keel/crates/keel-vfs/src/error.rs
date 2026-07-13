//! keel-vfs error taxonomy — the go-mtpx `errors.go` error set, plus the
//! keel-only `ExclusiveAccess` passthrough.
//!
//! Ported from go-mtpx `errors.go` together with the `fmt.Errorf(...)` call
//! sites in `main.go` / `helpers.go` / `utils.go` that supply each error's
//! actual message. In Go every one of these types is an empty struct embedding
//! the `error` interface:
//!
//! ```go
//! type FileObjectError struct { error }
//! ```
//!
//! so `.Error()` simply delegates to the wrapped error's message. The FFI mapper
//! (`send_to_js/errors.go`) discriminates them by *type* (a Go type switch) and
//! only falls back to substring-matching the message. keel mirrors that exactly:
//! the enum variant is the "type", and `Display` reproduces the wrapped message
//! verbatim so the FFI's substring overrides (`StoreFull`, `device is not open`,
//! `LIBUSB_ERROR_NO_DEVICE`, `more than 1 device`, …) keep firing.
//!
//! Contract taxonomy (docs/CONTRACTS.md keel-vfs):
//! `VfsError::{MtpDetectFailed, Configure, DeviceInfo, StorageInfo, NoStorage,
//! ListDirectory, FileNotFound, FilePermission, LocalFile, InvalidPath,
//! FileTransfer, FileObject, SendObject, Mtp}` — plus the `ExclusiveAccess`
//! extension the keel-usb block documents.

use std::fmt;

use keel_mtp::MtpError;

/// The go-mtpx error taxonomy (`errors.go`) + the `ExclusiveAccess` extension.
#[derive(Debug)]
pub enum VfsError {
    /// `MtpDetectFailedError{error: err}` (main.go:23) — device selection failed.
    /// Wraps keel-usb's `DiscoverError` (which is not an `MtpError`), so keel
    /// carries its rendered message; the FFI substring-matches e.g.
    /// `"more than 1 device"` out of it, just as it did off Go's wrapped error.
    MtpDetectFailed(String),

    /// keel EXTENSION (no go-mtpx analogue, documented in docs/CONTRACTS.md
    /// keel-usb). The USB device is held exclusively by another process
    /// (ptpcamerad / Image Capture / Photos / Smart Switch, …). `owner` is the
    /// IORegistry-named holder, best-effort. Peeled off the discover error at
    /// `initialize` so the FFI can raise `ErrorDeviceSetup` with a
    /// "Quit <owner> and try again" message instead of a generic detect failure.
    ExclusiveAccess { owner: Option<String> },

    /// `ConfigureError{error: err}` (main.go:32) — the session ladder failed.
    Configure(MtpError),
    /// `DeviceInfoError{error: err}` (main.go:49).
    DeviceInfo(MtpError),
    /// `StorageInfoError{error: err}` (main.go:59, 71).
    StorageInfo(MtpError),
    /// `NoStorageError{fmt.Errorf("no storage found")}` (main.go:63).
    NoStorage,
    /// `ListDirectoryError{error: err}` (helpers.go:328) — GetObjectHandles in a
    /// directory walk.
    ListDirectory(MtpError),
    /// `FileNotFoundError{fmt.Errorf("file not found: %s", …)}` (helpers.go:113).
    /// An internal control-flow marker inside path resolution (mapped to
    /// `InvalidPath` at the `GetObjectFromPath` boundary), carrying the formatted
    /// message Go built.
    FileNotFound(String),
    /// `FilePermissionError{error: err}` — local-filesystem permission denial
    /// (helpers.go / the upload+download local paths).
    FilePermission(String),
    /// `LocalFileError{error: err}` — other local-filesystem error
    /// (helpers.go / the upload+download local paths).
    LocalFile(String),
    /// `InvalidPathError{fmt.Errorf(...)}` — many call sites, each with its own
    /// message (main.go:119, helpers.go:120/143/152/166/181, …); carries the
    /// formatted text verbatim.
    InvalidPath(String),
    /// `FileTransferError{fmt.Errorf("an error occured while …")}` (main.go:537,
    /// helpers.go:555).
    FileTransfer(String),
    /// `FileObjectError{error: err}` — wraps the underlying MTP op error
    /// (GetObjectInfo, GetObjectHandles, GetObjectPropValue, DeleteObject,
    /// SetObjectPropValue). Kept as the live `MtpError` (not flattened to a
    /// string) so callers can inspect the response code — e.g. `FileExists`'
    /// `RCError == 0x2009` test (main.go:186-193) via [`VfsError::rc_code`].
    FileObject(MtpError),
    /// `SendObjectError{error: err}` (helpers.go:221, 257, 270).
    SendObject(MtpError),
    /// Bare passthrough of an `MtpError` (contract `VfsError::Mtp`).
    Mtp(MtpError),

    /// keel EXTENSION mirroring Go's `send_to_js.TransferCancelledError`
    /// (ferry/kernel/send_to_js/errors.go:14-18). go-mtpx itself has no cancel
    /// concept; the Go kernel bolted one on at the FFI layer (legacy kernel L356/372/
    /// 454/470 poll an `atomic.Bool` inside the preprocess/progress callbacks and
    /// return `TransferCancelledError` on a fire). keel moves that poll into the
    /// transfer callbacks (`upload.rs` / `download.rs` take a `should_cancel`
    /// closure) and raises this distinct variant instead of threading a
    /// stringly-typed error up through `SendObjectError`/`FileTransferError`.
    ///
    /// Display is byte-for-byte Go's `TransferCancelledError.Error()`,
    /// `"transfer cancelled by user"`, so the FFI mapper's *substring* fallback
    /// (send_to_js/helpers.go:15-17 — `strings.Contains(e.Error(), "transfer
    /// cancelled by user")`) still fires even though keel-ffi will normally match
    /// this variant by type → `ErrorTransferCancelled`. Carries no wrapped
    /// `MtpError`, so [`rc_code`](VfsError::rc_code) / `source` are `None`.
    Cancelled,
}

impl VfsError {
    /// If this error ultimately wraps a non-OK MTP response code, return it.
    ///
    /// Load-bearing for go-mtpx `FileExists` (main.go:186-193): it sets
    /// `Exists = false` when a `FileObjectError` wraps `mtp.RCError == 0x2009`
    /// (RC_InvalidObjectHandle). Go read that numeric code straight off the
    /// wrapped `mtp.RCError`; keel preserves the ability by carrying the live
    /// `MtpError` in the op-wrapping variants and exposing the code here.
    pub fn rc_code(&self) -> Option<u16> {
        let mtp = match self {
            VfsError::Configure(e)
            | VfsError::DeviceInfo(e)
            | VfsError::StorageInfo(e)
            | VfsError::ListDirectory(e)
            | VfsError::FileObject(e)
            | VfsError::SendObject(e)
            | VfsError::Mtp(e) => e,
            _ => return None,
        };
        match mtp {
            MtpError::Rc(rc) => Some(rc.code()),
            _ => None,
        }
    }
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Go's embedded-error types render as the wrapped message.
            VfsError::MtpDetectFailed(s) => write!(f, "{s}"),
            VfsError::Configure(e) => write!(f, "{e}"),
            VfsError::DeviceInfo(e) => write!(f, "{e}"),
            VfsError::StorageInfo(e) => write!(f, "{e}"),
            // main.go:63.
            VfsError::NoStorage => write!(f, "no storage found"),
            VfsError::ListDirectory(e) => write!(f, "{e}"),
            VfsError::FileNotFound(s) => write!(f, "{s}"),
            VfsError::FilePermission(s) => write!(f, "{s}"),
            VfsError::LocalFile(s) => write!(f, "{s}"),
            VfsError::InvalidPath(s) => write!(f, "{s}"),
            VfsError::FileTransfer(s) => write!(f, "{s}"),
            VfsError::FileObject(e) => write!(f, "{e}"),
            VfsError::SendObject(e) => write!(f, "{e}"),
            VfsError::Mtp(e) => write!(f, "{e}"),
            // send_to_js/errors.go:17 — verbatim so the FFI substring fallback fires.
            VfsError::Cancelled => write!(f, "transfer cancelled by user"),
            // keel extension. Mirrors keel-usb's DiscoverError::ExclusiveAccess
            // wording so the FFI can surface (and, with an owner, name) the
            // blocking process. No go-mtpx string to match — this path never
            // existed in Go.
            VfsError::ExclusiveAccess { owner: Some(who) } => write!(
                f,
                "device is held exclusively by another process ({who}); quit it and try again"
            ),
            VfsError::ExclusiveAccess { owner: None } => write!(
                f,
                "device is held exclusively by another process; quit Image Capture, \
                 Photos, or any phone-sync app and try again"
            ),
        }
    }
}

impl std::error::Error for VfsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VfsError::Configure(e)
            | VfsError::DeviceInfo(e)
            | VfsError::StorageInfo(e)
            | VfsError::ListDirectory(e)
            | VfsError::FileObject(e)
            | VfsError::SendObject(e)
            | VfsError::Mtp(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_proto::{RcError, RespCode};

    #[test]
    fn no_storage_message_matches_go() {
        // main.go:63 — fmt.Errorf("no storage found").
        assert_eq!(VfsError::NoStorage.to_string(), "no storage found");
    }

    #[test]
    fn file_object_preserves_rc_substring_for_ffi() {
        // send_to_js/helpers.go substring-matches the bare RC name.
        let e = VfsError::FileObject(MtpError::Rc(RcError(RespCode::STORE_FULL)));
        assert!(e.to_string().contains("StoreFull"));
    }

    #[test]
    fn rc_code_exposes_wrapped_response_code() {
        // FileExists' 0x2009 (RC_InvalidObjectHandle) check depends on this.
        let e = VfsError::FileObject(MtpError::Rc(RcError(RespCode(0x2009))));
        assert_eq!(e.rc_code(), Some(0x2009));
        // Message-only variants carry no code.
        assert_eq!(VfsError::InvalidPath("x".into()).rc_code(), None);
        // A non-Rc MtpError yields no code.
        assert_eq!(VfsError::FileObject(MtpError::Closed).rc_code(), None);
    }

    #[test]
    fn exclusive_access_names_owner() {
        let e = VfsError::ExclusiveAccess {
            owner: Some("ptpcamerad".into()),
        };
        assert!(e.to_string().contains("ptpcamerad"));
    }
}

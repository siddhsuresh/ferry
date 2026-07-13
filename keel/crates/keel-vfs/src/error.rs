//! keel-vfs error taxonomy, plus the keel-only `ExclusiveAccess` passthrough.
//!
//! Each variant wraps either an `MtpError` or a preformatted message. The FFI
//! mapper discriminates these by *type* (the enum variant) and only falls back to
//! substring-matching the message. So `Display` reproduces the wrapped message
//! verbatim, keeping the FFI's substring overrides (`StoreFull`, `device is not
//! open`, `LIBUSB_ERROR_NO_DEVICE`, `more than 1 device`, …) firing.
//!
//! Contract taxonomy (docs/CONTRACTS.md keel-vfs):
//! `VfsError::{MtpDetectFailed, Configure, DeviceInfo, StorageInfo, NoStorage,
//! ListDirectory, FileNotFound, FilePermission, LocalFile, InvalidPath,
//! FileTransfer, FileObject, SendObject, Mtp}` — plus the `ExclusiveAccess`
//! extension the keel-usb block documents.

use std::fmt;

use keel_mtp::MtpError;

/// The keel-vfs error taxonomy + the `ExclusiveAccess` extension.
#[derive(Debug)]
pub enum VfsError {
    /// Device selection failed. Wraps keel-usb's `DiscoverError` (which is not an
    /// `MtpError`), so it carries the rendered message; the FFI substring-matches
    /// e.g. `"more than 1 device"` out of it.
    MtpDetectFailed(String),

    /// keel extension (documented in docs/CONTRACTS.md keel-usb). The USB device is
    /// held exclusively by another process (ptpcamerad / Image Capture / Photos /
    /// Smart Switch, …). `owner` is the IORegistry-named holder, best-effort. Peeled
    /// off the discover error at `initialize` so the FFI can raise `ErrorDeviceSetup`
    /// with a "Quit <owner> and try again" message instead of a generic detect
    /// failure.
    ExclusiveAccess { owner: Option<String> },

    /// The session ladder failed.
    Configure(MtpError),
    /// GetDeviceInfo failed.
    DeviceInfo(MtpError),
    /// GetStorageIDs / GetStorageInfo failed.
    StorageInfo(MtpError),
    /// No storage found on the device.
    NoStorage,
    /// GetObjectHandles failed during a directory walk.
    ListDirectory(MtpError),
    /// An internal control-flow marker inside path resolution (mapped to
    /// `InvalidPath` at the `get_object_from_path` boundary), carrying the
    /// formatted "file not found" message.
    FileNotFound(String),
    /// Local-filesystem permission denial (the upload/download local paths).
    FilePermission(String),
    /// Other local-filesystem error (the upload/download local paths).
    LocalFile(String),
    /// Invalid path. Many call sites, each with its own message; carries the
    /// formatted text verbatim.
    InvalidPath(String),
    /// A file-transfer failure ("an error occured while …").
    FileTransfer(String),
    /// Wraps the underlying MTP op error (GetObjectInfo, GetObjectHandles,
    /// GetObjectPropValue, DeleteObject, SetObjectPropValue). Kept as the live
    /// `MtpError` (not flattened to a string) so callers can inspect the response
    /// code — e.g. `file_exists`' `RC == 0x2009` test via [`VfsError::rc_code`].
    FileObject(MtpError),
    /// A SendObjectInfo / SendObject failure.
    SendObject(MtpError),
    /// Bare passthrough of an `MtpError` (contract `VfsError::Mtp`).
    Mtp(MtpError),

    /// keel extension for transfer cancellation. The transfer callbacks
    /// (`upload.rs` / `download.rs` take a `should_cancel` closure) poll for a
    /// cancel request and raise this distinct variant instead of threading a
    /// stringly-typed error up through `SendObject` / `FileTransfer`.
    ///
    /// Display is the frozen string `"transfer cancelled by user"`, so the FFI
    /// mapper's *substring* fallback still fires even though keel-ffi normally
    /// matches this variant by type → `ErrorTransferCancelled`. Carries no wrapped
    /// `MtpError`, so [`rc_code`](VfsError::rc_code) / `source` are `None`.
    Cancelled,
}

impl VfsError {
    /// If this error ultimately wraps a non-OK MTP response code, return it.
    ///
    /// Load-bearing for `file_exists`: it sets `exists = false` when a `FileObject`
    /// error wraps RC 0x2009 (RC_InvalidObjectHandle). The op-wrapping variants
    /// carry the live `MtpError` so the numeric code stays inspectable here.
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
            // Wrapping variants render as their wrapped message.
            VfsError::MtpDetectFailed(s) => write!(f, "{s}"),
            VfsError::Configure(e) => write!(f, "{e}"),
            VfsError::DeviceInfo(e) => write!(f, "{e}"),
            VfsError::StorageInfo(e) => write!(f, "{e}"),
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
            // Verbatim so the FFI substring fallback fires.
            VfsError::Cancelled => write!(f, "transfer cancelled by user"),
            // keel extension. Mirrors keel-usb's DiscoverError::ExclusiveAccess
            // wording so the FFI can surface (and, with an owner, name) the
            // blocking process.
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
        // The exact "no storage found" message.
        assert_eq!(VfsError::NoStorage.to_string(), "no storage found");
    }

    #[test]
    fn file_object_preserves_rc_substring_for_ffi() {
        // The FFI substring-matches the bare RC name.
        let e = VfsError::FileObject(MtpError::Rc(RcError(RespCode::STORE_FULL)));
        assert!(e.to_string().contains("StoreFull"));
    }

    #[test]
    fn rc_code_exposes_wrapped_response_code() {
        // `file_exists`' 0x2009 (RC_InvalidObjectHandle) check depends on this.
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

//! keel-ffi error classifier — maps a kernel error to `(ErrorType, message)`.
//!
//! Takes an untyped error and returns `(ErrorType, message)` for the error
//! envelope. Swift classifies purely on the `errorType` string (22 canonical
//! values). The evaluation order is load-bearing:
//!
//! 1. **cancellation first** (early return) — the typed cancelled error OR the
//!    substring `"transfer cancelled by user"`.
//! 2. **typed switch** — over the bare `RCError` (0x2009 special) and the typed
//!    vfs error variants.
//! 3. **string-equality fallthrough** (only if step 2 left `errorType == ""`) —
//!    the fabricated sentinels `"ErrorMtpDetectFailed"` /
//!    `"ErrorMtpLockExists"` / `"ErrorDeviceChanged"`, else `ErrorGeneral`.
//! 4. **substring overrides** (in this exact precedence) — `allow storage
//!    access`, `device is not open`, `LIBUSB_ERROR_NO_DEVICE` (only when the
//!    type is already `ErrorDeviceInfo`), `more than 1 device`, `StoreFull`,
//!    `StoreNotAvailable`.
//!
//! Input is [`KernelError`], the crate's single funnel error, DEFINED HERE (the
//! cleaner dependency direction — `state.rs`/`abi.rs` depend on `errors.rs`, not
//! the reverse). `state.rs` constructs it (the session helpers, the FetchStorages
//! EOF wrap); `abi.rs` fires it through the callback; this module classifies it.
//! Its two variants are the two classifier operands:
//!   * [`KernelError::Vfs`] — a typed [`VfsError`] (the type-switch operand). A
//!     bare `RCError` arrives here as `VfsError::Mtp(MtpError::Rc(..))` (the
//!     contract's "bare passthrough"); a code wrapped in `FileObject` etc.
//!     matches its own variant, never the RCError case.
//!   * [`KernelError::Message`] — a fabricated string (the sentinels, the
//!     unmarshalling error, the FetchStorages EOF wrap).
//!
//! Preserved quirk (the Swift UI depends on it, so it's part of the wire
//! contract):
//! * `RCError == 0x2009` → `ErrorStorageFull`. 0x2009 is actually
//!   `InvalidObjectHandle`; the mapping is wrong but Swift keys on it, so it
//!   stays. The message even remains `"InvalidObjectHandle"`.
//!
//! Extra variant: [`VfsError::ExclusiveAccess`] → `ErrorDeviceSetup` with an
//! owner-named message, so Swift's existing `isDeviceSetupFailure` UX lights up
//! with a precise competing-app name. Kept inside the typed switch; it never
//! disturbs the 22 canonical values.

use keel_mtp::MtpError;
use keel_vfs::VfsError;

/// The single error value every export funnels into `SendError`. Constructed by
/// `state.rs`/`abi.rs`; classified by [`process_error`].
#[derive(Debug)]
pub enum KernelError {
    /// A typed keel-vfs error — the operand of the classifier's type switch.
    Vfs(VfsError),
    /// A bare fabricated-string error — the operand of the string fallthrough
    /// (`"ErrorMtpDetectFailed"`/`"ErrorDeviceChanged"`/`"ErrorMtpLockExists"`,
    /// the unmarshalling sentinel, the `"error allow storage access. …"` wrap).
    Message(String),
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The typed variant renders its wrapped message; a fabricated string
        // error is itself the message.
        match self {
            KernelError::Vfs(e) => write!(f, "{e}"),
            KernelError::Message(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for KernelError {}

/// The `ErrorType` string set — the exact wire values Swift classifies on.
/// Success is the empty string `""` (handled in `json.rs`).
pub mod error_type {
    pub const MTP_DETECT_FAILED: &str = "ErrorMtpDetectFailed";
    pub const MTP_LOCK_EXISTS: &str = "ErrorMtpLockExists";
    pub const DEVICE_CHANGED: &str = "ErrorDeviceChanged";
    pub const DEVICE_SETUP: &str = "ErrorDeviceSetup";
    pub const MULTIPLE_DEVICE: &str = "ErrorMultipleDevice";
    pub const ALLOW_STORAGE_ACCESS: &str = "ErrorAllowStorageAccess";
    pub const DEVICE_LOCKED: &str = "ErrorDeviceLocked";
    pub const DEVICE_INFO: &str = "ErrorDeviceInfo";
    pub const STORAGE_INFO: &str = "ErrorStorageInfo";
    pub const NO_STORAGE: &str = "ErrorNoStorage";
    pub const STORAGE_FULL: &str = "ErrorStorageFull";
    pub const LIST_DIRECTORY: &str = "ErrorListDirectory";
    pub const FILE_NOT_FOUND: &str = "ErrorFileNotFound";
    pub const FILE_PERMISSION: &str = "ErrorFilePermission";
    pub const LOCAL_FILE_READ: &str = "ErrorLocalFileRead";
    pub const INVALID_PATH: &str = "ErrorInvalidPath";
    pub const FILE_TRANSFER: &str = "ErrorFileTransfer";
    pub const FILE_OBJECT_READ: &str = "ErrorFileObjectRead";
    pub const SEND_OBJECT: &str = "ErrorSendObject";
    pub const TRANSFER_CANCELLED: &str = "ErrorTransferCancelled";
    pub const GENERAL: &str = "ErrorGeneral";
}

/// Message for [`VfsError::ExclusiveAccess`]. Names the competing app so Swift's
/// `isDeviceSetupFailure` UX can tell the user exactly what to quit.
fn exclusive_access_message(owner: Option<&str>) -> String {
    match owner {
        Some(o) => format!("{o} is using the phone. Quit it and reconnect."),
        None => "Another app is using the phone. Quit it and reconnect.".to_string(),
    }
}

/// Classify a [`KernelError`] into `(errorType, errorMsg)`. Steps and precedence
/// are described in the module docs.
pub fn process_error(err: &KernelError) -> (&'static str, String) {
    let e_error = err.to_string(); // the rendered message, computed once

    // ---- 1. cancellation FIRST -------------------------------------------
    // Typed sentinel OR the substring fallback (a transport layer may have
    // wrapped the sentinel on the way up).
    let is_cancel_type = matches!(err, KernelError::Vfs(VfsError::Cancelled));
    if is_cancel_type || e_error.contains("transfer cancelled by user") {
        return (error_type::TRANSFER_CANCELLED, e_error);
    }

    // ---- 2. typed switch -------------------------------------------------
    // Leaves errorType == "" for unmatched types (and for a bare RCError whose
    // code != 0x2009), deferring to steps 3/4.
    let mut error_type_str: &'static str = "";
    let mut error_msg = String::new();

    match err {
        KernelError::Vfs(v) => match v {
            // Bare RCError (via the bare-passthrough variant): 0x2009 →
            // ErrorStorageFull (preserved quirk). Message stays the RC name
            // ("InvalidObjectHandle"). Must precede the catch-all Mtp arm.
            VfsError::Mtp(MtpError::Rc(rc)) if rc.code() == 0x2009 => {
                error_type_str = error_type::STORAGE_FULL;
                error_msg = e_error.clone();
            }
            // Any other bare RCError / bare MtpError → no typed match; only
            // 0x2009 sets a type here. Falls to steps 3/4.
            VfsError::Mtp(_) => {}
            VfsError::MtpDetectFailed(_) => {
                error_type_str = error_type::MTP_DETECT_FAILED;
                error_msg = e_error.clone();
            }
            VfsError::Configure(_) => {
                error_type_str = error_type::DEVICE_SETUP;
                error_msg = e_error.clone();
            }
            VfsError::DeviceInfo(_) => {
                error_type_str = error_type::DEVICE_INFO;
                error_msg = e_error.clone();
            }
            VfsError::StorageInfo(_) => {
                error_type_str = error_type::STORAGE_INFO;
                error_msg = e_error.clone();
            }
            VfsError::NoStorage => {
                error_type_str = error_type::NO_STORAGE;
                error_msg = e_error.clone();
            }
            VfsError::ListDirectory(_) => {
                error_type_str = error_type::LIST_DIRECTORY;
                error_msg = e_error.clone();
            }
            VfsError::FileNotFound(_) => {
                error_type_str = error_type::FILE_NOT_FOUND;
                error_msg = e_error.clone();
            }
            VfsError::FilePermission(_) => {
                error_type_str = error_type::FILE_PERMISSION;
                error_msg = e_error.clone();
            }
            VfsError::LocalFile(_) => {
                error_type_str = error_type::LOCAL_FILE_READ;
                error_msg = e_error.clone();
            }
            VfsError::InvalidPath(_) => {
                error_type_str = error_type::INVALID_PATH;
                error_msg = e_error.clone();
            }
            VfsError::FileTransfer(_) => {
                error_type_str = error_type::FILE_TRANSFER;
                error_msg = e_error.clone();
            }
            VfsError::FileObject(_) => {
                error_type_str = error_type::FILE_OBJECT_READ;
                error_msg = e_error.clone();
            }
            VfsError::SendObject(_) => {
                error_type_str = error_type::SEND_OBJECT;
                error_msg = e_error.clone();
            }
            // Extra variant — kept in the typed switch so it slots in ahead of
            // the string/substring logic without disturbing the 22 values.
            VfsError::ExclusiveAccess { owner } => {
                error_type_str = error_type::DEVICE_SETUP;
                error_msg = exclusive_access_message(owner.as_deref());
            }
            // Handled by step 1; unreachable here.
            VfsError::Cancelled => {}
        },
        // Plain string error — no type match; steps 3/4 handle it.
        KernelError::Message(_) => {}
    }

    // ---- 3. string-equality fallthrough ----------------------------------
    if error_type_str.is_empty() {
        if e_error == "ErrorMtpDetectFailed" {
            error_type_str = error_type::MTP_DETECT_FAILED;
            error_msg = e_error.clone();
        } else if e_error == "ErrorMtpLockExists" {
            error_type_str = error_type::MTP_LOCK_EXISTS;
            error_msg = e_error.clone();
        } else if e_error == "ErrorDeviceChanged" {
            error_type_str = error_type::DEVICE_CHANGED;
            error_msg = e_error.clone();
        } else {
            error_type_str = error_type::GENERAL;
            error_msg = e_error.clone();
        }
    }

    // ---- 4. substring overrides ------------------------------------------
    // Tests `errorMsg` (which at this point always equals the rendered message)
    // and then resets `errorMsg`. The if/else-if precedence is exact. The
    // `LIBUSB_ERROR_NO_DEVICE` override is gated on the type already being
    // ErrorDeviceInfo (so only a DeviceInfo error carrying a device-gone
    // transport error is re-tagged DeviceChanged).
    if error_msg.contains("allow storage access") {
        error_type_str = error_type::ALLOW_STORAGE_ACCESS;
        error_msg = e_error.clone();
    } else if error_msg.contains("device is not open") {
        error_type_str = error_type::DEVICE_LOCKED;
        error_msg = e_error.clone();
    } else if error_type_str == error_type::DEVICE_INFO
        && error_msg.contains("LIBUSB_ERROR_NO_DEVICE")
    {
        error_type_str = error_type::DEVICE_CHANGED;
        error_msg = e_error.clone();
    } else if error_msg.contains("more than 1 device") {
        error_type_str = error_type::MULTIPLE_DEVICE;
        error_msg = e_error.clone();
    } else if error_msg.contains("StoreFull") {
        error_type_str = error_type::STORAGE_FULL;
        error_msg = e_error.clone();
    } else if error_msg.contains("StoreNotAvailable") {
        error_type_str = error_type::NO_STORAGE;
        error_msg = e_error.clone();
    }

    (error_type_str, error_msg)
}

/// Build the FetchStorages Samsung-workaround sentinel emitted when
/// `FetchStorages` returns an EOF: `"error allow storage access. {inner}"`.
/// `state.rs` calls this so the `"allow storage access"` substring
/// [`process_error`] overrides on is produced from a single verbatim source.
pub fn allow_storage_access_sentinel(inner: &str) -> String {
    format!("error allow storage access. {inner}")
}

#[cfg(test)]
mod tests {
    use super::error_type as et;
    use super::*;
    use keel_mtp::TransportError;
    use keel_proto::{RcError, RespCode};

    fn vfs(e: VfsError) -> KernelError {
        KernelError::Vfs(e)
    }
    fn msg(s: &str) -> KernelError {
        KernelError::Message(s.to_string())
    }
    fn rc(code: u16) -> KernelError {
        KernelError::Vfs(VfsError::Mtp(MtpError::Rc(RcError(RespCode(code)))))
    }

    // ---- table-driven: all typed VfsError variants → canonical errorType --

    #[test]
    fn typed_vfs_variants_map_to_canonical_types() {
        // Inert RC 0x2001 (OK) carrier so no substring override trips these
        // pure-type assertions.
        let inert = || MtpError::Rc(RcError(RespCode(0x2001)));
        let cases: Vec<(KernelError, &str)> = vec![
            (vfs(VfsError::MtpDetectFailed("x".into())), et::MTP_DETECT_FAILED),
            (vfs(VfsError::Configure(inert())), et::DEVICE_SETUP),
            (vfs(VfsError::DeviceInfo(inert())), et::DEVICE_INFO),
            (vfs(VfsError::StorageInfo(inert())), et::STORAGE_INFO),
            (vfs(VfsError::NoStorage), et::NO_STORAGE),
            (vfs(VfsError::ListDirectory(inert())), et::LIST_DIRECTORY),
            (vfs(VfsError::FileNotFound("nf".into())), et::FILE_NOT_FOUND),
            (vfs(VfsError::FilePermission("perm".into())), et::FILE_PERMISSION),
            (vfs(VfsError::LocalFile("lf".into())), et::LOCAL_FILE_READ),
            (vfs(VfsError::InvalidPath("ip".into())), et::INVALID_PATH),
            (vfs(VfsError::FileTransfer("ft".into())), et::FILE_TRANSFER),
            (vfs(VfsError::FileObject(inert())), et::FILE_OBJECT_READ),
            (vfs(VfsError::SendObject(inert())), et::SEND_OBJECT),
        ];
        for (err, want) in cases {
            let (got, _) = process_error(&err);
            assert_eq!(got, want, "for {err:?}");
        }
    }

    // ---- string-fallthrough sentinels ------------------------------------

    #[test]
    fn fabricated_string_sentinels() {
        assert_eq!(process_error(&msg("ErrorMtpDetectFailed")).0, et::MTP_DETECT_FAILED);
        assert_eq!(process_error(&msg("ErrorMtpLockExists")).0, et::MTP_LOCK_EXISTS);
        assert_eq!(process_error(&msg("ErrorDeviceChanged")).0, et::DEVICE_CHANGED);
        // Anything else → ErrorGeneral, message preserved.
        let (t, m) = process_error(&msg("some random failure"));
        assert_eq!(t, et::GENERAL);
        assert_eq!(m, "some random failure");
    }

    // ---- cancellation is checked FIRST -----------------------------------

    #[test]
    fn cancellation_by_type_and_by_substring() {
        let (t, m) = process_error(&vfs(VfsError::Cancelled));
        assert_eq!(t, et::TRANSFER_CANCELLED);
        assert_eq!(m, "transfer cancelled by user");
        // Substring fallback (sentinel wrapped in a plain message).
        let (t2, _) = process_error(&msg("upload failed: transfer cancelled by user"));
        assert_eq!(t2, et::TRANSFER_CANCELLED);
    }

    // ---- the preserved 0x2009 quirk --------------------------------------

    #[test]
    fn bare_rc_0x2009_maps_to_storage_full_bug() {
        // 0x2009 is InvalidObjectHandle, but it maps to ErrorStorageFull and
        // leaves the message as the RC name. Preserved.
        let (t, m) = process_error(&rc(0x2009));
        assert_eq!(t, et::STORAGE_FULL);
        assert_eq!(m, "InvalidObjectHandle");
    }

    #[test]
    fn bare_rc_other_code_falls_through_then_substring() {
        // StoreFull (0x200C): bare RCError != 0x2009 → step 2 no-op → step 3
        // ErrorGeneral → step 4 "StoreFull" substring → ErrorStorageFull.
        assert_eq!(process_error(&rc(0x200C)).0, et::STORAGE_FULL);
        // StoreNotAvailable (0x2013) → ErrorNoStorage via substring.
        assert_eq!(process_error(&rc(0x2013)).0, et::NO_STORAGE);
        // An unrelated RC → ErrorGeneral, message = RC name.
        let (t3, m3) = process_error(&rc(0x2003)); // SessionNotOpen
        assert_eq!(t3, et::GENERAL);
        assert_eq!(m3, "SessionNotOpen");
    }

    // ---- substring overrides & precedence --------------------------------

    #[test]
    fn allow_storage_access_override_wins() {
        let e = allow_storage_access_sentinel("EOF");
        let (t, m) = process_error(&msg(&e));
        assert_eq!(t, et::ALLOW_STORAGE_ACCESS);
        assert_eq!(m, e);
    }

    #[test]
    fn device_not_open_maps_to_device_locked() {
        // MtpError::Closed renders "…device is not open"; reaches the mapper as a
        // bare-passthrough VfsError::Mtp(Closed).
        let (t, _) = process_error(&vfs(VfsError::Mtp(MtpError::Closed)));
        assert_eq!(t, et::DEVICE_LOCKED);
    }

    #[test]
    fn libusb_no_device_only_retags_device_info() {
        // DeviceInfoError + LIBUSB_ERROR_NO_DEVICE → DeviceChanged (gated override).
        let di = vfs(VfsError::DeviceInfo(MtpError::Transport(TransportError::DeviceGone)));
        assert_eq!(process_error(&di).0, et::DEVICE_CHANGED);

        // Same transport error as a bare passthrough (NOT DeviceInfo) → stays
        // ErrorGeneral: the override is gated on the type already being DeviceInfo.
        let bare = vfs(VfsError::Mtp(MtpError::Transport(TransportError::DeviceGone)));
        assert_eq!(process_error(&bare).0, et::GENERAL);
    }

    #[test]
    fn more_than_one_device_override() {
        // MtpDetectFailed carrying the discover message → re-tagged Multiple.
        let e = vfs(VfsError::MtpDetectFailed("select: more than 1 device found".to_string()));
        assert_eq!(process_error(&e).0, et::MULTIPLE_DEVICE);
    }

    // ---- keel extension: ExclusiveAccess ---------------------------------

    #[test]
    fn exclusive_access_maps_to_device_setup_with_owner() {
        let (t, m) = process_error(&vfs(VfsError::ExclusiveAccess {
            owner: Some("ptpcamerad".into()),
        }));
        assert_eq!(t, et::DEVICE_SETUP);
        assert!(m.contains("ptpcamerad"), "message names the owner: {m}");
        assert!(m.contains("Quit"), "{m}");

        // No owner → generic but still ErrorDeviceSetup.
        let (t2, _) = process_error(&vfs(VfsError::ExclusiveAccess { owner: None }));
        assert_eq!(t2, et::DEVICE_SETUP);
    }

    // ---- Configure carrying a sync error still classifies as DeviceSetup --

    #[test]
    fn configure_error_is_device_setup() {
        let e = vfs(VfsError::Configure(MtpError::Sync("desync".into())));
        assert_eq!(process_error(&e).0, et::DEVICE_SETUP);
    }
}

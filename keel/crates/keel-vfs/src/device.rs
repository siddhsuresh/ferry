//! The device lifecycle exports — go-mtpx `main.go` `Initialize` (18-36),
//! `Dispose` (39-41), `FetchDeviceInfo` (44-53), and `FetchStorages` (56-81).
//!
//! In Go these are free functions over an `*mtp.Device` that bundles USB handle,
//! session, and probe. keel splits that: `keel-usb` discovers/opens/claims/probes
//! and hands back a live [`Transport`], and `keel-mtp` runs the session ladder.
//! So [`Device`] here is a thin wrapper over a configured
//! `MtpSession<UsbTransport>`, and these four are its constructor + methods.
//!
//! Error wrapping mirrors Go one-for-one: discover failure →
//! [`VfsError::MtpDetectFailed`] (except the exclusive-access case, peeled off as
//! [`VfsError::ExclusiveAccess`]); session-ladder failure →
//! [`VfsError::Configure`]; GetDeviceInfo → [`VfsError::DeviceInfo`]; the storage
//! ops → [`VfsError::StorageInfo`] / [`VfsError::NoStorage`].

use keel_mtp::MtpSession;
use keel_mtp::session::UsbInfo as MtpUsbInfo;
use keel_proto::{DeviceInfo, StorageInfo};
use keel_usb::{DiscoverError, Discovered, UsbInfo, UsbTransport, discover};

use crate::error::VfsError;

/// go-mtpx `Init` (structs.go:11-13). Options for [`initialize`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Init {
    /// Go's `Init.DebugMode`, which set `dev.{MTP,Data,USB}Debug`. keel routes
    /// wire debugging through the `log`/`env_logger` facade (plan §2), so this
    /// flag is currently advisory — see the open issues returned to the gate.
    pub debug_mode: bool,
}

/// go-mtpx `StorageData` (structs.go:15-18): a storage ID paired with its info.
#[derive(Clone, Debug)]
pub struct StorageData {
    pub sid: u32,
    pub info: StorageInfo,
}

/// A live MTP device: a configured session over the USB transport, plus the USB
/// identity captured at discovery. The keel analogue of go-mtpx's `*mtp.Device`.
pub struct Device {
    session: MtpSession<UsbTransport>,
}

/// Map a keel-usb discovery failure into the vfs taxonomy.
///
/// Go wraps *every* `SelectDeviceWithDebugging` failure as `MtpDetectFailedError`
/// (main.go:21-24). keel keeps that for all cases except the ptpcamerad /
/// Image-Capture exclusive-access one, which it peels off into
/// [`VfsError::ExclusiveAccess`] (a keel extension) so the FFI can name the
/// blocking process instead of reporting a generic detect failure.
impl From<DiscoverError> for VfsError {
    fn from(e: DiscoverError) -> Self {
        match e {
            DiscoverError::ExclusiveAccess { owner } => VfsError::ExclusiveAccess { owner },
            other => VfsError::MtpDetectFailed(other.to_string()),
        }
    }
}

/// go-mtpx `Initialize` (main.go:18-36).
///
/// Discover the device (`SelectDeviceWithDebugging("")` → [`discover`]), then run
/// the session ladder (`dev.Configure()` → [`MtpSession::configure`]). The 15 s
/// `devTimeout` Go installs before `Configure` (main.go:29) is already applied
/// inside `configure` (keel-mtp `SESSION_TIMEOUT`). The USB identity keel-usb
/// returns alongside the transport is installed onto the session so later
/// `usb_info()` reads (FFI `usbDeviceInfo`, the device-change serial compare)
/// have it — Go got this for free because `Open()` populated it on the device.
pub fn initialize(opts: Init) -> Result<Device, VfsError> {
    let Init { debug_mode } = opts;
    if debug_mode {
        log::debug!("keel-vfs initialize: debug mode requested");
    }

    // main.go:19 — device selection (+ open/claim/probe). The `?` maps
    // DiscoverError via `From` above: ExclusiveAccess → ExclusiveAccess,
    // everything else → MtpDetectFailed (main.go:23).
    let Discovered {
        transport,
        usb_info,
    } = discover()?;

    // main.go:31 — dev.Configure(): the OpenSession recovery ladder.
    let mut session = MtpSession::configure(transport).map_err(VfsError::Configure)?;

    // Install the USB descriptor identity captured at discovery.
    session.set_usb_info(to_mtp_usb_info(usb_info));

    Ok(Device { session })
}

fn to_mtp_usb_info(u: UsbInfo) -> MtpUsbInfo {
    MtpUsbInfo {
        vendor_id: u.vendor_id,
        product_id: u.product_id,
        bcd_device: u.bcd_device,
        manufacturer: u.manufacturer,
        product: u.product,
        serial: u.serial,
    }
}

impl Device {
    /// go-mtpx `Initialize` — see the free [`initialize`] function.
    pub fn initialize(opts: Init) -> Result<Device, VfsError> {
        initialize(opts)
    }

    /// go-mtpx `Dispose` (main.go:39-41): `dev.Close()`. Consumes the device.
    pub fn dispose(self) {
        self.session.close();
    }

    /// go-mtpx `FetchDeviceInfo` (main.go:44-53).
    pub fn fetch_device_info(&mut self) -> Result<DeviceInfo, VfsError> {
        self.session.device_info().map_err(VfsError::DeviceInfo)
    }

    /// go-mtpx `FetchStorages` (main.go:56-81): GetStorageIDs, error out if none,
    /// then GetStorageInfo per ID.
    pub fn fetch_storages(&mut self) -> Result<Vec<StorageData>, VfsError> {
        let sids = self.session.storage_ids().map_err(VfsError::StorageInfo)?;

        // main.go:62 — `len(sids.Values) < 1`.
        if sids.is_empty() {
            return Err(VfsError::NoStorage);
        }

        let mut result = Vec::with_capacity(sids.len());
        for sid in sids {
            let info = self
                .session
                .storage_info(sid)
                .map_err(VfsError::StorageInfo)?;
            result.push(StorageData { sid, info });
        }

        Ok(result)
    }

    /// Borrow the underlying session (immutably). The other vfs modules
    /// (walk / dirops / upload / download) and the FFI reach the session — e.g.
    /// `session().usb_info()` for the device-change serial compare — through
    /// these accessors.
    pub fn session(&self) -> &MtpSession<UsbTransport> {
        &self.session
    }

    /// Borrow the underlying session mutably. The path-level operations
    /// (`walk` / `dirops` / `upload` / `download`, and the FFI) drive MTP ops by
    /// taking `&mut MtpSession<T>`; this is how they obtain it from a [`Device`].
    pub fn session_mut(&mut self) -> &mut MtpSession<UsbTransport> {
        &mut self.session
    }
}

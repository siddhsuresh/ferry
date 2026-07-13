//! The device lifecycle exports — `initialize`, `dispose`, `fetch_device_info`,
//! and `fetch_storages`.
//!
//! `keel-usb` discovers/opens/claims/probes and hands back a live [`Transport`],
//! and `keel-mtp` runs the session ladder. [`Device`] here is a thin wrapper over
//! a configured `MtpSession<UsbTransport>`, and these four are its constructor +
//! methods.
//!
//! Error wrapping: discover failure → [`VfsError::MtpDetectFailed`] (except the
//! exclusive-access case, peeled off as [`VfsError::ExclusiveAccess`]);
//! session-ladder failure → [`VfsError::Configure`]; GetDeviceInfo →
//! [`VfsError::DeviceInfo`]; the storage ops → [`VfsError::StorageInfo`] /
//! [`VfsError::NoStorage`].

use keel_mtp::MtpSession;
use keel_mtp::session::UsbInfo as MtpUsbInfo;
use keel_proto::{DeviceInfo, StorageInfo};
use keel_usb::{DiscoverError, Discovered, UsbInfo, UsbTransport, discover};

use crate::error::VfsError;

/// Options for [`initialize`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Init {
    /// Request wire-level debug logging. keel routes wire debugging through the
    /// `log`/`env_logger` facade, so this flag is currently advisory.
    pub debug_mode: bool,
}

/// A storage ID paired with its info.
#[derive(Clone, Debug)]
pub struct StorageData {
    pub sid: u32,
    pub info: StorageInfo,
}

/// A live MTP device: a configured session over the USB transport, plus the USB
/// identity captured at discovery.
pub struct Device {
    session: MtpSession<UsbTransport>,
}

/// Map a keel-usb discovery failure into the vfs taxonomy.
///
/// Every discovery failure becomes [`VfsError::MtpDetectFailed`], except the
/// ptpcamerad / Image-Capture exclusive-access one, which is peeled off into
/// [`VfsError::ExclusiveAccess`] so the FFI can name the blocking process instead
/// of reporting a generic detect failure.
impl From<DiscoverError> for VfsError {
    fn from(e: DiscoverError) -> Self {
        match e {
            DiscoverError::ExclusiveAccess { owner } => VfsError::ExclusiveAccess { owner },
            other => VfsError::MtpDetectFailed(other.to_string()),
        }
    }
}

/// Initialize a device.
///
/// Discover the device ([`discover`]), then run the session ladder
/// ([`MtpSession::configure`]). The 15 s session timeout is applied inside
/// `configure` (keel-mtp `SESSION_TIMEOUT`). The USB identity keel-usb returns
/// alongside the transport is installed onto the session so later `usb_info()`
/// reads (the FFI `usbDeviceInfo` call, the device-change serial compare) have it.
pub fn initialize(opts: Init) -> Result<Device, VfsError> {
    let Init { debug_mode } = opts;
    if debug_mode {
        log::debug!("keel-vfs initialize: debug mode requested");
    }

    // Device selection (+ open/claim/probe). The `?` maps DiscoverError via `From`
    // above: ExclusiveAccess → ExclusiveAccess, everything else → MtpDetectFailed.
    let Discovered {
        transport,
        usb_info,
    } = discover()?;

    // Configure: the OpenSession recovery ladder.
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
    /// Initialize a device — see the free [`initialize`] function.
    pub fn initialize(opts: Init) -> Result<Device, VfsError> {
        initialize(opts)
    }

    /// Close the device, consuming it.
    pub fn dispose(self) {
        self.session.close();
    }

    /// Fetch the device info.
    pub fn fetch_device_info(&mut self) -> Result<DeviceInfo, VfsError> {
        self.session.device_info().map_err(VfsError::DeviceInfo)
    }

    /// Fetch the storages: GetStorageIDs, error out if none, then GetStorageInfo
    /// per ID.
    pub fn fetch_storages(&mut self) -> Result<Vec<StorageData>, VfsError> {
        let sids = self.session.storage_ids().map_err(VfsError::StorageInfo)?;

        // No storages available.
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

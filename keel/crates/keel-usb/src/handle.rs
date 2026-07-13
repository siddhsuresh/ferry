//! Device open / claim / string-descriptor / `UsbInfo` layer.
//!
//! Ports the USB-handle half of go-mtpfs `mtp/mtp.go` — `Device.Open`,
//! `Device.claim`, `Device.ID`, `Device.GetUsbInfo` — onto nusb 0.2.4.
//!
//! Two deliberate departures from the Go reference, both mandated by the plan:
//!
//! 1. **Claim errors are NOT tolerated.** Go's `Open()` ignores the return of
//!    `d.claim()` (mtp.go:170 — `d.claim()` with the error discarded) because
//!    libusb lets you read string descriptors over endpoint 0 without owning
//!    the interface. nusb requires a claimed interface before any endpoint I/O,
//!    and — more importantly — a failed claim with `kIOReturnExclusiveAccess`
//!    is exactly the ptpcamerad/Image-Capture signal Ferry has never surfaced
//!    (plan risk #2). So we treat claim failures as first-class outcomes.
//!
//! 2. **The nusb #206 claim ladder** (plan §3.1, docs/nusb-api.md §3): on macOS
//!    a vendor-class / class-0 device may not have its `IOUSBHostInterface`
//!    service published, so the first `claim_interface` fails `NotFound`. Issue
//!    `set_configuration` to force IOKit to publish it, then re-claim.

use nusb::{Device, DeviceInfo, ErrorKind, Interface, MaybeFuture};

use crate::discover::exclusive;

/// USB device identity, mirroring go-mtpfs `UsbDeviceInfo` (mtp.go:49-63) with
/// the field names the CONTRACTS.md `keel-usb` block fixes.
///
/// `bcd_device` is the `bcdDevice` release number (Go's `Device` field). nusb
/// exposes it as `DeviceInfo::device_version()`; docs/nusb-api.md flags this as
/// unverified against the mtp-rs reference, so it is best-effort — verify on the
/// phone at M2.
#[derive(Clone, Debug)]
pub struct UsbInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
}

/// Run a nusb blocking-syscall `MaybeFuture` to completion.
///
/// docs/nusb-api.md §0: `open`, `claim_interface`, `set_configuration`,
/// `clear_halt`, `control_*`, and `list_devices` are blocking ioctls dressed as
/// futures. They MUST be `.wait()`-ed — `.await`-ing one panics at runtime
/// ("Awaiting blocking syscall without an async runtime") because keel does not
/// enable nusb's `tokio`/`smol` feature. Funnelling every such call through this
/// one helper removes the per-site `.wait()`-vs-`.await` footgun (nusb #212).
pub(crate) fn blocking<F: MaybeFuture>(op: F) -> F::Output {
    op.wait()
}

/// How opening / claiming a candidate device failed. Kept internal; `discover`
/// folds these into the public `DiscoverError` after scanning every candidate.
pub(crate) enum OpenError {
    /// `kIOReturnExclusiveAccess` (nusb `ErrorKind::Busy`) — another process
    /// (ptpcamerad, Image Capture, Photos, Smart Switch, …) holds the device.
    /// `owner` is the IORegistry-named holder, best-effort (see `exclusive`).
    Exclusive { owner: Option<String> },
    /// macOS user-client denial — nusb could not create the IOKit
    /// PlugInInterface. Distinct from exclusive access: the kernel owns the
    /// device or the app lacks the right to touch it. `String` is nusb's message.
    PlugIn(String),
    /// Device left the bus mid-scan (`Disconnected` / post-enumeration `NotFound`).
    Gone,
    /// Any other nusb error, carrying its description.
    Other(String),
}

/// Classify a nusb open/claim/config `Error` into an `OpenError`.
///
/// `di` is needed so the `Busy` (exclusive-access) arm can name the holder from
/// the IORegistry.
pub(crate) fn classify_error(e: nusb::Error, di: &DeviceInfo) -> OpenError {
    match e.kind() {
        // Busy == macOS kIOReturnExclusiveAccess (docs/nusb-api.md §4). This is
        // THE ptpcamerad case. nusb's macOS backend maps it here for both the
        // interface-open path and require_open_exclusive.
        ErrorKind::Busy => OpenError::Exclusive {
            owner: exclusive::owner_of(di),
        },
        // Device unplugged mid-scan. Post-enumeration NotFound is defensively
        // treated as gone too (docs/nusb-api.md §6).
        ErrorKind::Disconnected | ErrorKind::NotFound => OpenError::Gone,
        _ => {
            let msg = e.to_string();
            // nusb macOS emits "failed to create IOKit PlugInInterface for
            // device" (iokit_usb.rs) when IOCreatePlugInInterfaceForService is
            // denied — a user-client/security-model denial, not exclusive access.
            if msg.contains("PlugInInterface") {
                OpenError::PlugIn(msg)
            } else {
                OpenError::Other(msg)
            }
        }
    }
}

/// Claim `interface_number`, applying the nusb #206 macOS ladder.
///
/// go-mtpfs claims in `Device.claim` (mtp.go:135-149) and separately activates
/// the config in `select.go` (SetConfiguration when `config != configValue`).
/// Here the config activation is folded into the ladder: the only time we need
/// to set the configuration is when the interface service isn't published, which
/// nusb reports as `NotFound` on the first claim. We set the *candidate's* config
/// value (Go's `configValue`) rather than a hard-coded 1 so multi-config devices
/// land on the right one; for Android phones this is 1 either way.
pub(crate) fn claim_with_ladder(
    device: &Device,
    di: &DeviceInfo,
    interface_number: u8,
    config_value: u8,
) -> Result<Interface, OpenError> {
    match blocking(device.claim_interface(interface_number)) {
        Ok(iface) => Ok(iface),
        // nusb #206: macOS hasn't published the IOUSBHostInterface service for
        // this vendor-class / class-0 device. Force the configuration so IOKit
        // publishes it, then re-claim (docs/nusb-api.md §3).
        Err(e) if matches!(e.kind(), ErrorKind::NotFound) => {
            blocking(device.set_configuration(config_value)).map_err(|e2| classify_error(e2, di))?;
            blocking(device.claim_interface(interface_number)).map_err(|e2| classify_error(e2, di))
        }
        Err(e) => Err(classify_error(e, di)),
    }
}

/// Build `UsbInfo` from the cached enumeration fields.
///
/// go-mtpfs `GetUsbInfo` (mtp.go:238-289) re-reads the manufacturer/product/
/// serial string descriptors from the opened handle by index. nusb caches those
/// strings on `DeviceInfo` at enumeration (macOS/Linux populate them), so we read
/// them there — no extra control transfers, and behaviourally identical (same
/// descriptor bytes). Missing strings map to `""`, matching Go's index-0 branch.
pub(crate) fn read_usb_info(di: &DeviceInfo) -> UsbInfo {
    UsbInfo {
        vendor_id: di.vendor_id(),
        product_id: di.product_id(),
        // bcdDevice — Go's UsbDeviceInfo.Device. Best-effort per docs/nusb-api.md.
        bcd_device: di.device_version(),
        manufacturer: di.manufacturer_string().unwrap_or_default().to_string(),
        product: di.product_string().unwrap_or_default().to_string(),
        serial: di.serial_number().unwrap_or_default().to_string(),
    }
}

/// The interface string for `interface_number`, if the OS published one.
///
/// Go keys the MTP-ness probe on `ifaceDescr.InterfaceStringIndex == 0`
/// (mtp.go:172): index 0 ⇒ no string ⇒ fall through to the GetDeviceInfo probe;
/// otherwise read the string and require it to contain MTP/CDC/ACM. nusb's cached
/// `interface_string()` is `None` exactly when the index is 0, so this maps
/// cleanly and avoids issuing GetStringDescriptorASCII ourselves.
pub(crate) fn interface_string_for(di: &DeviceInfo, interface_number: u8) -> Option<String> {
    di.interfaces()
        .find(|i| i.interface_number() == interface_number)
        .and_then(|i| i.interface_string())
        .map(str::to_string)
}

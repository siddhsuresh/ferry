//! Device discovery + selection.
//!
//! Ports go-mtpfs `mtp/select.go` (`candidateFromDeviceDescriptor`,
//! `FindDevices`, `selectDevice`) and the MTP-ness probe from `mtp/mtp.go`
//! `Device.Open`, onto nusb 0.2.4.
//!
//! Candidate matching is by **endpoint shape**, never by class: the target
//! phones (VID 18d1, confirmed) are vendor class 0xFF, so class 6 (Still Image)
//! is unreliable. A candidate interface has exactly three endpoints —
//! bulk-IN + bulk-OUT + interrupt-IN — exactly Go's `candidateFromDeviceDescriptor`
//! (`len(a.EndPoints) != 3 { continue }`, then require send/fetch/event EPs).

use nusb::descriptors::TransferType;
use nusb::transfer::Direction;
use nusb::{Device, DeviceInfo};

use crate::handle::{
    blocking, classify_error, claim_with_ladder, interface_string_for, read_usb_info, OpenError,
};
use crate::pipes::UsbTransport;

// The IOKit exclusive-owner lookup lives in src/exclusive.rs. lib.rs only
// declares discover/pipes/handle, so we attach exclusive.rs here (its file sits
// next to this one in src/). `handle` reaches it via `crate::discover::exclusive`.
#[path = "exclusive.rs"]
pub(crate) mod exclusive;

pub use crate::handle::UsbInfo;

/// A selected, opened, claimed MTP device: a live transport plus its identity.
pub struct Discovered {
    pub transport: UsbTransport,
    pub usb_info: UsbInfo,
}

/// Discovery failures. `NoDevice` / `MultipleDevices` Display text is fixed by
/// CONTRACTS.md (the FFI error mapper string-matches it).
///
/// `ExclusiveAccess` is a keel **extension** to the CONTRACTS.md enum (documented
/// there as the addition keel-usb makes): Ferry has zero exclusive-access
/// handling today (plan risk #2), and this carries the IORegistry-named holder
/// up so keel-vfs → keel-ffi can surface "Quit <app> and try again".
pub enum DiscoverError {
    /// Display == "no MTP devices found" (go-mtpfs select.go:90).
    NoDevice,
    /// Display contains "more than 1 device" (go-mtpfs select.go:117).
    MultipleDevices(usize),
    /// ptpcamerad / Image Capture / Photos / Smart Switch holds the device.
    /// `owner` is the IORegistry-named process, best-effort (may be `None`).
    ExclusiveAccess { owner: Option<String> },
    /// User-client / permission denial (e.g. the IOKit PlugInInterface case) —
    /// carries user-facing guidance. NOT exclusive access.
    Access(String),
    /// Anything else (enumeration failure, endpoint-open failure, …).
    Other(String),
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // EXACT string — matched verbatim downstream.
            DiscoverError::NoDevice => write!(f, "no MTP devices found"),
            // Must CONTAIN "more than 1 device"; keep the "mtp:" prefix from Go.
            DiscoverError::MultipleDevices(n) => {
                write!(f, "mtp: more than 1 device: found {n} MTP candidates")
            }
            DiscoverError::ExclusiveAccess { owner } => match owner {
                Some(who) => write!(
                    f,
                    "device is held exclusively by another process ({who}); quit it and try again"
                ),
                None => write!(
                    f,
                    "device is held exclusively by another process; quit Image Capture, Photos, or any phone-sync app and try again"
                ),
            },
            DiscoverError::Access(msg) => write!(f, "{msg}"),
            DiscoverError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::fmt::Debug for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DiscoverError({self})")
    }
}

impl std::error::Error for DiscoverError {}

/// Find one MTP device, faithful to go-mtpfs `SelectDevice("")`.
///
/// Enumerate → build endpoint-shape candidates → open/claim/probe each → count.
/// Zero opened ⇒ `NoDevice` (or a surfaced exclusive/plugin/other error if one
/// blocked the only plausible device). Exactly one ⇒ return it. More than one ⇒
/// `MultipleDevices` (Go hard-errors here; relaxed post-parity per plan §10).
pub fn discover() -> Result<Discovered, DiscoverError> {
    let devices = blocking(nusb::list_devices()).map_err(|e| DiscoverError::Other(e.to_string()))?;

    let mut found: Vec<Discovered> = Vec::new();
    // Remembered blockers, surfaced only if NOTHING opened. Precedence:
    // exclusive-access (the ptpcamerad case the user must act on) > plugin
    // denial > other. A named exclusive owner is preferred over an unnamed one.
    let mut exclusive: Option<Option<String>> = None;
    let mut plugin: Option<String> = None;
    let mut other: Option<String> = None;

    for di in devices {
        if !plausible(&di) {
            continue;
        }
        match try_open_candidate(&di) {
            Ok(Some(d)) => found.push(d),
            Ok(None) => {} // opened but not an MTP interface / probe rejected it
            Err(OpenError::Exclusive { owner }) => {
                if !matches!(exclusive, Some(Some(_))) {
                    exclusive = Some(owner);
                }
            }
            Err(OpenError::PlugIn(m)) => {
                plugin.get_or_insert(m);
            }
            Err(OpenError::Gone) => {} // Go's `continue` — device left mid-scan
            Err(OpenError::Other(m)) => {
                other.get_or_insert(m);
            }
        }
    }

    match found.len() {
        1 => Ok(found.into_iter().next().expect("len checked == 1")),
        0 => {
            if let Some(owner) = exclusive {
                Err(DiscoverError::ExclusiveAccess { owner })
            } else if let Some(msg) = plugin {
                Err(DiscoverError::Access(plugin_guidance(&msg)))
            } else if let Some(msg) = other {
                Err(DiscoverError::Other(msg))
            } else {
                Err(DiscoverError::NoDevice)
            }
        }
        n => Err(DiscoverError::MultipleDevices(n)),
    }
}

/// Cheap pre-filter over the enumeration summary (no device open).
///
/// go-mtpfs inspects the config descriptors of *every* device on the bus. We
/// can't see endpoints without opening on nusb, so we first skip the obvious
/// non-candidates (hubs, keyboards, audio, …) to avoid opening — and possibly
/// perturbing — unrelated devices. The endpoint-shape check below remains the
/// actual decider; this only narrows what we bother opening.
fn plausible(di: &DeviceInfo) -> bool {
    // Hubs never carry an MTP interface.
    if di.class() == 0x09 {
        return false;
    }
    // Composite (0x00), Still Image (0x06), or vendor (0xFF) at the device level:
    // worth opening. Android phones are 0x00 or 0xFF; PTP cameras are 0x06.
    match di.class() {
        0x00 | 0x06 | 0xFF => true,
        _ => di.interfaces().any(|i| {
            matches!(i.class(), 0x06 | 0xFF)
                || i.interface_string().is_some_and(|s| {
                    let u = s.to_ascii_uppercase();
                    u.contains("MTP") || u.contains("PTP")
                })
        }),
    }
}

/// A candidate MTP interface located by endpoint shape.
struct Candidate {
    interface_number: u8,
    config_value: u8,
    bulk_in: u8,
    bulk_out: u8,
    interrupt_in: u8,
}

/// Scan every configuration/alt-setting for the MTP endpoint shape.
///
/// Faithful to `candidateFromDeviceDescriptor` (select.go:11-49): exactly three
/// endpoints — one bulk-IN, one bulk-OUT, one interrupt-IN. Returns the first
/// match, in descriptor order.
fn find_mtp_interface(device: &Device) -> Option<Candidate> {
    for config in device.configurations() {
        let config_value = config.configuration_value();
        for alt in config.interface_alt_settings() {
            // Go: `if len(a.EndPoints) != 3 { continue }`.
            if alt.endpoints().count() != 3 {
                continue;
            }
            let (mut bulk_in, mut bulk_out, mut interrupt_in) = (None, None, None);
            for ep in alt.endpoints() {
                match (ep.direction(), ep.transfer_type()) {
                    (Direction::In, TransferType::Bulk) => bulk_in = Some(ep.address()),
                    (Direction::Out, TransferType::Bulk) => bulk_out = Some(ep.address()),
                    (Direction::In, TransferType::Interrupt) => interrupt_in = Some(ep.address()),
                    _ => {}
                }
            }
            if let (Some(bulk_in), Some(bulk_out), Some(interrupt_in)) =
                (bulk_in, bulk_out, interrupt_in)
            {
                return Some(Candidate {
                    interface_number: alt.interface_number(),
                    config_value,
                    bulk_in,
                    bulk_out,
                    interrupt_in,
                });
            }
        }
    }
    None
}

/// Open + claim + MTP-ness-probe one candidate device.
///
/// `Ok(Some)` = a usable MTP transport; `Ok(None)` = opened but not MTP (no
/// endpoint shape, or an interface string that lacks MTP/CDC/ACM); `Err` = a
/// blocker (exclusive access, plugin denial, gone, other).
fn try_open_candidate(di: &DeviceInfo) -> Result<Option<Discovered>, OpenError> {
    // nusb's macOS open() succeeds even under exclusive access (it defers the
    // exclusive device-open); the Busy error surfaces at claim, below.
    let device = blocking(di.open()).map_err(|e| classify_error(e, di))?;

    let Some(cand) = find_mtp_interface(&device) else {
        return Ok(None);
    };

    let interface = claim_with_ladder(&device, di, cand.interface_number, cand.config_value)?;

    // MTP-ness probe (go-mtpfs Open, mtp.go:172-200).
    match interface_string_for(di, cand.interface_number) {
        Some(s) => {
            // Old-Samsung allowance: CDC/ACM count as MTP (mtp.go:196).
            if !(s.contains("MTP") || s.contains("CDC") || s.contains("ACM")) {
                return Ok(None);
            }
        }
        None => {
            // SEAM (plan `keel-usb probe`): Go's InterfaceStringIndex == 0 branch
            // runs a pre-session GetDeviceInfo and accepts
            // microsoft/WindowsPhone or fujifilm.co.jp extensions. Wiring a real
            // GetDeviceInfo here needs keel-mtp's transaction engine (a bare,
            // session-less RunTransaction on this very transport), which is being
            // written concurrently; do it at the gate. The endpoint shape has
            // already matched, so we accept — see open issues.
        }
    }

    let transport = UsbTransport::open(
        device,
        interface,
        cand.interface_number,
        cand.bulk_in,
        cand.bulk_out,
        cand.interrupt_in,
    )
    .map_err(|e| OpenError::Other(e.to_string()))?;

    let usb_info = read_usb_info(di);
    Ok(Some(Discovered { transport, usb_info }))
}

/// Turn nusb's raw PlugInInterface message into user-facing guidance.
///
/// This is the macOS user-client denial: the process lacks the right to create
/// an IOKit user client for the device (typically when run bare from a terminal
/// rather than a signed .app). NOT exclusive access — no other app to quit.
fn plugin_guidance(raw: &str) -> String {
    format!(
        "macOS denied USB access to this device ({raw}). Launch Ferry as a signed app \
         (not from a bare terminal) and, if it persists, reconnect the device."
    )
}

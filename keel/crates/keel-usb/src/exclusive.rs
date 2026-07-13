//! Naming the process that holds a USB device exclusively (ptpcamerad, Image
//! Capture, Photos, Smart Switch, …) via the macOS IORegistry.
//!
//! When `claim_interface` fails with `kIOReturnExclusiveAccess` (nusb
//! `ErrorKind::Busy`), we open the device's IORegistry entry and read a string
//! property naming the holder so keel-vfs → keel-ffi can tell the user exactly
//! which app to quit.
//!
//! ## HARDWARE-UNVERIFIED — flagged for the human's on-device pass
//!
//! The exact property key macOS uses is not something we can confirm without the
//! phone + a live ptpcamerad grab. We try, in order, `"USB Exclusive Owner"` (the
//! historically-documented IOUSBFamily key), `"kUSBExclusiveOwner"` (its
//! constant-name spelling), and `"IOUserClientCreator"` (the generic IOKit
//! property naming the process that owns a user client, formatted
//! `"pid <pid>, <name>"`), returning the first non-empty string, searching the
//! device node and its interface children (`kIORegistryIterateRecursively`). If
//! none is present the owner is `None`, which the contract permits
//! (`ExclusiveAccess { owner: None }`).
//!
//! The device itself is matched precisely by nusb's `registry_entry_id()` (the
//! IORegistryEntryID), so there is no ambiguity about *which* device we inspect —
//! only about which property, if any, carries the holder's name.

use nusb::DeviceInfo;

/// Best-effort name of the process holding `di` exclusively. `None` if it can't
/// be determined (or off macOS).
#[cfg(target_os = "macos")]
pub(crate) fn owner_of(di: &DeviceInfo) -> Option<String> {
    macos::exclusive_owner(di.registry_entry_id())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn owner_of(_di: &DeviceInfo) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr;

    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::CFString;
    use io_kit_sys::keys::kIOServicePlane;
    use io_kit_sys::types::io_registry_entry_t;
    use io_kit_sys::{
        kIOMasterPortDefault, kIORegistryIterateRecursively, IOObjectRelease,
        IORegistryEntryIDMatching, IORegistryEntrySearchCFProperty, IOServiceGetMatchingService,
    };

    /// Candidate property keys, most-specific first. See the module docs for why
    /// this is a list rather than one key (unverified on real hardware).
    const OWNER_KEYS: &[&str] = &[
        "USB Exclusive Owner",
        "kUSBExclusiveOwner",
        "IOUserClientCreator",
    ];

    pub(super) fn exclusive_owner(registry_entry_id: u64) -> Option<String> {
        // SAFETY: raw IOKit FFI. `IORegistryEntryIDMatching` returns a +1
        // CFMutableDictionary that `IOServiceGetMatchingService` consumes (so we
        // must NOT release it). The returned `service` is +1 and released below.
        unsafe {
            let matching = IORegistryEntryIDMatching(registry_entry_id);
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(kIOMasterPortDefault, matching.cast_const());
            if service == 0 {
                return None;
            }
            let owner = read_owner(service);
            IOObjectRelease(service);
            owner
        }
    }

    /// Search `service` (and its children, in the service plane) for the first
    /// owner property that resolves to a non-empty string.
    ///
    /// SAFETY: `service` is a live io_registry_entry_t. Each search returns a +1
    /// CFType (Create Rule) which `wrap_under_create_rule` takes ownership of and
    /// releases on drop.
    unsafe fn read_owner(service: io_registry_entry_t) -> Option<String> {
        for key in OWNER_KEYS {
            let cf_key = CFString::new(key);
            // SAFETY: `service` is live; `cf_key` outlives the call; the result
            // is a +1 CFType we take ownership of below.
            let raw = unsafe {
                IORegistryEntrySearchCFProperty(
                    service,
                    kIOServicePlane,
                    cf_key.as_concrete_TypeRef(),
                    ptr::null(),                   // default allocator
                    kIORegistryIterateRecursively, // device node + interface children
                )
            };
            if raw.is_null() {
                continue;
            }
            // SAFETY: `raw` is a non-null +1 CFTypeRef from the Create-rule search.
            let value = unsafe { CFType::wrap_under_create_rule(raw) };
            if let Some(text) = value.downcast::<CFString>() {
                if let Some(name) = clean_owner(&text.to_string()) {
                    return Some(name);
                }
            }
        }
        None
    }

    /// Trim and, for the `IOUserClientCreator` "pid 1234, ptpcamerad" form,
    /// extract just the process name.
    fn clean_owner(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some((prefix, name)) = trimmed.rsplit_once(", ") {
            if prefix.trim_start().starts_with("pid ") {
                let name = name.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        Some(trimmed.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::clean_owner;

        #[test]
        fn extracts_process_name_from_iouserclientcreator() {
            assert_eq!(
                clean_owner("pid 431, ptpcamerad").as_deref(),
                Some("ptpcamerad")
            );
        }

        #[test]
        fn passes_through_plain_owner_string() {
            assert_eq!(clean_owner("Image Capture").as_deref(), Some("Image Capture"));
        }

        #[test]
        fn empty_is_none() {
            assert_eq!(clean_owner("   "), None);
        }

        #[test]
        fn does_not_split_names_containing_comma_space() {
            // Only the "pid N, name" shape is special-cased.
            assert_eq!(
                clean_owner("Adobe, Inc. Helper").as_deref(),
                Some("Adobe, Inc. Helper")
            );
        }
    }
}

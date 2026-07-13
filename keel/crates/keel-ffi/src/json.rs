//! keel-ffi JSON layer — the exact wire contract Swift decodes.
//!
//! This module owns the whole JSON contract: the wire **types**, the serializer
//! ([`to_json`]), case-insensitive input decode ([`decode_input`]), and the
//! payload **builders** (one per operation). `abi.rs` reads input, calls
//! `state`, and hands the domain result to the matching builder here; so the
//! domain→wire mapping, the nil-slice `null` quirk, the literal-`Z` `dateAdded`,
//! and elapsed-time all live in this one place.
//!
//! # Three casing regimes (docs/CONTRACTS.md keel-ffi/json)
//! The wire contract mixes three casing conventions in one payload tree, and
//! Swift decodes them position-for-position, so each must be reproduced exactly:
//!
//! 1. **Envelope + camelCase payload structs.** `errorType`, `error`, `data`,
//!    `isFolder`, `dateAdded`, `fullPath`, `filesSentProgress`, …
//! 2. **`mtpDeviceInfo` / `usbDeviceInfo` / storages → raw PascalCase.** The
//!    device-info and storage structs carry no rename tags, so the field name is
//!    emitted verbatim: `StandardVersion`, `MTPVendorExtensionID`, `Sid`, `Info`,
//!    `MaxCapability`, `FreeSpaceInBytes`, `StorageDescription`, … (golden
//!    fixtures 0001–0003).
//! 3. **`FileExists` data → all-lowercase `fullpath`.** The one field whose key
//!    is literally lowercase, unlike every other `fullPath` in the tree
//!    (golden 0008).
//!
//! # `null` vs `[]` (the nil-slice quirk — required by the wire contract)
//! The Walk and FileExists builders emit `"data":null`, not `[]`, for an empty
//! result — the frozen contract distinguishes a nil slice (`null`) from a
//! non-nil empty one (`[]`). Modeled with [`null_if_empty`] → `Option<Vec<_>>`.
//! By contrast the device-info arrays (`CaptureFormats`, …) arrive pre-populated
//! (non-nil) from the wire decoder, so an empty one is `[]` (golden 0001
//! `"CaptureFormats":[]`) — those stay plain `Vec`.
//!
//! # Float formatting (6-digit lossy)
//! The wire contract formats floats by rounding to 6 fractional decimal digits,
//! stripping trailing zeros, and dropping the fraction entirely when integral
//! (so `100`, not `100.0`; `9.63`; `83.330971`). serde_json's native (ryu
//! shortest round-trip) output DIFFERS (e.g. `66.66667` vs the required
//! `66.666672`), so [`to_json`] installs [`CompactFloatFormatter`], a custom
//! `serde_json` `Formatter` that reproduces the 6-digit lossy form →
//! byte-identical floats.
//!
//! # No HTML escaping
//! The wire contract leaves `< > &` and non-ASCII (emoji filenames) as raw
//! UTF-8. serde_json's default `CompactFormatter` does the same, so no
//! configuration is needed.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ===========================================================================
// Serializer: 6-digit lossy float formatting
// ===========================================================================

/// A `serde_json` `Formatter` that emits floats in the wire contract's 6-digit
/// lossy form, and everything else exactly like the default `CompactFormatter`
/// (all non-float methods use the trait defaults).
///
/// Load-bearing: the golden progress payloads (fixtures 0011–0013) carry
/// `filesSentProgress`/`progress` percentages that are NOT normalized by the
/// conformance oracle, so they must be byte-identical.
struct CompactFloatFormatter;

impl serde_json::ser::Formatter for CompactFloatFormatter {
    fn write_f32<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: f32,
    ) -> std::io::Result<()> {
        writer.write_all(float32_lossy(value).as_bytes())
    }

    fn write_f64<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: f64,
    ) -> std::io::Result<()> {
        writer.write_all(float64_lossy(value).as_bytes())
    }
}

/// Append the positive magnitude `mag` in the 6-digit lossy form.
///
/// `lval = (mag*1e6 + 0.5) as u64`, print `lval/1e6`, then the 6-digit fraction
/// `lval%1e6` left-padded with zeros and right-stripped of zeros.
fn append_lossy(out: &mut String, mag: f64) {
    const EXP: u64 = 1_000_000; // 10^6, precision = 6
    const POW10: [u64; 7] = [1, 10, 100, 1000, 10000, 100000, 1000000];

    // `(mag * exp + 0.5) as u64` — truncation toward zero of a positive value
    // == round-half-up. `as u64` truncates and saturates.
    let lval = (mag * EXP as f64 + 0.5) as u64;
    out.push_str(&(lval / EXP).to_string());

    let fval = lval % EXP;
    if fval == 0 {
        return; // integral → no fractional part ("100", "0")
    }
    out.push('.');
    // Left-pad the fraction to 6 digits.
    let mut p: i32 = 5; // precision - 1
    while p > 0 && fval < POW10[p as usize] {
        out.push('0');
        p -= 1;
    }
    out.push_str(&fval.to_string());
    // Strip trailing zeros (fval != 0 guarantees this never eats the '.').
    while out.ends_with('0') {
        out.pop();
    }
}

/// 6-digit lossy formatting for `f32`. `0x4ffffff` = 83_886_079 is the cutover
/// to full-precision formatting; above it (and for non-finite values, which
/// never reach a progress/speed field) we fall back to a best-effort shortest
/// form — an effectively-dead deviation.
fn float32_lossy(val: f32) -> String {
    let mut out = String::new();
    let (neg, mag) = if val < 0.0 { (true, -val) } else { (false, val) };
    if neg {
        out.push('-');
    }
    if !mag.is_finite() || mag > 83_886_079.0_f32 {
        out.push_str(&format!("{mag}"));
        return out;
    }
    append_lossy(&mut out, mag as f64);
    out
}

/// 6-digit lossy formatting for `f64`. See [`float32_lossy`] for the cutover.
fn float64_lossy(val: f64) -> String {
    let mut out = String::new();
    let (neg, mag) = if val < 0.0 { (true, -val) } else { (false, val) };
    if neg {
        out.push('-');
    }
    if !mag.is_finite() || mag > 83_886_079.0_f64 {
        out.push_str(&format!("{mag}"));
        return out;
    }
    append_lossy(&mut out, mag);
    out
}

/// Serialize any payload to the wire contract: compact, no HTML escaping,
/// 6-digit lossy floats. Returns `""` on marshal failure; these plain structs
/// never fail to serialize, but the fallback is here rather than panicking
/// across the FFI boundary.
pub fn to_json<T: Serialize>(value: &T) -> String {
    let mut buf = Vec::new();
    {
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, CompactFloatFormatter);
        if value.serialize(&mut ser).is_err() {
            return String::new();
        }
    }
    String::from_utf8(buf).unwrap_or_default()
}

// ===========================================================================
// Output envelope
// ===========================================================================

/// The `{errorType, error, data}` result envelope shared by every operation's
/// result. One generic covers them all; field order (errorType, error, data) is
/// fixed by the wire contract.
///
/// `error_type` is `&'static str` because the value is always either `""`
/// (success) or one of [`crate::errors`]' fixed `ErrorType` constants.
#[derive(Debug, Serialize)]
pub struct Envelope<T: Serialize> {
    #[serde(rename = "errorType")]
    pub error_type: &'static str,
    pub error: String,
    pub data: T,
}

impl<T: Serialize> Envelope<T> {
    /// A success envelope: `errorType` and `error` are the empty string.
    pub fn ok(data: T) -> Self {
        Envelope {
            error_type: "",
            error: String::new(),
            data,
        }
    }
}

/// The error envelope: `data` is `null`. `()` serializes to `null`.
pub fn error_envelope(error_type: &'static str, error: String) -> Envelope<()> {
    Envelope {
        error_type,
        error,
        data: (),
    }
}

// ===========================================================================
// Regime 2: raw PascalCase — device info / USB device info / storages
// ===========================================================================

/// The `data` of Initialize / FetchDeviceInfo. On the success path both fields
/// are always populated, so they're kept non-optional.
#[derive(Debug, Serialize)]
pub struct DeviceInfoData {
    #[serde(rename = "mtpDeviceInfo")]
    pub mtp_device_info: MtpDeviceInfo,
    #[serde(rename = "usbDeviceInfo")]
    pub usb_device_info: UsbDeviceInfo,
}

/// Raw PascalCase device info. No rename tags ⇒ the field names are the wire
/// keys (golden 0001). Arrays are plain `Vec` (never nil off the wire decoder)
/// so an empty one serializes `[]`, not `null`.
#[derive(Debug, Serialize)]
pub struct MtpDeviceInfo {
    #[serde(rename = "StandardVersion")]
    pub standard_version: u16,
    #[serde(rename = "MTPVendorExtensionID")]
    pub mtp_vendor_extension_id: u32,
    #[serde(rename = "MTPVersion")]
    pub mtp_version: u16,
    #[serde(rename = "MTPExtension")]
    pub mtp_extension: String,
    #[serde(rename = "FunctionalMode")]
    pub functional_mode: u16,
    #[serde(rename = "OperationsSupported")]
    pub operations_supported: Vec<u16>,
    #[serde(rename = "EventsSupported")]
    pub events_supported: Vec<u16>,
    #[serde(rename = "DevicePropertiesSupported")]
    pub device_properties_supported: Vec<u16>,
    #[serde(rename = "CaptureFormats")]
    pub capture_formats: Vec<u16>,
    #[serde(rename = "PlaybackFormats")]
    pub playback_formats: Vec<u16>,
    #[serde(rename = "Manufacturer")]
    pub manufacturer: String,
    #[serde(rename = "Model")]
    pub model: String,
    #[serde(rename = "DeviceVersion")]
    pub device_version: String,
    #[serde(rename = "SerialNumber")]
    pub serial_number: String,
}

/// Raw PascalCase USB device info. `Device` is bcdDevice (golden 0001
/// `"Device":1537`).
#[derive(Debug, Serialize)]
pub struct UsbDeviceInfo {
    #[serde(rename = "IdVendor")]
    pub id_vendor: u16,
    #[serde(rename = "IdProduct")]
    pub id_product: u16,
    #[serde(rename = "Device")]
    pub device: u16,
    #[serde(rename = "Manufacturer")]
    pub manufacturer: String,
    #[serde(rename = "Product")]
    pub product: String,
    #[serde(rename = "SerialNumber")]
    pub serial_number: String,
}

/// Raw PascalCase storage entry — the element of the FetchStorages `data` array
/// (golden 0003).
#[derive(Debug, Serialize)]
pub struct StorageData {
    #[serde(rename = "Sid")]
    pub sid: u32,
    #[serde(rename = "Info")]
    pub info: StorageInfo,
}

/// Raw PascalCase storage info.
#[derive(Debug, Serialize)]
pub struct StorageInfo {
    #[serde(rename = "StorageType")]
    pub storage_type: u16,
    #[serde(rename = "FilesystemType")]
    pub filesystem_type: u16,
    #[serde(rename = "AccessCapability")]
    pub access_capability: u16,
    #[serde(rename = "MaxCapability")]
    pub max_capability: u64,
    #[serde(rename = "FreeSpaceInBytes")]
    pub free_space_in_bytes: u64,
    #[serde(rename = "FreeSpaceInImages")]
    pub free_space_in_images: u32,
    #[serde(rename = "StorageDescription")]
    pub storage_description: String,
    #[serde(rename = "VolumeLabel")]
    pub volume_label: String,
}

// ===========================================================================
// Regime 1: camelCase payloads
// ===========================================================================

/// Walk `data` element. Note the wire keys: `isFolder` (not isDir), `dateAdded`
/// (not modTime), `path` (not fullPath).
#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub size: i64,
    #[serde(rename = "isFolder")]
    pub is_dir: bool,
    /// Pre-formatted timestamp string ([`format_date_added`]).
    #[serde(rename = "dateAdded")]
    pub mod_time: String,
    pub name: String,
    #[serde(rename = "path")]
    pub full_path: String,
    #[serde(rename = "parentPath")]
    pub parent_path: String,
    pub extension: String,
    #[serde(rename = "parentId")]
    pub parent_id: u32,
    #[serde(rename = "objectId")]
    pub object_id: u32,
}

/// FileExists `data` element. Regime 3: `fullpath` is the one literally-lowercase
/// key in the whole contract (golden 0008).
#[derive(Debug, Serialize)]
pub struct FileExistsData {
    pub fullpath: String,
    pub exists: bool,
}

/// Upload/Download preprocess `data` (golden 0009/0010).
#[derive(Debug, Serialize)]
pub struct TransferPreprocessData {
    #[serde(rename = "fullPath")]
    pub full_path: String,
    pub name: String,
    pub size: i64,
}

/// Upload/Download progress `data` (golden 0011–0013). `speed` is f64; the two
/// percentage fields are f32, which the lossy formatter renders as `33.333336`,
/// `100`.
#[derive(Debug, Serialize)]
pub struct TransferProgressInfo {
    #[serde(rename = "fullPath")]
    pub full_path: String,
    pub name: String,
    #[serde(rename = "elapsedTime")]
    pub elapsed_time: i64,
    pub speed: f64,
    #[serde(rename = "totalFiles")]
    pub total_files: i64,
    #[serde(rename = "totalDirectories")]
    pub total_directories: i64,
    #[serde(rename = "filesSent")]
    pub files_sent: i64,
    #[serde(rename = "filesSentProgress")]
    pub files_sent_progress: f32,
    #[serde(rename = "activeFileSize")]
    pub active_file_size: TransferSizeInfo,
    #[serde(rename = "bulkFileSize")]
    pub bulk_file_size: TransferSizeInfo,
    /// The transfer status string `"InProgress"` or `"Completed"`. Carried as
    /// the raw string the builder supplies.
    pub status: String,
}

/// Per-file / bulk byte totals with a progress percentage.
#[derive(Debug, Serialize)]
pub struct TransferSizeInfo {
    pub total: i64,
    pub sent: i64,
    pub progress: f32,
}

// ===========================================================================
// Input decode (case-insensitive)
// ===========================================================================

/// Build the unmarshalling-error sentinel emitted when input JSON fails to
/// decode.
///
/// Preserved BYTE-FOR-BYTE for the wire contract, including the misspelling
/// **"occured"** and the trailing `": "` with nothing after it. `op` is the
/// operation name (`"MakeDirectory"`, `"Walk"`, …). The embedded error detail
/// differs from the original serializer's text — a deviation that never affects
/// classification (always `ErrorGeneral`), and Swift does not parse the message
/// body.
pub fn unmarshalling_error(op: &str, err: &serde_json::Error) -> String {
    format!("error occured while Unmarshalling {op} input data {err}: ")
}

/// Decode an input payload with lenient, case-insensitive semantics:
/// * **case-insensitive keys** — Swift sends `"files"`, the contract key is
///   `"Files"`; both must decode (docs/CONTRACTS.md keel-ffi/json). Also covers
///   `storageId`/`StorageId`, etc.
/// * **missing field → zero value** (`#[serde(default)]` on every input struct).
/// * **explicit `null` → zero value** — a null field is treated as absent;
///   [`normalize_input`] drops null entries so the struct default applies (a
///   plain serde decode would instead error on `null` into `String`/`u32`).
/// * **unknown fields ignored** (serde default).
///
/// The caller wraps any `Err` via [`unmarshalling_error`].
pub fn decode_input<T: DeserializeOwned>(json: &str) -> Result<T, serde_json::Error> {
    let raw: Value = serde_json::from_str(json)?;
    serde_json::from_value(normalize_input(raw))
}

/// Recursively lowercase object keys (ASCII fold) and drop `null`-valued entries
/// (null → zero-value leniency). Applied before deserializing into the
/// lowercase-renamed input structs. Input payloads are flat objects, so this is
/// cheap.
fn normalize_input(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if val.is_null() {
                    continue;
                }
                out.insert(k.to_ascii_lowercase(), normalize_input(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(normalize_input).collect()),
        other => other,
    }
}

// Field renames are the ASCII-lowercase form of each contract key, because
// `decode_input` lowercases the incoming keys first. `#[serde(default)]` gives
// the missing-field zero values.

/// `MakeDirectory` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MakeDirectoryInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "fullpath")]
    pub full_path: String,
}

/// `FetchThumbnail` input — a Ferry extension.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FetchThumbnailInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "fullpath")]
    pub full_path: String,
}

/// `FileExists` input. The contract key is `"Files"` (capital F); Swift sends
/// `"files"` — both fold to `"files"`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FileExistsInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "files")]
    pub files: Vec<String>,
}

/// `DeleteFile` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeleteFileInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "files")]
    pub files: Vec<String>,
}

/// `RenameFile` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RenameFileInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "fullpath")]
    pub full_path: String,
    #[serde(rename = "newfilename")]
    pub new_file_name: String,
}

/// `Walk` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WalkInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "fullpath")]
    pub full_path: String,
    #[serde(rename = "recursive")]
    pub recursive: bool,
    #[serde(rename = "skipdisallowedfiles")]
    pub skip_disallowed_files: bool,
    #[serde(rename = "skiphiddenfiles")]
    pub skip_hidden_files: bool,
}

/// `UploadFiles` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UploadFilesInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "sources")]
    pub sources: Vec<String>,
    #[serde(rename = "destination")]
    pub destination: String,
    #[serde(rename = "preprocessfiles")]
    pub preprocess_files: bool,
}

/// `DownloadFiles` input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DownloadFilesInput {
    #[serde(rename = "storageid")]
    pub storage_id: u32,
    #[serde(rename = "sources")]
    pub sources: Vec<String>,
    #[serde(rename = "destination")]
    pub destination: String,
    #[serde(rename = "preprocessfiles")]
    pub preprocess_files: bool,
}

// ===========================================================================
// Payload builders
// ===========================================================================
//
// One per operation, each mapping keel domain types (keel-proto / keel-mtp /
// keel-vfs) into the wire structs above and serialising via `to_json`. `abi.rs`
// (exports) and `sampler.rs` (the 500 ms poller) call these.

/// Initialize `data` = `{mtpDeviceInfo, usbDeviceInfo}`.
pub fn initialize_json(dev: &keel_proto::DeviceInfo, usb: &keel_mtp::session::UsbInfo) -> String {
    to_json(&Envelope::ok(device_info_data(dev, usb)))
}

/// FetchDeviceInfo `data`. Byte-identical shape to `initialize_json`; kept as a
/// separate entry point per operation.
pub fn device_info_json(dev: &keel_proto::DeviceInfo, usb: &keel_mtp::session::UsbInfo) -> String {
    to_json(&Envelope::ok(device_info_data(dev, usb)))
}

fn device_info_data(
    dev: &keel_proto::DeviceInfo,
    usb: &keel_mtp::session::UsbInfo,
) -> DeviceInfoData {
    DeviceInfoData {
        mtp_device_info: MtpDeviceInfo {
            standard_version: dev.standard_version,
            mtp_vendor_extension_id: dev.mtp_vendor_extension_id,
            mtp_version: dev.mtp_version,
            mtp_extension: dev.mtp_extension.clone(),
            functional_mode: dev.functional_mode,
            operations_supported: dev.operations_supported.clone(),
            events_supported: dev.events_supported.clone(),
            device_properties_supported: dev.device_properties_supported.clone(),
            capture_formats: dev.capture_formats.clone(),
            playback_formats: dev.playback_formats.clone(),
            manufacturer: dev.manufacturer.clone(),
            model: dev.model.clone(),
            device_version: dev.device_version.clone(),
            serial_number: dev.serial_number.clone(),
        },
        usb_device_info: UsbDeviceInfo {
            id_vendor: usb.vendor_id,
            id_product: usb.product_id,
            device: usb.bcd_device, // `Device` == bcdDevice (golden 0001).
            manufacturer: usb.manufacturer.clone(),
            product: usb.product.clone(),
            serial_number: usb.serial.clone(),
        },
    }
}

/// FetchStorages `data` — raw-PascalCase `[{Sid, Info{…}}]`.
///
/// Unlike `walk_json`/`file_exists_json` (empty → `null`), the storages array is
/// serialised **directly**, so a non-nil empty set is `[]`, not `null` — this
/// matches the wire contract, which distinguishes the two. On the success path
/// an empty slice is unreachable (zero storages errors as `NoStorage`), but the
/// shape must still be right, so the `Vec` is serialised directly rather than
/// through `null_if_empty`.
pub fn storages_json(storages: &[keel_vfs::device::StorageData]) -> String {
    let data: Vec<StorageData> = storages
        .iter()
        .map(|s| StorageData {
            sid: s.sid,
            info: StorageInfo {
                storage_type: s.info.storage_type,
                filesystem_type: s.info.filesystem_type,
                access_capability: s.info.access_capability,
                max_capability: s.info.max_capability,
                free_space_in_bytes: s.info.free_space_in_bytes,
                free_space_in_images: s.info.free_space_in_images,
                storage_description: s.info.storage_description.clone(),
                volume_label: s.info.volume_label.clone(),
            },
        })
        .collect();
    to_json(&Envelope::ok(data))
}

/// MakeDirectory result: `data: true`.
pub fn make_directory_json() -> String {
    to_json(&Envelope::ok(true))
}

/// Ferry `FetchThumbnail` result: the thumbnail bytes as a base64 string, or
/// `data: null` when the object has no thumbnail.
pub fn thumbnail_json(bytes: Option<&[u8]>) -> String {
    match bytes {
        Some(b) => to_json(&Envelope::ok(base64_encode(b))),
        None => to_json(&Envelope::ok(Option::<&str>::None)),
    }
}

/// Standard RFC 4648 base64 (with `=` padding). Small self-contained encoder so
/// the crate keeps its minimal dependency set.
fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// FileExists result. Pairs each input path with its `exists` flag positionally:
/// the result never exceeds the inputs, even on the batch-abort quirk. Empty →
/// `null`.
pub fn file_exists_json(exists: &[bool], input_files: &[String]) -> String {
    let data: Vec<FileExistsData> = exists
        .iter()
        .zip(input_files.iter())
        .map(|(&exists, path)| FileExistsData {
            fullpath: path.clone(),
            exists,
        })
        .collect();
    to_json(&Envelope::ok(null_if_empty(data)))
}

/// DeleteFile result: `data: true`.
pub fn delete_file_json() -> String {
    to_json(&Envelope::ok(true))
}

/// RenameFile result: `data: true`.
pub fn rename_file_json() -> String {
    to_json(&Envelope::ok(true))
}

/// Walk result. Maps each `keel_vfs::FileInfo` to the camelCase wire struct,
/// formatting `dateAdded` (see [`format_date_added`]). An empty walk serialises
/// `data: null` (the nil-slice quirk).
pub fn walk_json(files: &[keel_vfs::FileInfo]) -> String {
    let data: Vec<FileInfo> = files
        .iter()
        .map(|f| FileInfo {
            size: f.size,
            is_dir: f.is_dir,
            mod_time: format_date_added(f.mod_time),
            name: f.name.clone(),
            full_path: f.full_path.clone(),
            parent_path: f.parent_path.clone(),
            extension: f.extension.clone(),
            parent_id: f.parent_id,
            object_id: f.object_id,
        })
        .collect();
    to_json(&Envelope::ok(null_if_empty(data)))
}

/// Upload preprocess `data`. Fired from the sampler.
pub fn upload_preprocess_json(full_path: &str, name: &str, size: i64) -> String {
    to_json(&Envelope::ok(TransferPreprocessData {
        full_path: full_path.to_string(),
        name: name.to_string(),
        size,
    }))
}

/// Download preprocess `data`. Same wire shape as the upload preprocess.
pub fn download_preprocess_json(full_path: &str, name: &str, size: i64) -> String {
    to_json(&Envelope::ok(TransferPreprocessData {
        full_path: full_path.to_string(),
        name: name.to_string(),
        size,
    }))
}

/// Transfer progress result. Maps `keel_vfs::ProgressInfo` to the wire struct.
/// `elapsedTime` is computed HERE as the milliseconds since `p.start_time` (a
/// wall-clock delta the conformance oracle normalises).
pub fn progress_json(p: &keel_vfs::ProgressInfo) -> String {
    // `elapsedTime` is a SIGNED i64 that goes negative when `start_time` is in
    // the future. `start_time` is set at transfer start, so the negative branch
    // is unreachable in practice, but the sign is reproduced rather than clamped
    // to 0. `elapsedTime` is a conformance-normalised field, so this is
    // behaviour-exact either way.
    let now = SystemTime::now();
    let elapsed_time = match now.duration_since(p.start_time) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    };
    to_json(&Envelope::ok(TransferProgressInfo {
        full_path: p.file_info.full_path.clone(),
        name: p.file_info.name.clone(),
        elapsed_time,
        speed: p.speed,
        total_files: p.total_files,
        total_directories: p.total_directories,
        files_sent: p.files_sent,
        files_sent_progress: p.files_sent_progress,
        active_file_size: TransferSizeInfo {
            total: p.active_file_size.total,
            sent: p.active_file_size.sent,
            progress: p.active_file_size.progress,
        },
        bulk_file_size: TransferSizeInfo {
            total: p.bulk_file_size.total,
            sent: p.bulk_file_size.sent,
            progress: p.bulk_file_size.progress,
        },
        status: p.status.as_str().to_string(),
    }))
}

/// Transfer-done result: `data: true`.
pub fn transfer_done_json() -> String {
    to_json(&Envelope::ok(true))
}

/// Dispose result: `data: true`.
pub fn dispose_json() -> String {
    to_json(&Envelope::ok(true))
}

/// Error result: the error envelope, `data: null`.
pub fn error_json(error_type: &'static str, error: String) -> String {
    to_json(&error_envelope(error_type, error))
}

/// Nil-slice semantics: an empty result is `null`, a non-empty one is an array.
/// See the module-level "`null` vs `[]`" note.
fn null_if_empty<T>(v: Vec<T>) -> Option<Vec<T>> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Format a `keel_vfs::FileInfo.mod_time` as the wire contract's timestamp
/// layout, `"2006-01-02T15:04:05.000Z"`.
///
/// The trailing `Z` in that layout is a **literal**, not a zone token: the
/// wall-clock digits are formatted and a bare `Z` is appended — never a real
/// UTC conversion (a "correct" formatter would shift the timestamp). keel-proto
/// decodes PTP datetimes AS UTC, so rendering the `SystemTime` in **UTC** civil
/// time yields the device's original wall-clock digits, then the literal `Z`
/// (golden 0004). `dateAdded` is a conformance-normalised field, so this is
/// behaviour-exact regardless.
///
/// `None` is the zero timestamp, which that layout renders as
/// `"0001-01-01T00:00:00.000Z"`.
fn format_date_added(mod_time: Option<SystemTime>) -> String {
    const GO_ZERO_TIME: &str = "0001-01-01T00:00:00.000Z";
    let Some(t) = mod_time else {
        return GO_ZERO_TIME.to_string();
    };
    let (secs, millis) = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_millis()),
        // A pre-epoch instant keel-proto never produces; fall back to the zero timestamp.
        Err(_) => return GO_ZERO_TIME.to_string(),
    };
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let (hh, mi, se) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mi:02}:{se:02}.{millis:03}Z")
}

/// Howard Hinnant's `civil_from_days` (public-domain
/// <http://howardhinnant.github.io/date_algorithms.html>): days-since-Unix-epoch
/// → `(year, month, day)` proleptic Gregorian. Independent copy of keel-proto's
/// (private) codec helper — the two must agree; the algorithm is fixed.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
// Golden f32 percentages below are written with the exact digit sequence the
// wire contract emits (e.g. the literal `83.330971` mirrors the asserted output
// string `"83.330971"`), so the test reads as a direct golden check. Those extra
// digits are redundant for f32 — the literal rounds to the identical bit
// pattern regardless — so clippy::excessive_precision would suggest truncating
// them; the full digits are kept deliberately for the golden correspondence.
#[allow(clippy::excessive_precision)]
mod tests {
    use super::*;

    // ---- float formatting (6-digit lossy) --------------------------------

    #[test]
    fn lossy_floats_match_golden() {
        // f32 percentages (golden 0011–0013).
        assert_eq!(float32_lossy(33.333336), "33.333336");
        assert_eq!(float32_lossy(66.666672), "66.666672");
        assert_eq!(float32_lossy(83.330971), "83.330971");
        assert_eq!(float32_lossy(83.333809), "83.333809");
        // integral → no ".0" (golden progress:100).
        assert_eq!(float32_lossy(100.0), "100");
        assert_eq!(float32_lossy(0.0), "0");
        // f64 speed (golden 0011/0012).
        assert_eq!(float64_lossy(9.63), "9.63");
        assert_eq!(float64_lossy(0.0), "0");
    }

    #[test]
    fn lossy_floats_computed_like_go() {
        // Percentages computed as (sent/total)*100 in f32.
        let p = |s: f32, t: f32| (s / t) * 100.0_f32;
        assert_eq!(float32_lossy(p(1.0, 3.0)), "33.333336");
        assert_eq!(float32_lossy(p(2.0, 3.0)), "66.666672");
        assert_eq!(float32_lossy(p(1_500_000.0, 1_800_051.0)), "83.330971");
        assert_eq!(float32_lossy(p(1_500_051.0, 1_800_051.0)), "83.333809");
    }

    #[test]
    fn lossy_strips_trailing_and_pads_leading_zeros() {
        assert_eq!(float64_lossy(1.5), "1.5"); // trailing zeros stripped
        assert_eq!(float64_lossy(0.000005), "0.000005"); // leading zeros padded
        assert_eq!(float64_lossy(9.630000), "9.63");
        assert_eq!(float64_lossy(-9.63), "-9.63");
    }

    // ---- envelope + payload casing (exact bytes) -------------------------

    #[test]
    fn error_envelope_data_is_null() {
        // golden 0018/0019 shape.
        let e = error_envelope("ErrorInvalidPath", "file not found: /x".to_string());
        assert_eq!(
            to_json(&e),
            r#"{"errorType":"ErrorInvalidPath","error":"file not found: /x","data":null}"#
        );
        // The error_json builder produces the same bytes.
        assert_eq!(
            error_json("ErrorGeneral", "boom".to_string()),
            r#"{"errorType":"ErrorGeneral","error":"boom","data":null}"#
        );
    }

    #[test]
    fn bool_success_builders() {
        // golden 0006/0007/0014/0016/0017/0020-0022.
        for j in [
            make_directory_json(),
            delete_file_json(),
            rename_file_json(),
            transfer_done_json(),
            dispose_json(),
        ] {
            assert_eq!(j, r#"{"errorType":"","error":"","data":true}"#);
        }
    }

    #[test]
    fn storages_json_matches_golden_0003() {
        let s = keel_vfs::device::StorageData {
            sid: 65537,
            info: keel_proto::StorageInfo {
                storage_type: 3,
                filesystem_type: 2,
                access_capability: 0,
                max_capability: 241419628544,
                free_space_in_bytes: 191037419520,
                free_space_in_images: 1073741824,
                storage_description: "Internal shared storage".to_string(),
                volume_label: String::new(),
            },
        };
        assert_eq!(
            storages_json(std::slice::from_ref(&s)),
            r#"{"errorType":"","error":"","data":[{"Sid":65537,"Info":{"StorageType":3,"FilesystemType":2,"AccessCapability":0,"MaxCapability":241419628544,"FreeSpaceInBytes":191037419520,"FreeSpaceInImages":1073741824,"StorageDescription":"Internal shared storage","VolumeLabel":""}}]}"#
        );
    }

    #[test]
    fn storages_json_empty_is_bracket_not_null() {
        // The storages slice is serialised directly, so a non-nil empty set is
        // `[]`, unlike the Walk / FileExists arms which are `null`. (Unreachable
        // on the happy path — zero storages errors as NoStorage — but the shape
        // must match the wire contract.)
        assert_eq!(
            storages_json(&[]),
            r#"{"errorType":"","error":"","data":[]}"#
        );
    }

    #[test]
    fn file_exists_json_pairs_and_nulls_empty() {
        assert_eq!(
            file_exists_json(
                &[true, false],
                &[
                    "/Download/keel-golden-test".to_string(),
                    "/Download/keel-golden-test/definitely-missing.bin".to_string(),
                ],
            ),
            r#"{"errorType":"","error":"","data":[{"fullpath":"/Download/keel-golden-test","exists":true},{"fullpath":"/Download/keel-golden-test/definitely-missing.bin","exists":false}]}"#
        );
        // Empty → null (nil-slice quirk).
        assert_eq!(
            file_exists_json(&[], &[]),
            r#"{"errorType":"","error":"","data":null}"#
        );
    }

    #[test]
    fn walk_json_maps_domain_and_formats_date() {
        // golden 0004 first element, built from a keel_vfs::FileInfo.
        let f = keel_vfs::FileInfo {
            size: 0,
            is_dir: true,
            mod_time: Some(keel_proto::codec::decode_datetime("20251026T144911").unwrap()),
            name: "Pictures".to_string(),
            full_path: "/Pictures".to_string(),
            parent_path: "/".to_string(),
            extension: String::new(),
            parent_id: 0,
            object_id: 7,
            ..Default::default()
        };
        assert_eq!(
            walk_json(std::slice::from_ref(&f)),
            r#"{"errorType":"","error":"","data":[{"size":0,"isFolder":true,"dateAdded":"2025-10-26T14:49:11.000Z","name":"Pictures","path":"/Pictures","parentPath":"/","extension":"","parentId":0,"objectId":7}]}"#
        );
    }

    #[test]
    fn walk_json_empty_is_null() {
        assert_eq!(walk_json(&[]), r#"{"errorType":"","error":"","data":null}"#);
    }

    #[test]
    fn format_date_added_utc_civil_and_zero() {
        let t = keel_proto::codec::decode_datetime("20251026T144911").unwrap();
        assert_eq!(format_date_added(Some(t)), "2025-10-26T14:49:11.000Z");
        assert_eq!(format_date_added(Some(UNIX_EPOCH)), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_date_added(None), "0001-01-01T00:00:00.000Z");
    }

    #[test]
    fn mtp_device_info_empty_array_is_bracket_not_null() {
        // golden 0001: CaptureFormats:[] (Vec, not Option) even when empty.
        let d = MtpDeviceInfo {
            standard_version: 100,
            mtp_vendor_extension_id: 6,
            mtp_version: 100,
            mtp_extension: "microsoft.com: 1.0; android.com: 1.0;".to_string(),
            functional_mode: 0,
            operations_supported: vec![4097, 4098],
            events_supported: vec![16386],
            device_properties_supported: vec![54273],
            capture_formats: vec![],
            playback_formats: vec![12288],
            manufacturer: "Nothing".to_string(),
            model: "A059".to_string(),
            device_version: "1.0".to_string(),
            serial_number: "2B6DC722".to_string(),
        };
        let s = to_json(&d);
        assert!(s.contains(r#""CaptureFormats":[]"#), "{s}");
        assert!(s.contains(r#""StandardVersion":100"#), "{s}");
    }

    #[test]
    fn initialize_json_matches_full_golden_0001() {
        // End-to-end byte parity against the golden fixture (embedded at compile
        // time), built from keel domain types.
        let golden = include_str!("../../../fixtures/golden/0001.json").trim_end();
        let dev = keel_proto::DeviceInfo {
            standard_version: 100,
            mtp_vendor_extension_id: 6,
            mtp_version: 100,
            mtp_extension: "microsoft.com: 1.0; android.com: 1.0;".to_string(),
            functional_mode: 0,
            operations_supported: vec![
                4097, 4098, 4099, 4100, 4101, 4102, 4103, 4104, 4105, 4106, 4107, 4108, 4109, 4112,
                4116, 4117, 4118, 4119, 4121, 4122, 4123, 38913, 38914, 38915, 38916, 38917, 38337,
                38338, 38339, 38340, 38341,
            ],
            events_supported: vec![16386, 16387, 16388, 16389, 16390, 16391],
            device_properties_supported: vec![54273, 54274, 20483, 20481, 54279, 54278],
            capture_formats: vec![],
            playback_formats: vec![
                12288, 12289, 12292, 12293, 12296, 12297, 12299, 14337, 14338, 14340, 14343, 14344,
                14347, 14349, 47361, 47362, 47363, 47490, 47491, 47492, 47621, 47632, 47633, 47636,
                47746, 47366, 14353, 14354,
            ],
            manufacturer: "Nothing".to_string(),
            model: "A059".to_string(),
            device_version: "1.0".to_string(),
            serial_number: "2B6DC722089C681979BA7660DCE9A7D5".to_string(),
        };
        let usb = keel_mtp::session::UsbInfo {
            vendor_id: 6353,
            product_id: 20193,
            bcd_device: 1537,
            manufacturer: "Nothing".to_string(),
            product: "VOLCANO-QRD_SN:88BB8A47".to_string(),
            serial: "00161358V000616".to_string(),
        };
        assert_eq!(initialize_json(&dev, &usb), golden);
        assert_eq!(device_info_json(&dev, &usb), golden);
    }

    #[test]
    fn preprocess_builders_casing() {
        // golden 0009/0010.
        assert_eq!(
            upload_preprocess_json("/tmp/keel-golden-src/sub/nested.bin", "nested.bin", 300000),
            r#"{"errorType":"","error":"","data":{"fullPath":"/tmp/keel-golden-src/sub/nested.bin","name":"nested.bin","size":300000}}"#
        );
        assert_eq!(
            download_preprocess_json("/tmp/x", "x", 5),
            r#"{"errorType":"","error":"","data":{"fullPath":"/tmp/x","name":"x","size":5}}"#
        );
    }

    #[test]
    fn progress_json_matches_golden_0011_shape() {
        // golden 0011: elapsedTime is volatile; assert the rest verbatim.
        let mut p = keel_vfs::ProgressInfo::default();
        p.file_info.full_path =
            "/Download/keel-golden-test/keel-golden-src/note-🛳️.txt".to_string();
        p.file_info.name = "note-🛳️.txt".to_string();
        p.speed = 9.63;
        p.total_files = 3;
        p.total_directories = 2;
        p.files_sent = 1;
        p.files_sent_progress = 33.333336;
        p.active_file_size = keel_vfs::SizeInfo {
            total: 1500000,
            sent: 1500000,
            progress: 100.0,
        };
        p.bulk_file_size = keel_vfs::SizeInfo {
            total: 1800051,
            sent: 1500000,
            progress: 83.330971,
        };
        p.status = keel_vfs::TransferStatus::InProgress;
        let out = progress_json(&p);
        assert!(out.starts_with(
            r#"{"errorType":"","error":"","data":{"fullPath":"/Download/keel-golden-test/keel-golden-src/note-🛳️.txt","name":"note-🛳️.txt","elapsedTime":"#
        ), "{out}");
        assert!(out.ends_with(
            r#","speed":9.63,"totalFiles":3,"totalDirectories":2,"filesSent":1,"filesSentProgress":33.333336,"activeFileSize":{"total":1500000,"sent":1500000,"progress":100},"bulkFileSize":{"total":1800051,"sent":1500000,"progress":83.330971},"status":"InProgress"}}"#
        ), "{out}");
    }

    #[test]
    fn no_html_escaping_of_special_chars() {
        // No HTML escaping — <, >, & stay raw (serde_json default).
        let d = FileExistsData {
            fullpath: "/a<b>&c".to_string(),
            exists: true,
        };
        assert_eq!(to_json(&d), r#"{"fullpath":"/a<b>&c","exists":true}"#);
    }

    // ---- input decode (case-insensitive, lenient) ------------------------

    #[test]
    fn input_decode_accepts_swift_lowercase_files() {
        // Swift sends "files"; the contract key is "Files".
        let i: FileExistsInput =
            decode_input(r#"{"storageId":65537,"files":["/a","/b"]}"#).unwrap();
        assert_eq!(i.storage_id, 65537);
        assert_eq!(i.files, vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn input_decode_accepts_go_pascalcase_files() {
        let i: FileExistsInput = decode_input(r#"{"StorageId":1,"Files":["/x"]}"#).unwrap();
        assert_eq!(i.storage_id, 1);
        assert_eq!(i.files, vec!["/x".to_string()]);
    }

    #[test]
    fn input_decode_is_fully_case_insensitive() {
        let i: WalkInput = decode_input(
            r#"{"STORAGEID":9,"FULLPATH":"/D","Recursive":true,"SKIPHiddenFiles":true}"#,
        )
        .unwrap();
        assert_eq!(i.storage_id, 9);
        assert_eq!(i.full_path, "/D");
        assert!(i.recursive);
        assert!(i.skip_hidden_files);
        assert!(!i.skip_disallowed_files); // missing → default false
    }

    #[test]
    fn input_decode_missing_and_null_fields_default() {
        // Missing storageId → 0; explicit null destination → "" (null leniency).
        let i: UploadFilesInput =
            decode_input(r#"{"sources":["/a"],"destination":null,"preprocessFiles":true}"#)
                .unwrap();
        assert_eq!(i.storage_id, 0);
        assert_eq!(i.destination, "");
        assert_eq!(i.sources, vec!["/a".to_string()]);
        assert!(i.preprocess_files);
    }

    #[test]
    fn input_decode_ignores_unknown_fields() {
        let i: MakeDirectoryInput =
            decode_input(r#"{"storageId":2,"fullPath":"/n","extra":123}"#).unwrap();
        assert_eq!(i.storage_id, 2);
        assert_eq!(i.full_path, "/n");
    }

    #[test]
    fn unmarshalling_sentinel_is_verbatim() {
        let err = serde_json::from_str::<Value>("{bad").unwrap_err();
        let msg = unmarshalling_error("Walk", &err);
        assert!(msg.starts_with("error occured while Unmarshalling Walk input data "));
        assert!(msg.ends_with(": "));
    }
}

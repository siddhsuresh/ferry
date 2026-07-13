//! PTP/MTP datasets — explicit decode/encode, no reflection.
//!
//! Ported 1:1 from go-mtpfs `mtp/types.go` (struct shapes) and `mtp/encoding.go`
//! (the `decodeWithSelector` / `encodeField` reflection engine that Go runs over
//! those structs). Field names are the Go names, snake_cased; field *order* is
//! load-bearing — it is the wire order the Go reflection walk produced.
//!
//! Direction per dataset (which way the Go ops actually move it — see
//! `mtp/ops.go`):
//!   * DeviceInfo, StorageInfo         — decode only (GetDeviceInfo/GetStorageInfo)
//!   * ObjectInfo                      — decode (GetObjectInfo) AND encode
//!                                       (SendObjectInfo writes it, ops.go:184)
//!   * DevicePropDesc, ObjectPropDesc  — decode only (GetObjectPropDesc etc.)
//!   * Uint32Array/Uint16Array         — decode (GetStorageIDs/GetObjectHandles/
//!                                       GetObjectPropsSupported)
//!   * Uint64Value/StringValue         — decode (GetObjectPropValue) + encode
//!                                       (SetObjectPropValue, e.g. rename)
//!
//! All primitive reads/writes go through `codec` (little-endian ints, PTP
//! strings, PTP datetimes, count-prefixed arrays), so the "trust nothing"
//! bounds checking lives in one place.
//!
//! Fidelity fixes applied here (plan §3.5, "internal bugs are fixed"):
//!   * INT128/UINT128 values decode via a 16-byte little-endian read instead of
//!     Go's `[16]byte` path, which panics (`encoding.go:273` default arm).
//!   * Array-typed property values decode via a count-prefixed loop instead of
//!     Go's `InstantiateType`, which panics on any array selector
//!     (`encoding.go:404` default arm).
//!   * DevicePropDesc/ObjectPropDesc enumeration form decodes instead of
//!     panicking (Go's `decodeArray` over `[]interface{}` hits
//!     `kindSize(Interface)` → panic, `encoding.go:98`).
//!
//! None of these paths are FFI-observable in Ferry (prop descs / non-string,
//! non-u64 prop values never reach the callback JSON), so fixing them cannot
//! change parity — it only stops the crash.

use std::time::SystemTime;

use crate::codec;
use crate::consts::DataType;
use crate::error::ProtoError;

// ---------------------------------------------------------------------------
// MTP data-type codes and form flags.
//
// These mirror go-mtpfs `mtp/const.go:742-754` (data type codes) and
// `const.go:724-726` (form flags). They are public USB-IF PTP/MTP spec values
// (facts, not expression). Kept as private literals here so the selector match
// below does not couple to whatever naming `consts.rs` exposes; the gate agent
// may dedupe against `consts::DataType` associated constants if it prefers.
// ---------------------------------------------------------------------------
const DTC_INT8: u16 = 0x0001;
const DTC_UINT8: u16 = 0x0002;
const DTC_INT16: u16 = 0x0003;
const DTC_UINT16: u16 = 0x0004;
const DTC_INT32: u16 = 0x0005;
const DTC_UINT32: u16 = 0x0006;
const DTC_INT64: u16 = 0x0007;
const DTC_UINT64: u16 = 0x0008;
const DTC_INT128: u16 = 0x0009;
const DTC_UINT128: u16 = 0x000A;
const DTC_ARRAY_MASK: u16 = 0x4000;
const DTC_STR: u16 = 0xFFFF;

const DPFF_RANGE: u8 = 0x01;
const DPFF_ENUMERATION: u8 = 0x02;

/// Bound an element count against the bytes actually remaining before
/// allocating, so a hostile length field cannot drive a huge `with_capacity`.
/// Used for the array-typed property values, whose element widths (u64/u128 and
/// the signed variants) go beyond codec's u16/u32 array helpers.
fn checked_need(count: usize, elem: usize, have: usize) -> Result<(), ProtoError> {
    let need = count.checked_mul(elem).ok_or(ProtoError::Truncated {
        need: usize::MAX,
        have,
    })?;
    if have < need {
        return Err(ProtoError::Truncated { need, have });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PropValue — the Rust form of Go's `DataDependentType` (`interface{}` decoded
// via a `DataTypeSelector`). types.go:39-41.
// ---------------------------------------------------------------------------

/// A single MTP property value, tagged by the data-type selector that produced
/// it. Covers every DTC_* scalar, the STR type, and every array (`ARRAY_MASK`)
/// variant — the last two categories are the ones Go panicked on.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Str(String),
    I8Array(Vec<i8>),
    U8Array(Vec<u8>),
    I16Array(Vec<i16>),
    U16Array(Vec<u16>),
    I32Array(Vec<i32>),
    U32Array(Vec<u32>),
    I64Array(Vec<i64>),
    U64Array(Vec<u64>),
    I128Array(Vec<i128>),
    U128Array(Vec<u128>),
}

impl PropValue {
    /// Encode the raw value bytes (no type/length prefix), matching Go's
    /// `Encode` over a value wrapper struct (`SendData`, ops.go:98) — arrays get
    /// a uint32 count prefix, strings the PTP string encoding, scalars raw LE.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            PropValue::I8(v) => codec::write_i8(*v, out),
            PropValue::U8(v) => codec::write_u8(*v, out),
            PropValue::I16(v) => codec::write_i16(*v, out),
            PropValue::U16(v) => codec::write_u16(*v, out),
            PropValue::I32(v) => codec::write_i32(*v, out),
            PropValue::U32(v) => codec::write_u32(*v, out),
            PropValue::I64(v) => codec::write_i64(*v, out),
            PropValue::U64(v) => codec::write_u64(*v, out),
            PropValue::I128(v) => codec::write_i128(*v, out),
            PropValue::U128(v) => codec::write_u128(*v, out),
            PropValue::Str(s) => codec::encode_string(s, out),
            PropValue::I8Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_i8(x, out);
                }
            }
            PropValue::U8Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_u8(x, out);
                }
            }
            PropValue::I16Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_i16(x, out);
                }
            }
            PropValue::U16Array(a) => codec::write_u16_array(a, out),
            PropValue::I32Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_i32(x, out);
                }
            }
            PropValue::U32Array(a) => codec::write_u32_array(a, out),
            PropValue::I64Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_i64(x, out);
                }
            }
            PropValue::U64Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_u64(x, out);
                }
            }
            PropValue::I128Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_i128(x, out);
                }
            }
            PropValue::U128Array(a) => {
                codec::write_u32(a.len() as u32, out);
                for &x in a {
                    codec::write_u128(x, out);
                }
            }
        }
    }
}

/// Decode one `DataDependentType` value given its selector — the fixed version
/// of Go's `InstantiateType` + `decodeField` (encoding.go:230-409).
pub fn decode_prop_value(buf: &mut &[u8], selector: u16) -> Result<PropValue, ProtoError> {
    // STR (0xFFFF) has the ARRAY_MASK (0x4000) bit set, so it must be tested
    // before the array check or it would be misread as an array.
    if selector == DTC_STR {
        return Ok(PropValue::Str(codec::decode_string(buf)?));
    }
    if selector & DTC_ARRAY_MASK != 0 {
        // Array value: uint32 count then N base-typed elements. FIX (plan §3.5):
        // Go's InstantiateType panics on every array selector (encoding.go:404).
        let base = selector & !DTC_ARRAY_MASK;
        let count = codec::read_u32(buf)? as usize;
        return decode_array_value(buf, base, count);
    }
    Ok(match selector {
        DTC_INT8 => PropValue::I8(codec::read_i8(buf)?),
        // NB: Go's InstantiateType reads UINT8 into an int8 (encoding.go:374) —
        // a value-preserving-for-<128 internal bug; we decode it as unsigned.
        DTC_UINT8 => PropValue::U8(codec::read_u8(buf)?),
        DTC_INT16 => PropValue::I16(codec::read_i16(buf)?),
        DTC_UINT16 => PropValue::U16(codec::read_u16(buf)?),
        DTC_INT32 => PropValue::I32(codec::read_i32(buf)?),
        DTC_UINT32 => PropValue::U32(codec::read_u32(buf)?),
        DTC_INT64 => PropValue::I64(codec::read_i64(buf)?),
        DTC_UINT64 => PropValue::U64(codec::read_u64(buf)?),
        // FIX (plan §3.5): Go decodes INT128/UINT128 into a [16]byte, then
        // panics on `reflect.Array` (encoding.go:273). We read 16 LE bytes.
        DTC_INT128 => PropValue::I128(codec::read_i128(buf)?),
        DTC_UINT128 => PropValue::U128(codec::read_u128(buf)?),
        _ => return Err(ProtoError::Unsupported("unknown MTP data type code")),
    })
}

fn decode_array_value(buf: &mut &[u8], base: u16, count: usize) -> Result<PropValue, ProtoError> {
    let elem = match base {
        DTC_INT8 | DTC_UINT8 => 1,
        DTC_INT16 | DTC_UINT16 => 2,
        DTC_INT32 | DTC_UINT32 => 4,
        DTC_INT64 | DTC_UINT64 => 8,
        DTC_INT128 | DTC_UINT128 => 16,
        _ => return Err(ProtoError::Unsupported("unknown MTP array element type")),
    };
    checked_need(count, elem, buf.len())?;
    Ok(match base {
        DTC_INT8 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_i8(buf)?);
            }
            PropValue::I8Array(v)
        }
        DTC_UINT8 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_u8(buf)?);
            }
            PropValue::U8Array(v)
        }
        DTC_INT16 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_i16(buf)?);
            }
            PropValue::I16Array(v)
        }
        DTC_UINT16 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_u16(buf)?);
            }
            PropValue::U16Array(v)
        }
        DTC_INT32 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_i32(buf)?);
            }
            PropValue::I32Array(v)
        }
        DTC_UINT32 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_u32(buf)?);
            }
            PropValue::U32Array(v)
        }
        DTC_INT64 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_i64(buf)?);
            }
            PropValue::I64Array(v)
        }
        DTC_UINT64 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_u64(buf)?);
            }
            PropValue::U64Array(v)
        }
        DTC_INT128 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_i128(buf)?);
            }
            PropValue::I128Array(v)
        }
        DTC_UINT128 => {
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(codec::read_u128(buf)?);
            }
            PropValue::U128Array(v)
        }
        // Unreachable: `elem` already rejected any other base above. Return an
        // error rather than panic to honor the no-panic-on-device-input rule.
        _ => return Err(ProtoError::Unsupported("unknown MTP array element type")),
    })
}

/// The FORM field of a property description. Go models this as an untyped
/// `interface{}` (`DevicePropDesc.Form`, types.go:74) holding either a
/// `*PropDescRangeForm`, a `*PropDescEnumForm`, or nil.
#[derive(Clone, Debug, PartialEq)]
pub enum PropForm {
    None,
    /// FORM_Range (DPFF_Range, 0x01) — types.go:53-57.
    Range {
        minimum: PropValue,
        maximum: PropValue,
        step: PropValue,
    },
    /// FORM_Enum (DPFF_Enumeration, 0x02) — types.go:59-61.
    Enum { values: Vec<PropValue> },
}

/// Decode the FORM field — the fixed version of `decodePropDescForm`
/// (encoding.go:411-424).
fn decode_form(buf: &mut &[u8], selector: u16, form_flag: u8) -> Result<PropForm, ProtoError> {
    match form_flag {
        DPFF_RANGE => Ok(PropForm::Range {
            minimum: decode_prop_value(buf, selector)?,
            maximum: decode_prop_value(buf, selector)?,
            step: decode_prop_value(buf, selector)?,
        }),
        DPFF_ENUMERATION => {
            // The enum form is a UINT16 "Number of Values" count then that many
            // selector-typed values (PTP/MTP spec). FIX (plan §3.5): Go decodes
            // `Values []interface{}` through `decodeArray`, which panics at
            // `kindSize(reflect.Interface)` (encoding.go:98) before reading
            // anything. NB the width here is uint16 — distinct from the uint32
            // count used by array-typed *values* above.
            let count = codec::read_u16(buf)? as usize;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(decode_prop_value(buf, selector)?);
            }
            Ok(PropForm::Enum { values })
        }
        // DPFF_None (0x00) or any unknown flag → no form data (encoding.go:422).
        _ => Ok(PropForm::None),
    }
}

// ---------------------------------------------------------------------------
// DeviceInfo — types.go:21-36. Decode only (GetDeviceInfo, ops.go:65).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceInfo {
    pub standard_version: u16,
    pub mtp_vendor_extension_id: u32,
    pub mtp_version: u16,
    pub mtp_extension: String,
    pub functional_mode: u16,
    pub operations_supported: Vec<u16>,
    pub events_supported: Vec<u16>,
    pub device_properties_supported: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub playback_formats: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial_number: String,
}

impl DeviceInfo {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(DeviceInfo {
            standard_version: codec::read_u16(buf)?,
            mtp_vendor_extension_id: codec::read_u32(buf)?,
            mtp_version: codec::read_u16(buf)?,
            mtp_extension: codec::decode_string(buf)?,
            functional_mode: codec::read_u16(buf)?,
            operations_supported: codec::read_u16_array(buf)?,
            events_supported: codec::read_u16_array(buf)?,
            device_properties_supported: codec::read_u16_array(buf)?,
            capture_formats: codec::read_u16_array(buf)?,
            playback_formats: codec::read_u16_array(buf)?,
            manufacturer: codec::decode_string(buf)?,
            model: codec::decode_string(buf)?,
            device_version: codec::decode_string(buf)?,
            serial_number: codec::decode_string(buf)?,
        })
    }
}

// ---------------------------------------------------------------------------
// StorageInfo — types.go:107-116. Decode only (GetStorageInfo, ops.go:145).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StorageInfo {
    pub storage_type: u16,
    pub filesystem_type: u16,
    pub access_capability: u16,
    pub max_capability: u64,
    pub free_space_in_bytes: u64,
    pub free_space_in_images: u32,
    pub storage_description: String,
    pub volume_label: String,
}

impl StorageInfo {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(StorageInfo {
            storage_type: codec::read_u16(buf)?,
            filesystem_type: codec::read_u16(buf)?,
            access_capability: codec::read_u16(buf)?,
            max_capability: codec::read_u64(buf)?,
            free_space_in_bytes: codec::read_u64(buf)?,
            free_space_in_images: codec::read_u32(buf)?,
            storage_description: codec::decode_string(buf)?,
            volume_label: codec::decode_string(buf)?,
        })
    }

    /// types.go:118 — FST_GenericHierarchical (const.go:865).
    pub fn is_hierarchical(&self) -> bool {
        self.filesystem_type == 0x0002
    }
    /// types.go:122 — FST_DCF (const.go:866).
    pub fn is_dcf(&self) -> bool {
        self.filesystem_type == 0x0003
    }
    /// types.go:126 — ST_RemovableROM (0x0002) || ST_RemovableRAM (0x0004),
    /// const.go:1918/1920.
    pub fn is_removable(&self) -> bool {
        self.storage_type == 0x0002 || self.storage_type == 0x0004
    }
}

// ---------------------------------------------------------------------------
// ObjectInfo — types.go:131-151. Decode (GetObjectInfo) AND encode
// (SendObjectInfo writes it, ops.go:184).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectInfo {
    pub storage_id: u32,
    pub object_format: u16,
    pub protection_status: u16,
    /// u32 on the wire (types.go:135). `0xFFFFFFFF` is the ">4 GiB" sentinel:
    /// go-mtpx re-fetches the real size via OPC_ObjectSize when it sees this
    /// value (helpers.go:20). Kept as u32 — widening it would break that check
    /// and the encoded wire size.
    pub compressed_size: u32,
    pub thumb_format: u16,
    pub thumb_compressed_size: u32,
    pub thumb_pix_width: u32,
    pub thumb_pix_height: u32,
    pub image_pix_width: u32,
    pub image_pix_height: u32,
    pub image_bit_depth: u32,
    pub parent_object: u32,
    pub association_type: u16,
    pub association_desc: u32,
    pub sequence_number: u32,
    pub filename: String,
    /// `None` == Go's zero `time.Time` (encoded as an empty PTP string;
    /// `encodeTime`/`decodeTime`, encoding.go:188-228). A concrete value —
    /// including the Unix epoch — is `Some`, so the epoch is *not* treated as
    /// absent (it would encode as "19700101T000000").
    pub capture_date: Option<SystemTime>,
    pub modification_date: Option<SystemTime>,
    pub keywords: String,
}

impl ObjectInfo {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(ObjectInfo {
            storage_id: codec::read_u32(buf)?,
            object_format: codec::read_u16(buf)?,
            protection_status: codec::read_u16(buf)?,
            compressed_size: codec::read_u32(buf)?,
            thumb_format: codec::read_u16(buf)?,
            thumb_compressed_size: codec::read_u32(buf)?,
            thumb_pix_width: codec::read_u32(buf)?,
            thumb_pix_height: codec::read_u32(buf)?,
            image_pix_width: codec::read_u32(buf)?,
            image_pix_height: codec::read_u32(buf)?,
            image_bit_depth: codec::read_u32(buf)?,
            parent_object: codec::read_u32(buf)?,
            association_type: codec::read_u16(buf)?,
            association_desc: codec::read_u32(buf)?,
            sequence_number: codec::read_u32(buf)?,
            filename: codec::decode_string(buf)?,
            capture_date: decode_date_field(buf)?,
            modification_date: decode_date_field(buf)?,
            keywords: codec::decode_string(buf)?,
        })
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        codec::write_u32(self.storage_id, out);
        codec::write_u16(self.object_format, out);
        codec::write_u16(self.protection_status, out);
        codec::write_u32(self.compressed_size, out);
        codec::write_u16(self.thumb_format, out);
        codec::write_u32(self.thumb_compressed_size, out);
        codec::write_u32(self.thumb_pix_width, out);
        codec::write_u32(self.thumb_pix_height, out);
        codec::write_u32(self.image_pix_width, out);
        codec::write_u32(self.image_pix_height, out);
        codec::write_u32(self.image_bit_depth, out);
        codec::write_u32(self.parent_object, out);
        codec::write_u16(self.association_type, out);
        codec::write_u32(self.association_desc, out);
        codec::write_u32(self.sequence_number, out);
        codec::encode_string(&self.filename, out);
        encode_date_field(self.capture_date, out);
        encode_date_field(self.modification_date, out);
        codec::encode_string(&self.keywords, out);
    }
}

/// Decode a PTP-datetime field: read the PTP string, then parse it. An empty
/// string maps to `None` (Go's `decodeTime` leaves the zero `time.Time` and
/// only parses when the string is non-empty, encoding.go:210).
fn decode_date_field(buf: &mut &[u8]) -> Result<Option<SystemTime>, ProtoError> {
    let s = codec::decode_string(buf)?;
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(codec::decode_datetime(&s)?))
    }
}

/// Encode a PTP-datetime field: `None` → empty PTP string (Go writes "" for the
/// zero time, encoding.go:191); `Some(t)` → the "YYYYMMDDThhmmss" encoding.
fn encode_date_field(t: Option<SystemTime>, out: &mut Vec<u8>) {
    match t {
        None => codec::encode_string("", out),
        Some(t) => codec::encode_datetime(t, out),
    }
}

// ---------------------------------------------------------------------------
// DevicePropDesc — types.go:63-75 (DevicePropDescFixed + Form). Decode only.
// `pd.Decode` in Go: decode the fixed part, then the form (encoding.go:435).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct DevicePropDesc {
    pub device_property_code: u16,
    pub data_type: DataType,
    pub get_set: u8,
    pub factory_default_value: PropValue,
    pub current_value: PropValue,
    pub form_flag: u8,
    pub form: PropForm,
}

impl DevicePropDesc {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        let device_property_code = codec::read_u16(buf)?;
        // The DataType field seeds the selector for the DataDependentType values
        // that follow (`decodeWithSelector`, encoding.go:334).
        let data_type = DataType(codec::read_u16(buf)?);
        let get_set = codec::read_u8(buf)?;
        let factory_default_value = decode_prop_value(buf, data_type.0)?;
        let current_value = decode_prop_value(buf, data_type.0)?;
        let form_flag = codec::read_u8(buf)?;
        let form = decode_form(buf, data_type.0, form_flag)?;
        Ok(DevicePropDesc {
            device_property_code,
            data_type,
            get_set,
            factory_default_value,
            current_value,
            form_flag,
            form,
        })
    }
}

// ---------------------------------------------------------------------------
// ObjectPropDesc — types.go:77-89. Like DevicePropDesc but with a single
// DataDependentType value and a GroupCode (u32) in place of CurrentValue.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectPropDesc {
    pub object_property_code: u16,
    pub data_type: DataType,
    pub get_set: u8,
    pub factory_default_value: PropValue,
    pub group_code: u32,
    pub form_flag: u8,
    pub form: PropForm,
}

impl ObjectPropDesc {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        let object_property_code = codec::read_u16(buf)?;
        let data_type = DataType(codec::read_u16(buf)?);
        let get_set = codec::read_u8(buf)?;
        let factory_default_value = decode_prop_value(buf, data_type.0)?;
        let group_code = codec::read_u32(buf)?;
        let form_flag = codec::read_u8(buf)?;
        let form = decode_form(buf, data_type.0, form_flag)?;
        Ok(ObjectPropDesc {
            object_property_code,
            data_type,
            get_set,
            factory_default_value,
            group_code,
            form_flag,
            form,
        })
    }
}

// ---------------------------------------------------------------------------
// Thin value wrappers — types.go:91-105. These are the concrete decode/encode
// targets go-mtpx hands to GetObjectPropValue / SetObjectPropValue / the ID
// ops, so the caller (keel-mtp) picks the type from the property it queried
// rather than from a datatype selector.
// ---------------------------------------------------------------------------

/// GetStorageIDs / GetObjectHandles target (Uint32Array, types.go:91).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uint32Array {
    pub values: Vec<u32>,
}

impl Uint32Array {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(Uint32Array {
            values: codec::read_u32_array(buf)?,
        })
    }
}

/// GetObjectPropsSupported target (Uint16Array, types.go:95).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uint16Array {
    pub values: Vec<u16>,
}

impl Uint16Array {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(Uint16Array {
            values: codec::read_u16_array(buf)?,
        })
    }
}

/// GetObjectPropValue(OPC_ObjectSize) target (Uint64Value, types.go:99;
/// go-mtpx helpers.go:21).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uint64Value {
    pub value: u64,
}

impl Uint64Value {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(Uint64Value {
            value: codec::read_u64(buf)?,
        })
    }
    pub fn encode(&self, out: &mut Vec<u8>) {
        codec::write_u64(self.value, out);
    }
}

/// GetObjectPropValue(OPC_ObjectFileName) target and the value SetObjectPropValue
/// writes on rename (StringValue, types.go:103; go-mtpx helpers.go:91,
/// main.go:252).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StringValue {
    pub value: String,
}

impl StringValue {
    pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError> {
        Ok(StringValue {
            value: codec::decode_string(buf)?,
        })
    }
    pub fn encode(&self, out: &mut Vec<u8>) {
        codec::encode_string(&self.value, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// Little-endian bytes of a PTP string: 1-byte count (code units incl. the
    /// trailing NUL) then UTF-16LE + the NUL unit. Matches `encodeStr`
    /// (encoding.go:46). ASCII-only, sufficient for these golden vectors.
    fn ptp_str(s: &str) -> Vec<u8> {
        if s.is_empty() {
            return vec![0x00];
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        let count = units.len() + 1; // + NUL unit
        let mut v = vec![count as u8];
        for u in units {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&[0x00, 0x00]);
        v
    }

    #[test]
    fn device_info_decode_with_extension() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100u16.to_le_bytes()); // standard_version
        bytes.extend_from_slice(&6u32.to_le_bytes()); // mtp_vendor_extension_id
        bytes.extend_from_slice(&110u16.to_le_bytes()); // mtp_version
        bytes.extend_from_slice(&ptp_str("microsoft.com: 1.0;")); // mtp_extension
        bytes.extend_from_slice(&0u16.to_le_bytes()); // functional_mode
        // operations_supported = [0x1001, 0x1002]
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0x1001u16.to_le_bytes());
        bytes.extend_from_slice(&0x1002u16.to_le_bytes());
        // events_supported = []
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // device_properties_supported = [0x5001]
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0x5001u16.to_le_bytes());
        // capture_formats = []
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // playback_formats = [0x3001]
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0x3001u16.to_le_bytes());
        bytes.extend_from_slice(&ptp_str("Nothing")); // manufacturer
        bytes.extend_from_slice(&ptp_str("A059")); // model
        bytes.extend_from_slice(&ptp_str("1.0")); // device_version
        bytes.extend_from_slice(&ptp_str("SER123")); // serial_number

        let mut cur = &bytes[..];
        let di = DeviceInfo::decode(&mut cur).expect("decode");
        assert!(cur.is_empty(), "all bytes consumed");
        assert_eq!(di.standard_version, 100);
        assert_eq!(di.mtp_vendor_extension_id, 6);
        assert_eq!(di.mtp_version, 110);
        assert_eq!(di.mtp_extension, "microsoft.com: 1.0;");
        assert_eq!(di.functional_mode, 0);
        assert_eq!(di.operations_supported, vec![0x1001, 0x1002]);
        assert_eq!(di.events_supported, Vec::<u16>::new());
        assert_eq!(di.device_properties_supported, vec![0x5001]);
        assert_eq!(di.capture_formats, Vec::<u16>::new());
        assert_eq!(di.playback_formats, vec![0x3001]);
        assert_eq!(di.manufacturer, "Nothing");
        assert_eq!(di.model, "A059");
        assert_eq!(di.device_version, "1.0");
        assert_eq!(di.serial_number, "SER123");
    }

    #[test]
    fn storage_info_decode() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0001u16.to_le_bytes()); // storage_type
        bytes.extend_from_slice(&0x0002u16.to_le_bytes()); // filesystem_type (hierarchical)
        bytes.extend_from_slice(&0x0000u16.to_le_bytes()); // access_capability
        bytes.extend_from_slice(&(64u64 << 30).to_le_bytes()); // max_capability
        bytes.extend_from_slice(&(10u64 << 30).to_le_bytes()); // free_space_in_bytes
        bytes.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // free_space_in_images
        bytes.extend_from_slice(&ptp_str("Internal shared storage")); // storage_description
        bytes.extend_from_slice(&ptp_str("")); // volume_label (empty)

        let mut cur = &bytes[..];
        let si = StorageInfo::decode(&mut cur).expect("decode");
        assert!(cur.is_empty());
        assert_eq!(si.storage_type, 1);
        assert_eq!(si.filesystem_type, 2);
        assert_eq!(si.max_capability, 64u64 << 30);
        assert_eq!(si.free_space_in_bytes, 10u64 << 30);
        assert_eq!(si.free_space_in_images, 0xFFFFFFFF);
        assert_eq!(si.storage_description, "Internal shared storage");
        assert_eq!(si.volume_label, "");
        assert!(si.is_hierarchical());
        assert!(!si.is_dcf());
    }

    /// Encode → decode must reproduce the struct exactly, including the empty
    /// (`None`) date fields and the >4 GiB compressed-size sentinel.
    #[test]
    fn object_info_roundtrip_no_dates() {
        let oi = ObjectInfo {
            storage_id: 0x00010001,
            object_format: 0x3000, // Undefined — what go-mtpx uploads use
            protection_status: 0,
            compressed_size: 0xFFFFFFFF, // >4 GiB sentinel
            thumb_format: 0,
            thumb_compressed_size: 0,
            thumb_pix_width: 0,
            thumb_pix_height: 0,
            image_pix_width: 0,
            image_pix_height: 0,
            image_bit_depth: 0,
            parent_object: 0xFFFFFFFF,
            association_type: 0,
            association_desc: 0,
            sequence_number: 0,
            filename: "movie.mkv".to_string(),
            capture_date: None,
            modification_date: None,
            keywords: String::new(),
        };
        let mut out = Vec::new();
        oi.encode(&mut out);
        let mut cur = &out[..];
        let decoded = ObjectInfo::decode(&mut cur).expect("decode");
        assert!(cur.is_empty(), "all bytes consumed");
        assert_eq!(decoded, oi);
    }

    /// A dated ObjectInfo: assert every non-date field survives the round-trip
    /// exactly, and the date fields survive structurally (Some/None). Exact
    /// instant equality is intentionally not asserted — datetime tz handling is
    /// codec.rs's contract, and a real-codec round-trip may normalize tz.
    #[test]
    fn object_info_roundtrip_with_dates() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let oi = ObjectInfo {
            storage_id: 0x00010001,
            object_format: 0x3801, // EXIF/JPEG
            protection_status: 0,
            compressed_size: 2048,
            filename: "IMG_0001.jpg".to_string(),
            capture_date: Some(t),
            modification_date: None,
            keywords: "vacation".to_string(),
            ..Default::default()
        };
        let mut out = Vec::new();
        oi.encode(&mut out);
        let mut cur = &out[..];
        let decoded = ObjectInfo::decode(&mut cur).expect("decode");
        assert!(cur.is_empty());
        assert_eq!(decoded.storage_id, oi.storage_id);
        assert_eq!(decoded.object_format, oi.object_format);
        assert_eq!(decoded.compressed_size, oi.compressed_size);
        assert_eq!(decoded.filename, oi.filename);
        assert_eq!(decoded.keywords, oi.keywords);
        assert!(decoded.capture_date.is_some());
        assert!(decoded.modification_date.is_none());
    }

    #[test]
    fn truncated_input_errors_not_panics() {
        // A DeviceInfo header cut off mid-field must be a Truncated error.
        let bytes = [0x64u8, 0x00, 0x06]; // standard_version + 1 stray byte
        let mut cur = &bytes[..];
        let err = DeviceInfo::decode(&mut cur).unwrap_err();
        assert!(matches!(err, ProtoError::Truncated { .. }));
    }

    #[test]
    fn absurd_array_count_errors_not_ooms() {
        // count = 0xFFFFFFFF u16 elements but no element bytes → Truncated,
        // never a multi-GiB allocation. (Exercised via decode_prop_value's
        // array path, which uses our checked_need guard.)
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        let mut cur = &bytes[..];
        // AUINT16 selector = 0x4004
        let err = decode_prop_value(&mut cur, 0x4004).unwrap_err();
        assert!(matches!(err, ProtoError::Truncated { .. }));
    }

    // --- PropValue: the paths Go panics on (plan §3.5 fixes) --------------

    #[test]
    fn prop_value_uint128_decodes() {
        let mut bytes = Vec::new();
        let v: u128 = 0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10;
        bytes.extend_from_slice(&v.to_le_bytes());
        let mut cur = &bytes[..];
        // DTC_UINT128 = 0x000A
        let pv = decode_prop_value(&mut cur, 0x000A).expect("no panic");
        assert_eq!(pv, PropValue::U128(v));
        assert!(cur.is_empty());
    }

    #[test]
    fn prop_value_int128_decodes() {
        let mut bytes = Vec::new();
        let v: i128 = -12345678901234567890;
        bytes.extend_from_slice(&v.to_le_bytes());
        let mut cur = &bytes[..];
        // DTC_INT128 = 0x0009
        let pv = decode_prop_value(&mut cur, 0x0009).expect("no panic");
        assert_eq!(pv, PropValue::I128(v));
    }

    #[test]
    fn prop_value_uint32_array_decodes() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&3u32.to_le_bytes()); // count
        for x in [10u32, 20, 30] {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        let mut cur = &bytes[..];
        // AUINT32 = DTC_ARRAY_MASK(0x4000) | DTC_UINT32(0x0006) = 0x4006
        let pv = decode_prop_value(&mut cur, 0x4006).expect("no panic");
        assert_eq!(pv, PropValue::U32Array(vec![10, 20, 30]));
        assert!(cur.is_empty());
    }

    #[test]
    fn prop_value_string_roundtrips() {
        let mut out = Vec::new();
        PropValue::Str("rename.txt".into()).encode(&mut out);
        let mut cur = &out[..];
        // DTC_STR = 0xFFFF; must be handled before the array-mask test.
        let pv = decode_prop_value(&mut cur, 0xFFFF).expect("decode");
        assert_eq!(pv, PropValue::Str("rename.txt".into()));
    }

    // --- DevicePropDesc / ObjectPropDesc forms ---------------------------

    #[test]
    fn device_prop_desc_range_form() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x5001u16.to_le_bytes()); // device_property_code (BatteryLevel)
        bytes.extend_from_slice(&0x0002u16.to_le_bytes()); // data_type = DTC_UINT8
        bytes.push(0x00); // get_set = Get
        bytes.push(50); // factory default value (u8)
        bytes.push(80); // current value (u8)
        bytes.push(DPFF_RANGE); // form flag
        bytes.push(0); // range min (u8)
        bytes.push(100); // range max (u8)
        bytes.push(1); // range step (u8)

        let mut cur = &bytes[..];
        let pd = DevicePropDesc::decode(&mut cur).expect("decode");
        assert!(cur.is_empty());
        assert_eq!(pd.device_property_code, 0x5001);
        assert_eq!(pd.data_type.0, 0x0002);
        assert_eq!(pd.get_set, 0);
        assert_eq!(pd.factory_default_value, PropValue::U8(50));
        assert_eq!(pd.current_value, PropValue::U8(80));
        assert_eq!(
            pd.form,
            PropForm::Range {
                minimum: PropValue::U8(0),
                maximum: PropValue::U8(100),
                step: PropValue::U8(1),
            }
        );
    }

    #[test]
    fn object_prop_desc_enum_form() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xDC01u16.to_le_bytes()); // object_property_code (StorageID)
        bytes.extend_from_slice(&0x0004u16.to_le_bytes()); // data_type = DTC_UINT16
        bytes.push(0x01); // get_set = GetSet
        bytes.extend_from_slice(&7u16.to_le_bytes()); // factory default value (u16)
        bytes.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // group_code
        bytes.push(DPFF_ENUMERATION); // form flag
        bytes.extend_from_slice(&3u16.to_le_bytes()); // enum count (UINT16!)
        for x in [1u16, 2, 3] {
            bytes.extend_from_slice(&x.to_le_bytes());
        }

        let mut cur = &bytes[..];
        let pd = ObjectPropDesc::decode(&mut cur).expect("decode");
        assert!(cur.is_empty());
        assert_eq!(pd.object_property_code, 0xDC01);
        assert_eq!(pd.data_type.0, 0x0004);
        assert_eq!(pd.get_set, 1);
        assert_eq!(pd.factory_default_value, PropValue::U16(7));
        assert_eq!(pd.group_code, 1);
        assert_eq!(
            pd.form,
            PropForm::Enum {
                values: vec![PropValue::U16(1), PropValue::U16(2), PropValue::U16(3)],
            }
        );
    }

    // --- value wrappers --------------------------------------------------

    #[test]
    fn uint32_array_decode() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0x00010001u32.to_le_bytes());
        bytes.extend_from_slice(&0x00020001u32.to_le_bytes());
        let mut cur = &bytes[..];
        let a = Uint32Array::decode(&mut cur).expect("decode");
        assert_eq!(a.values, vec![0x00010001, 0x00020001]);
    }

    #[test]
    fn uint64_value_roundtrip() {
        let v = Uint64Value {
            value: 5_000_000_000,
        };
        let mut out = Vec::new();
        v.encode(&mut out);
        let mut cur = &out[..];
        assert_eq!(Uint64Value::decode(&mut cur).expect("decode"), v);
    }

    #[test]
    fn string_value_roundtrip() {
        let v = StringValue {
            value: "photo.png".into(),
        };
        let mut out = Vec::new();
        v.encode(&mut out);
        let mut cur = &out[..];
        assert_eq!(StringValue::decode(&mut cur).expect("decode"), v);
    }
}

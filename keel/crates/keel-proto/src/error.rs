//! Protocol-level error taxonomy for keel-proto.
//!
//! Ported from go-mtpfs `mtp/mtp.go` (the `RCError` type, lines 70-79) and the
//! ad-hoc `fmt.Errorf` sites in `mtp/encoding.go`. Two public types:
//!
//! * [`ProtoError`] — decode/encode failures raised by `container`/`codec`
//!   /`datasets`. Where Go trusted device input (and could `panic`/silently
//!   zero-fill), keel returns one of these instead (plan §3.5 "the Rust code
//!   won't trust device input").
//! * [`RcError`] — a non-OK MTP response code. Its `Display` is byte-for-byte
//!   the Go `RCError.Error()` string, because upper layers *string-match* it:
//!   `ferry/kernel/send_to_js/helpers.go:112,115` does
//!   `strings.Contains(errorMsg, "StoreFull")` / `"StoreNotAvailable"`, and
//!   those tokens come straight from the `RC_names` map value.

use crate::consts::RespCode;
use std::fmt;

/// Decode/encode failures in the wire codec.
///
/// The `Display` text is *not* load-bearing (no upper layer substring-matches
/// `ProtoError`; keel-mtp wraps it in `MtpError::Proto`). The variant set is
/// the contract taxonomy (docs/CONTRACTS.md keel-proto/error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    /// Not enough bytes remained to satisfy a fixed-width read.
    ///
    /// Go equivalents returned `io.EOF` / `fmt.Errorf("underflow")`
    /// (encoding.go decodeStr:31) or `"header specified 0x%x bytes, but have
    /// 0x%x"` (mtp.go decodeRep:349). We collapse them to a typed value that
    /// records what was needed vs. what was left.
    Truncated { need: usize, have: usize },

    /// A PTP string payload could not be interpreted.
    ///
    /// `decode_string` itself never raises this (it is lossy, like Go's
    /// `utf8.EncodeRune` fallback to U+FFFD); it exists for callers that want
    /// to reject malformed strings. Part of the contract taxonomy.
    BadString(String),

    /// A PTP datetime matched none of the tolerance ladder.
    ///
    /// Go returned the underlying `time.Parse` error (encoding.go
    /// decodeTime:223). We carry the offending string for debugging.
    BadDate(String),

    /// A bulk container `Type` field outside the valid range 1..=4.
    ///
    /// Go stored the raw `uint16` and only rejected it later, in
    /// `decodeRep` (mtp.go:340) with a `SyncError`. keel rejects unknown types
    /// at header-decode time; the common "valid but unexpected kind" case
    /// (e.g. Data where Response was wanted) still decodes fine and is the
    /// caller's (keel-mtp) job to turn into a desync error.
    BadContainerType(u16),

    /// A data-type selector / tag keel does not implement.
    ///
    /// Go's `InstantiateType` (encoding.go:405) `panic`ked on an unknown
    /// `DataTypeSelector`; per plan §3.5 keel returns this instead (fixes the
    /// INT128 / array-type decode panic).
    Unsupported(&'static str),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::Truncated { need, have } => {
                write!(f, "truncated: need {need} bytes, have {have}")
            }
            ProtoError::BadString(s) => write!(f, "bad string: {s}"),
            ProtoError::BadDate(s) => write!(f, "bad datetime: {s}"),
            ProtoError::BadContainerType(t) => {
                write!(f, "bad container type {t}")
            }
            ProtoError::Unsupported(what) => write!(f, "unsupported: {what}"),
        }
    }
}

impl std::error::Error for ProtoError {}

/// A non-OK MTP response code returned in a `Container.code` field.
///
/// Faithful port of go-mtpfs `mtp/mtp.go:70-79`:
///
/// ```go
/// type RCError uint16
/// func (e RCError) Error() string {
///     n, ok := RC_names[int(e)]
///     if ok { return n }
///     return fmt.Sprintf("RetCode %x", uint16(e))
/// }
/// ```
///
/// `Display` must reproduce this EXACTLY: the bare `RC_names` value when the
/// code is known (`"OK"`, `"StoreFull"`, `"StoreNotAvailable"`, …), otherwise
/// `"RetCode {code:x}"` (lowercase hex, no `0x`, no leading zeros). The FFI
/// error mapper substring-matches `"StoreFull"` / `"StoreNotAvailable"`
/// (send_to_js/helpers.go:112,115), so this string is load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RcError(pub RespCode);

impl fmt::Display for RcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = (self.0).0;
        match rc_name(code) {
            Some(name) => f.write_str(name),
            // mtp.go:78 — fmt.Sprintf("RetCode %x", uint16(e)).
            None => write!(f, "RetCode {code:x}"),
        }
    }
}

impl std::error::Error for RcError {}

impl RcError {
    /// The raw 16-bit response code.
    pub fn code(&self) -> u16 {
        (self.0).0
    }
}

/// Response-code → spec name, a verbatim port of `RC_names`
/// (go-mtpfs `mtp/const.go:1863`).
///
/// This table is deliberately kept HERE rather than delegating to
/// `consts::code_name`, because `RcError`'s `Display` is byte-for-byte
/// load-bearing (see the type doc) and must stay decoupled from whatever
/// presentation `consts::RespCode`/`code_name` choose for debug logs. Go's
/// `RCError.Error()` likewise had its own inline `RC_names` lookup. Any drift
/// between the two copies is a bug; the values are fixed USB-IF/MTP spec facts.
/// (Go skips 0xA805 — so do we.)
pub(crate) fn rc_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0x2000 => "Undefined",
        0x2001 => "OK",
        0x2002 => "GeneralError",
        0x2003 => "SessionNotOpen",
        0x2004 => "InvalidTransactionID",
        0x2005 => "OperationNotSupported",
        0x2006 => "ParameterNotSupported",
        0x2007 => "IncompleteTransfer",
        0x2008 => "InvalidStorageId",
        0x2009 => "InvalidObjectHandle",
        0x200A => "DevicePropNotSupported",
        0x200B => "InvalidObjectFormatCode",
        0x200C => "StoreFull",
        0x200D => "ObjectWriteProtected",
        0x200E => "StoreReadOnly",
        0x200F => "AccessDenied",
        0x2010 => "NoThumbnailPresent",
        0x2011 => "SelfTestFailed",
        0x2012 => "PartialDeletion",
        0x2013 => "StoreNotAvailable",
        0x2014 => "SpecificationByFormatUnsupported",
        0x2015 => "NoValidObjectInfo",
        0x2016 => "InvalidCodeFormat",
        0x2017 => "UnknownVendorCode",
        0x2018 => "CaptureAlreadyTerminated",
        0x2019 => "DeviceBusy",
        0x201A => "InvalidParentObject",
        0x201B => "InvalidDevicePropFormat",
        0x201C => "InvalidDevicePropValue",
        0x201D => "InvalidParameter",
        0x201E => "SessionAlreadyOpened",
        0x201F => "TransactionCanceled",
        0x2020 => "SpecificationOfDestinationUnsupported",
        0x2021 => "InvalidEnumHandle",
        0x2022 => "NoStreamEnabled",
        0x2023 => "InvalidDataSet",
        0xA121 => "MTP_Invalid_WFC_Syntax",
        0xA122 => "MTP_WFC_Version_Not_Supported",
        0xA171 => "MTP_Media_Session_Limit_Reached",
        0xA172 => "MTP_No_More_Data",
        0xA800 => "MTP_Undefined",
        0xA801 => "MTP_Invalid_ObjectPropCode",
        0xA802 => "MTP_Invalid_ObjectProp_Format",
        0xA803 => "MTP_Invalid_ObjectProp_Value",
        0xA804 => "MTP_Invalid_ObjectReference",
        0xA806 => "MTP_Invalid_Dataset",
        0xA807 => "MTP_Specification_By_Group_Unsupported",
        0xA808 => "MTP_Specification_By_Depth_Unsupported",
        0xA809 => "MTP_Object_Too_Large",
        0xA80A => "MTP_ObjectProp_Not_Supported",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_name_known_codes() {
        assert_eq!(rc_name(0x2001), Some("OK"));
        assert_eq!(rc_name(0x2002), Some("GeneralError"));
        assert_eq!(rc_name(0x2009), Some("InvalidObjectHandle"));
        assert_eq!(rc_name(0x200C), Some("StoreFull"));
        assert_eq!(rc_name(0x2013), Some("StoreNotAvailable"));
        assert_eq!(rc_name(0x201E), Some("SessionAlreadyOpened"));
        assert_eq!(rc_name(0xA80A), Some("MTP_ObjectProp_Not_Supported"));
    }

    #[test]
    fn rc_name_skips_0xa805_like_go() {
        // Go's RC_names has no 0xA805 entry (const.go:1863 map).
        assert_eq!(rc_name(0xA805), None);
    }

    #[test]
    fn rc_name_unknown_is_none() {
        assert_eq!(rc_name(0x5000), None);
        assert_eq!(rc_name(0xFFFF), None);
    }

    #[test]
    fn rcerror_display_matches_go() {
        // Known → bare RC_names value (the exact substrings the FFI mapper
        // looks for).
        assert_eq!(RcError(RespCode(0x200C)).to_string(), "StoreFull");
        assert_eq!(RcError(RespCode(0x2013)).to_string(), "StoreNotAvailable");
        assert_eq!(RcError(RespCode(0x2001)).to_string(), "OK");
        // Unknown → "RetCode %x" (lowercase hex, no 0x, no leading zeros).
        assert_eq!(RcError(RespCode(0x5000)).to_string(), "RetCode 5000");
        assert_eq!(RcError(RespCode(0xABCD)).to_string(), "RetCode abcd");
        assert_eq!(RcError(RespCode(0x0001)).to_string(), "RetCode 1");
    }

    #[test]
    fn ffi_substring_match_survives() {
        // Mirrors send_to_js/helpers.go:112,115.
        let full = RcError(RespCode(0x200C)).to_string();
        let na = RcError(RespCode(0x2013)).to_string();
        assert!(full.contains("StoreFull"));
        assert!(na.contains("StoreNotAvailable"));
    }
}

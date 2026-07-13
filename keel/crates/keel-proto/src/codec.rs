//! PTP dataset codec primitives — little-endian integers, count-prefixed
//! arrays, PTP strings, and PTP datetimes. Pure, no I/O.
//!
//! Ported from go-mtpfs `mtp/encoding.go`. Decoders read from and advance a
//! `&mut &[u8]` cursor (the crate-wide convention, mirroring the contract's
//! `decode_string(buf: &mut &[u8])` / `DeviceInfo::decode(buf: &mut &[u8])`).
//! Encoders append to a `&mut Vec<u8>`.
//!
//! Two deliberate fixes over Go (plan §3.5, "internal bugs are fixed"):
//!   1. **Real UTF-16** strings with surrogate pairs. Go used UCS-2
//!      (`uint16(r)` / one `utf8.EncodeRune` per code unit), which truncates
//!      and corrupts every code point > U+FFFF — emoji, rare CJK. See
//!      [`encode_string`] / [`decode_string`].
//!   2. **No trust of device input.** Go's `decodeArray` (encoding.go:104-135)
//!      ignored short reads and silently zero-filled; keel bounds every read
//!      against the buffer and returns [`ProtoError::Truncated`] instead of
//!      allocating on an attacker-controlled count.

use crate::error::ProtoError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Little-endian integer read/write helpers (u8..u128, i8..i128).
// ---------------------------------------------------------------------------
//
// Go used reflection + `binary.Read/Write` (encoding.go decodeField:255,
// encodeField:299); keel spells them out. Reads never panic: a short buffer
// yields `Truncated`, never an index panic or `unwrap` on device input.

macro_rules! le_int {
    ($read:ident, $write:ident, $t:ty) => {
        #[doc = concat!("Read a little-endian `", stringify!($t), "`, advancing `buf`.")]
        pub fn $read(buf: &mut &[u8]) -> Result<$t, ProtoError> {
            const N: usize = core::mem::size_of::<$t>();
            if buf.len() < N {
                return Err(ProtoError::Truncated { need: N, have: buf.len() });
            }
            let (head, tail) = buf.split_at(N);
            // `head` is exactly N bytes, so this conversion cannot fail; the
            // match keeps the path panic-free regardless.
            let arr: [u8; N] = match head.try_into() {
                Ok(a) => a,
                Err(_) => return Err(ProtoError::Truncated { need: N, have: buf.len() }),
            };
            *buf = tail;
            Ok(<$t>::from_le_bytes(arr))
        }

        #[doc = concat!("Append `v` as a little-endian `", stringify!($t), "`.")]
        pub fn $write(v: $t, out: &mut Vec<u8>) {
            out.extend_from_slice(&v.to_le_bytes());
        }
    };
}

le_int!(read_u8, write_u8, u8);
le_int!(read_u16, write_u16, u16);
le_int!(read_u32, write_u32, u32);
le_int!(read_u64, write_u64, u64);
le_int!(read_u128, write_u128, u128);
le_int!(read_i8, write_i8, i8);
le_int!(read_i16, write_i16, i16);
le_int!(read_i32, write_i32, i32);
le_int!(read_i64, write_i64, i64);
le_int!(read_i128, write_i128, i128);

// ---------------------------------------------------------------------------
// Count-prefixed arrays (u32 count, then that many elements).
// ---------------------------------------------------------------------------
//
// Go `decodeArray`/`encodeArray` (encoding.go:104-179). Only u16 and u32
// element types are reachable in practice: `kindSize` (encoding.go:81-100)
// panics for 8-byte kinds, so go-mtpfs never decodes u64 arrays, and every
// array field in `types.go` is `[]uint16` or `Uint32Array`.

/// Read a `u32`-count-prefixed array of little-endian `u16`s.
///
/// Unlike Go's `decodeArray` (which ignored short reads and left the tail
/// zeroed), a count that would run past the buffer is rejected before any
/// allocation, so a hostile count can neither over-read nor OOM.
pub fn read_u16_array(buf: &mut &[u8]) -> Result<Vec<u16>, ProtoError> {
    let count = read_u32(buf)? as usize;
    let need = match count.checked_mul(2) {
        Some(n) => n,
        None => return Err(ProtoError::Truncated { need: usize::MAX, have: buf.len() }),
    };
    if buf.len() < need {
        return Err(ProtoError::Truncated { need, have: buf.len() });
    }
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        v.push(read_u16(buf)?);
    }
    Ok(v)
}

/// Read a `u32`-count-prefixed array of little-endian `u32`s.
pub fn read_u32_array(buf: &mut &[u8]) -> Result<Vec<u32>, ProtoError> {
    let count = read_u32(buf)? as usize;
    let need = match count.checked_mul(4) {
        Some(n) => n,
        None => return Err(ProtoError::Truncated { need: usize::MAX, have: buf.len() }),
    };
    if buf.len() < need {
        return Err(ProtoError::Truncated { need, have: buf.len() });
    }
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        v.push(read_u32(buf)?);
    }
    Ok(v)
}

/// Write a `u32`-count-prefixed array of `u16`s (Go `encodeArray`).
pub fn write_u16_array(v: &[u16], out: &mut Vec<u8>) {
    write_u32(v.len() as u32, out);
    for &x in v {
        write_u16(x, out);
    }
}

/// Write a `u32`-count-prefixed array of `u32`s (Go `encodeArray`).
pub fn write_u32_array(v: &[u32], out: &mut Vec<u8>) {
    write_u32(v.len() as u32, out);
    for &x in v {
        write_u32(x, out);
    }
}

// ---------------------------------------------------------------------------
// PTP strings.
// ---------------------------------------------------------------------------

/// Max PTP string *content* code units, i.e. excluding the trailing NUL unit.
///
/// Go errored at `codepoints > 254` where `codepoints` counts content units
/// PLUS the NUL terminator (encoding.go:62-64), so the largest string that
/// encodes is 253 content units. See [`encode_string`] for how keel handles
/// overflow under an infallible signature.
const MAX_STRING_CONTENT_UNITS: usize = 253;

/// Encode a PTP string into `out`.
///
/// Wire form: a 1-byte count = number of UTF-16 code units **including** the
/// NUL terminator, followed by that many little-endian `u16`s. The empty string
/// is a single `0x00` byte with **no** terminator unit (encoding.go:47-50).
///
/// FIX (plan §3.5): Go wrote `uint16(r)` for each rune (encoding.go:57), a
/// UCS-2 truncation that mangles any code point above U+FFFF. keel emits real
/// UTF-16, so astral code points become surrogate pairs and round-trip intact.
///
/// DEVIATION: Go returned an error for strings whose count would exceed 254
/// (encoding.go:63). The contract fixes this function's signature as infallible
/// (`docs/CONTRACTS.md`), so instead keel caps the content at
/// [`MAX_STRING_CONTENT_UNITS`] (253) — never splitting a surrogate pair — so
/// the 1-byte count can never wrap or overflow. Ferry filenames never approach
/// this; the cap only guards against a pathological name producing a malformed
/// frame.
pub fn encode_string(s: &str, out: &mut Vec<u8>) {
    // encoding.go:47-50 — empty string is a lone count byte, no terminator.
    if s.is_empty() {
        out.push(0);
        return;
    }

    let count_idx = out.len();
    out.push(0); // placeholder count byte (encoding.go:53 `append(buf[:0], 0)`)

    let mut units: usize = 0;
    for u in s.encode_utf16() {
        if units >= MAX_STRING_CONTENT_UNITS {
            break;
        }
        out.extend_from_slice(&u.to_le_bytes());
        units += 1;
    }

    // If the cap landed us on a lone high surrogate, drop it so we never emit
    // half a pair (only reachable for absurdly long strings).
    if units == MAX_STRING_CONTENT_UNITS {
        let last = u16::from_le_bytes([out[out.len() - 2], out[out.len() - 1]]);
        if (0xD800..=0xDBFF).contains(&last) {
            out.truncate(out.len() - 2);
            units -= 1;
        }
    }

    // NUL terminator unit (encoding.go:61 `append(buf, 0, 0)`).
    out.extend_from_slice(&[0, 0]);
    units += 1; // count includes the terminator (encoding.go:62)

    out[count_idx] = units as u8; // fits: units <= 254 by construction
}

/// Decode a PTP string from `buf`, advancing past it.
///
/// FIX (plan §3.5): Go's `decodeStr` (encoding.go:15-44) ran each `u16` through
/// `utf8.EncodeRune` independently — UCS-2 — turning both halves of a surrogate
/// pair into U+FFFD. keel decodes real UTF-16; unpaired surrogates (genuinely
/// malformed device data) become U+FFFD via [`char::decode_utf16`], so decoding
/// is lossy-but-total and never panics.
///
/// A count byte declaring more units than remain in `buf` is a truncated frame
/// and returns [`ProtoError::Truncated`] (Go returned `"underflow"`,
/// encoding.go:31-33).
pub fn decode_string(buf: &mut &[u8]) -> Result<String, ProtoError> {
    // Count byte. On an empty buffer Go's `r.Read` failed and returned the
    // error (encoding.go:17-19) — so do we (a genuinely empty *string* is a
    // 0x00 byte we would read here, not an empty buffer).
    let sz = read_u8(buf)? as usize;
    if sz == 0 {
        // encoding.go:22-24 — count 0 is the empty string.
        return Ok(String::new());
    }

    let need = 2 * sz;
    if buf.len() < need {
        return Err(ProtoError::Truncated { need, have: buf.len() });
    }
    let (data, rest) = buf.split_at(need);
    *buf = rest;

    let mut units: Vec<u16> = (0..sz)
        .map(|i| u16::from_le_bytes([data[2 * i], data[2 * i + 1]]))
        .collect();

    // Strip the trailing NUL terminator unit. Go stripped a trailing 0x00
    // *byte* after UTF-8 encoding (encoding.go:39-41); only U+0000 encodes to a
    // 0x00 byte, so dropping a trailing 0x0000 code unit is equivalent.
    if units.last() == Some(&0) {
        units.pop();
    }

    Ok(char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

// ---------------------------------------------------------------------------
// PTP datetimes.
// ---------------------------------------------------------------------------

/// Encode a PTP datetime as the string `"YYYYMMDDThhmmss"` into `out`.
///
/// Go `encodeTime` (encoding.go:188-202) formatted with `timeFormat =
/// "20060102T150405"` and encoded a zero/unset `time.Time` as the empty string.
///
/// DEVIATION: Go formatted in the `time.Time`'s own location (local, for file
/// mtimes). `SystemTime` is a timezone-naive instant and keel-proto carries no
/// timezone facilities (`log`-only dep), so keel formats the instant as **UTC**
/// civil time. `decode_datetime` parses UTC symmetrically, so wire round-trips
/// are exact; callers that need device-local wall-clock must offset the instant
/// before calling. (Recorded as an open issue for keel-vfs.)
/// An instant before the Unix epoch encodes as the empty string, mirroring
/// Go's zero-`time.Time` → empty-string behaviour.
pub fn encode_datetime(t: SystemTime, out: &mut Vec<u8>) {
    let s = match t.duration_since(UNIX_EPOCH) {
        Ok(dur) => format_ptp_datetime(dur.as_secs()),
        Err(_) => String::new(),
    };
    encode_string(&s, out);
}

/// Parse a PTP datetime string into a [`SystemTime`], applying the exact
/// vendor-tolerance ladder from Go `decodeTime` (encoding.go:204-228):
///
/// 1. Samsung: strip trailing `'.'` (`strings.TrimRight(s, ".")`).
/// 2. Jolla Sailfish: strip trailing `'Z'` (`strings.TrimRight(s, "Z")`).
/// 3. Parse `"YYYYMMDDThhmmss"` as UTC (Go `time.Parse` with no location).
/// 4. Fallback — Nokia Lumia: parse `"YYYYMMDDThhmmss±hhmm"` (numeric TZ).
///
/// The empty string decodes to [`UNIX_EPOCH`] (Go left the zero `time.Time`).
/// Anything that survives step 1/2 non-empty but parses in neither format is a
/// [`ProtoError::BadDate`] — matching Go, which errored when a non-empty input
/// trimmed away or failed both parses. Never panics.
pub fn decode_datetime(s: &str) -> Result<SystemTime, ProtoError> {
    // Go checked the ORIGINAL string: an empty input stays zero-time with no
    // parse (encoding.go:210). A non-empty input that trims to "" still enters
    // the parse block and errors — we reproduce that below.
    if s.is_empty() {
        return Ok(UNIX_EPOCH);
    }

    // TrimRight cutsets, in Go's order: dots first, then Z's.
    let trimmed = s.trim_end_matches('.').trim_end_matches('Z');

    if let Some(secs) = parse_ptp_datetime(trimmed) {
        return secs_to_systemtime(secs);
    }
    if let Some(secs) = parse_ptp_datetime_tz(trimmed) {
        return secs_to_systemtime(secs);
    }
    Err(ProtoError::BadDate(s.to_string()))
}

/// Format Unix seconds as `"YYYYMMDDThhmmss"` (UTC civil time).
fn format_ptp_datetime(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as i64;
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, se) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{se:02}")
}

/// Parse `"YYYYMMDDThhmmss"` (exactly 15 bytes) → Unix seconds (UTC).
fn parse_ptp_datetime(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 15 || b[8] != b'T' {
        return None;
    }
    let y = parse_digits(&b[0..4])?;
    let mo = parse_digits(&b[4..6])?;
    let d = parse_digits(&b[6..8])?;
    let h = parse_digits(&b[9..11])?;
    let mi = parse_digits(&b[11..13])?;
    let se = parse_digits(&b[13..15])?;
    civil_to_unix_secs(y, mo, d, h, mi, se)
}

/// Parse `"YYYYMMDDThhmmss±hhmm"` (exactly 20 bytes) → Unix seconds (UTC),
/// applying the numeric timezone offset (Go `timeFormatNumTZ`).
fn parse_ptp_datetime_tz(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 {
        return None;
    }
    let base = parse_ptp_datetime(&s[..15])?; // wall clock, treated as UTC
    let sign: i64 = match b[15] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let oh = parse_digits(&b[16..18])?;
    let om = parse_digits(&b[18..20])?;
    if oh > 23 || om > 59 {
        return None;
    }
    let offset = sign * (oh * 3600 + om * 60);
    // Wall time in zone (+off) corresponds to UTC = wall - offset.
    Some(base - offset)
}

fn parse_digits(b: &[u8]) -> Option<i64> {
    if b.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    Some(v)
}

/// Validate coarse field ranges (like Go's `time.Parse` rejecting garbage) and
/// convert a civil UTC datetime to Unix seconds. Returns `None` on an
/// out-of-range field so the caller falls through to the TZ form / `BadDate`.
fn civil_to_unix_secs(y: i64, mo: i64, d: i64, h: i64, mi: i64, se: i64) -> Option<i64> {
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || se > 60
    {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

fn secs_to_systemtime(secs: i64) -> Result<SystemTime, ProtoError> {
    let st = if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs((-secs) as u64))
    };
    st.ok_or_else(|| ProtoError::BadDate(format!("datetime out of range: {secs}s")))
}

// Howard Hinnant's `days_from_civil` / `civil_from_days` (public-domain
// algorithms), with 1970-01-01 as day 0. Rust's `/` truncates toward zero,
// which is exactly what these algorithms assume.

/// Days since 1970-01-01 for a proleptic-Gregorian y/m/d.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Proleptic-Gregorian y/m/d for a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- integer helpers ---------------------------------------------------

    #[test]
    fn int_roundtrips() {
        let mut out = Vec::new();
        write_u8(0xAB, &mut out);
        write_u16(0xBEEF, &mut out);
        write_u32(0xDEAD_BEEF, &mut out);
        write_u64(0x0123_4567_89AB_CDEF, &mut out);
        write_i32(-2, &mut out);
        write_u128(0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00, &mut out);

        let mut b = &out[..];
        assert_eq!(read_u8(&mut b).unwrap(), 0xAB);
        assert_eq!(read_u16(&mut b).unwrap(), 0xBEEF);
        assert_eq!(read_u32(&mut b).unwrap(), 0xDEAD_BEEF);
        assert_eq!(read_u64(&mut b).unwrap(), 0x0123_4567_89AB_CDEF);
        assert_eq!(read_i32(&mut b).unwrap(), -2);
        assert_eq!(
            read_u128(&mut b).unwrap(),
            0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00
        );
        assert!(b.is_empty());
    }

    #[test]
    fn int_read_truncated_errors_not_panics() {
        let buf = [0x01u8, 0x02, 0x03];
        let mut b = &buf[..];
        assert_eq!(
            read_u32(&mut b),
            Err(ProtoError::Truncated { need: 4, have: 3 })
        );
        // cursor not advanced on failure
        assert_eq!(b.len(), 3);
        let mut empty: &[u8] = &[];
        assert_eq!(
            read_u8(&mut empty),
            Err(ProtoError::Truncated { need: 1, have: 0 })
        );
    }

    // --- arrays ------------------------------------------------------------

    #[test]
    fn u16_array_roundtrip() {
        let v = vec![0x1001u16, 0x1002, 0x9803];
        let mut out = Vec::new();
        write_u16_array(&v, &mut out);
        // count(u32) + 3*u16
        assert_eq!(&out[0..4], &3u32.to_le_bytes());
        let mut b = &out[..];
        assert_eq!(read_u16_array(&mut b).unwrap(), v);
        assert!(b.is_empty());
    }

    #[test]
    fn u32_array_roundtrip() {
        let v = vec![0xFFFF_FFFFu32, 0, 42];
        let mut out = Vec::new();
        write_u32_array(&v, &mut out);
        let mut b = &out[..];
        assert_eq!(read_u32_array(&mut b).unwrap(), v);
    }

    #[test]
    fn array_count_beyond_buffer_errors_not_allocates() {
        // count says 1_000_000 u16 but only a few bytes follow: must error
        // (Go would have zero-filled), never allocate on the hostile count.
        let mut out = Vec::new();
        write_u32(1_000_000, &mut out);
        out.extend_from_slice(&[0u8; 4]);
        let mut b = &out[..];
        match read_u16_array(&mut b) {
            Err(ProtoError::Truncated { need, have }) => {
                assert_eq!(need, 2_000_000);
                assert_eq!(have, 4);
            }
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    // --- strings: ported encoding_test.go cases ----------------------------

    #[test]
    fn encode_str_empty() {
        // TestEncodeStrEmpty: "" -> single 0x00 byte.
        let mut out = Vec::new();
        encode_string("", &mut out);
        assert_eq!(out, vec![0x00]);
    }

    #[test]
    fn encode_str_a_umlaut() {
        // TestEncodeStr: "ä" -> \x02\xe4\x00\x00\x00
        let mut out = Vec::new();
        encode_string("ä", &mut out);
        assert_eq!(out, vec![0x02, 0xE4, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn decode_str_o_umlaut_roundtrip() {
        // TestDecodeStr: encode "ö", decode back to "ö".
        let mut out = Vec::new();
        encode_string("ö", &mut out);
        let mut b = &out[..];
        assert_eq!(decode_string(&mut b).unwrap(), "ö");
        assert!(b.is_empty());
    }

    #[test]
    fn decode_empty_string_byte() {
        let buf = [0x00u8];
        let mut b = &buf[..];
        assert_eq!(decode_string(&mut b).unwrap(), "");
        assert!(b.is_empty());
    }

    // --- strings: emoji fix + edges ----------------------------------------

    #[test]
    fn emoji_surrogate_pair_roundtrip() {
        // The whole point of the UCS-2 fix. "😀" = U+1F600 = surrogate pair
        // [D83D, DE00]; count = 2 content + 1 terminator = 3.
        let mut out = Vec::new();
        encode_string("😀", &mut out);
        assert_eq!(
            out,
            vec![0x03, 0x3D, 0xD8, 0x00, 0xDE, 0x00, 0x00],
            "emoji must encode as a real UTF-16 surrogate pair, not UCS-2"
        );
        let mut b = &out[..];
        assert_eq!(decode_string(&mut b).unwrap(), "😀");
    }

    #[test]
    fn mixed_bmp_and_astral_roundtrip() {
        for s in ["a😀b", "日本語", "café", "🇺🇸flag", "𝕳ello"] {
            let mut out = Vec::new();
            encode_string(s, &mut out);
            let mut b = &out[..];
            assert_eq!(decode_string(&mut b).unwrap(), s);
        }
    }

    #[test]
    fn max_length_edge() {
        // 253 content units is the largest that encodes without capping:
        // count byte = 254.
        let s: String = "a".repeat(253);
        let mut out = Vec::new();
        encode_string(&s, &mut out);
        assert_eq!(out[0], 254);
        let mut b = &out[..];
        assert_eq!(decode_string(&mut b).unwrap(), s);

        // Overlong input is capped at 253 content units (documented deviation
        // from Go's error), count byte stays 254, no panic, no wrap.
        let long: String = "a".repeat(300);
        let mut out2 = Vec::new();
        encode_string(&long, &mut out2);
        assert_eq!(out2[0], 254);
        let mut b2 = &out2[..];
        assert_eq!(decode_string(&mut b2).unwrap(), "a".repeat(253));
    }

    #[test]
    fn cap_never_splits_surrogate_pair() {
        // 252 'a' then an emoji: the emoji's high surrogate would land at unit
        // 253 with its low half cut — the guard drops the lone high surrogate.
        let mut s = "a".repeat(252);
        s.push('😀');
        let mut out = Vec::new();
        encode_string(&s, &mut out);
        let mut b = &out[..];
        let decoded = decode_string(&mut b).unwrap();
        // No U+FFFD (would signal a split pair); the emoji is dropped whole.
        assert!(!decoded.contains('\u{FFFD}'));
        assert_eq!(decoded, "a".repeat(252));
    }

    #[test]
    fn decode_str_truncated_errors_not_panics() {
        // count says 5 units (10 bytes) but only 4 bytes follow.
        let buf = [0x05u8, 0x41, 0x00, 0x42, 0x00];
        let mut b = &buf[..];
        match decode_string(&mut b) {
            Err(ProtoError::Truncated { need: 10, have: 4 }) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn decode_str_empty_buffer_errors() {
        // No count byte available at all.
        let mut b: &[u8] = &[];
        assert!(matches!(
            decode_string(&mut b),
            Err(ProtoError::Truncated { need: 1, have: 0 })
        ));
    }

    #[test]
    fn decode_str_lone_surrogate_becomes_replacement() {
        // Malformed device data: a lone high surrogate (D800) + terminator.
        // count = 2 units.
        let buf = [0x02u8, 0x00, 0xD8, 0x00, 0x00];
        let mut b = &buf[..];
        assert_eq!(decode_string(&mut b).unwrap(), "\u{FFFD}");
    }

    // --- datetime ----------------------------------------------------------

    #[test]
    fn datetime_samsung_dot_roundtrip() {
        // Ported TestDecodeTime: "20120101T010022." (Samsung trailing dot)
        // decodes, re-encodes, and reads back as the canonical form.
        let t = decode_datetime("20120101T010022.").unwrap();
        let mut out = Vec::new();
        encode_datetime(t, &mut out);
        let mut b = &out[..];
        assert_eq!(decode_string(&mut b).unwrap(), "20120101T010022");
    }

    #[test]
    fn datetime_jolla_z_suffix() {
        let t = decode_datetime("20120101T010022Z").unwrap();
        let mut out = Vec::new();
        encode_datetime(t, &mut out);
        let mut b = &out[..];
        assert_eq!(decode_string(&mut b).unwrap(), "20120101T010022");
    }

    #[test]
    fn datetime_nokia_numeric_tz() {
        // +0100 wall clock => UTC is one hour earlier.
        let with_tz = decode_datetime("20120101T120000+0100").unwrap();
        let utc = decode_datetime("20120101T110000").unwrap();
        assert_eq!(with_tz, utc);

        // -0230 => UTC is 2h30 later.
        let neg = decode_datetime("20120101T120000-0230").unwrap();
        let utc2 = decode_datetime("20120101T143000").unwrap();
        assert_eq!(neg, utc2);
    }

    #[test]
    fn datetime_empty_is_epoch() {
        assert_eq!(decode_datetime("").unwrap(), UNIX_EPOCH);
    }

    #[test]
    fn datetime_trims_to_empty_is_error_like_go() {
        // Non-empty input that trims away enters the parse block and fails
        // both formats, so Go errored — and so do we.
        assert!(matches!(decode_datetime("Z"), Err(ProtoError::BadDate(_))));
        assert!(matches!(decode_datetime("."), Err(ProtoError::BadDate(_))));
    }

    #[test]
    fn datetime_garbage_errors_not_panics() {
        for s in ["garbage", "2012", "20120101X010022", "20120145T010022", "99999999T999999"] {
            assert!(
                matches!(decode_datetime(s), Err(ProtoError::BadDate(_))),
                "expected BadDate for {s:?}"
            );
        }
    }

    #[test]
    fn datetime_encode_zero_time() {
        // Pre-epoch instant encodes as the empty string (single 0x00).
        let mut out = Vec::new();
        encode_datetime(UNIX_EPOCH - Duration::from_secs(1), &mut out);
        assert_eq!(out, vec![0x00]);
    }

    #[test]
    fn civil_algorithms_are_inverse() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for &(y, m, d) in &[
            (1970, 1, 1),
            (2000, 2, 29),
            (2012, 1, 1),
            (2024, 12, 31),
            (1999, 6, 15),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "roundtrip {y}-{m}-{d}");
        }
    }

    #[test]
    fn datetime_full_wire_roundtrip() {
        // Build the on-wire date string, decode to instant, re-encode: stable.
        let mut wire = Vec::new();
        encode_string("20240229T235959", &mut wire);
        let mut b = &wire[..];
        let s = decode_string(&mut b).unwrap();
        let t = decode_datetime(&s).unwrap();
        let mut out = Vec::new();
        encode_datetime(t, &mut out);
        let mut b2 = &out[..];
        assert_eq!(decode_string(&mut b2).unwrap(), "20240229T235959");
    }
}

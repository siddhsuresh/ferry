//! The 12-byte little-endian MTP bulk container header and its framing.
//!
//! Wire layout (little-endian), 12 bytes:
//! ```text
//!   offset 0  u32  Length          (header + payload; 0xFFFFFFFF saturated)
//!   offset 4  u16  Type            (1=Command 2=Data 3=Response 4=Event)
//!   offset 6  u16  Code            (op / response / event code)
//!   offset 8  u32  TransactionID
//! ```

use crate::error::ProtoError;

/// Length of the bulk container header in bytes (two u16s + two u32s).
pub const HDR_LEN: u32 = 12;

/// Maximum number of `u32` parameters in a command/response container.
pub const MAX_PARAMS: usize = 5;

/// The kind of a bulk container, from the header `Type` field.
///
/// `Default` is `Command` because `Container::default()` is used to build
/// outgoing requests in keel-mtp.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ContainerKind {
    #[default]
    Command = 1,
    Data = 2,
    Response = 3,
    Event = 4,
}

impl TryFrom<u16> for ContainerKind {
    type Error = ProtoError;

    fn try_from(v: u16) -> Result<Self, ProtoError> {
        match v {
            1 => Ok(ContainerKind::Command),
            2 => Ok(ContainerKind::Data),
            3 => Ok(ContainerKind::Response),
            4 => Ok(ContainerKind::Event),
            // Reject only truly-out-of-range types here; a valid but unexpected
            // kind (e.g. Data where Response was wanted) decodes fine and is
            // left for the caller to catch.
            other => Err(ProtoError::BadContainerType(other)),
        }
    }
}

/// An MTP request/response/event container.
///
/// Has no `SessionID`: keel-mtp stamps that at the transaction layer, and the
/// on-wire bulk header carries no session field. `params` holds up to
/// [`MAX_PARAMS`] `u32` values; header encode/decode never touch them (the
/// caller appends/parses parameter bytes around the header).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Container {
    pub kind: ContainerKind,
    pub code: u16,
    pub transaction_id: u32,
    pub params: Vec<u32>,
}

impl Container {
    /// Encode the 12-byte header for a container carrying `payload_len` bytes
    /// of payload *after* the header (i.e. `4 * params.len()` for a command, or
    /// the data-phase byte count for a data container).
    ///
    /// The `Length` field is `HDR_LEN + payload_len`, **saturated to
    /// `0xFFFFFFFF`** when the total exceeds 32 bits — the >4 GiB sentinel: any
    /// header-inclusive total that overflows a `u32` is clamped to `0xFFFFFFFF`
    /// rather than truncated.
    pub fn encode_header(&self, payload_len: u64) -> [u8; 12] {
        let total = HDR_LEN as u64 + payload_len;
        let length = if total > 0xFFFF_FFFF {
            0xFFFF_FFFF
        } else {
            total as u32
        };

        let mut b = [0u8; 12];
        b[0..4].copy_from_slice(&length.to_le_bytes());
        b[4..6].copy_from_slice(&(self.kind as u16).to_le_bytes());
        b[6..8].copy_from_slice(&self.code.to_le_bytes());
        b[8..12].copy_from_slice(&self.transaction_id.to_le_bytes());
        b
    }

    /// Decode the 12-byte header from the front of `buf`.
    ///
    /// Returns the [`Container`] (with `params` left empty — parameters live in
    /// the bytes *after* the header and are parsed by the caller from the raw
    /// `Length`) and the raw `Length` field. The header is read first, then the
    /// caller uses `Length - HDR_LEN` to know how many parameter/data bytes
    /// follow.
    pub fn decode_header(buf: &[u8]) -> Result<(Container, u32), ProtoError> {
        if buf.len() < HDR_LEN as usize {
            return Err(ProtoError::Truncated {
                need: HDR_LEN as usize,
                have: buf.len(),
            });
        }

        let length = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let ty = u16::from_le_bytes([buf[4], buf[5]]);
        let code = u16::from_le_bytes([buf[6], buf[7]]);
        let tid = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);

        let kind = ContainerKind::try_from(ty)?;
        Ok((
            Container {
                kind,
                code,
                transaction_id: tid,
                params: Vec::new(),
            },
            length,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_len_and_max_params() {
        assert_eq!(HDR_LEN, 12);
        assert_eq!(MAX_PARAMS, 5);
    }

    #[test]
    fn encode_command_no_params() {
        // GetDeviceInfo-shaped command, no params, tid 7, code 0x1001.
        let c = Container {
            kind: ContainerKind::Command,
            code: 0x1001,
            transaction_id: 7,
            params: vec![],
        };
        let h = c.encode_header(0);
        assert_eq!(
            h,
            [
                0x0C, 0x00, 0x00, 0x00, // length = 12
                0x01, 0x00, // type = Command
                0x01, 0x10, // code = 0x1001
                0x07, 0x00, 0x00, 0x00, // tid = 7
            ]
        );
    }

    #[test]
    fn encode_command_with_params_payload_len() {
        // Two u32 params => payload_len = 8 => length = 20.
        let c = Container {
            kind: ContainerKind::Command,
            code: 0x1005,
            transaction_id: 1,
            params: vec![0xFFFF_FFFF, 0],
        };
        let h = c.encode_header((c.params.len() * 4) as u64);
        assert_eq!(&h[0..4], &20u32.to_le_bytes());
        assert_eq!(&h[4..6], &1u16.to_le_bytes()); // Command
    }

    #[test]
    fn encode_saturates_over_4gib() {
        let c = Container {
            kind: ContainerKind::Data,
            code: 0x1009,
            transaction_id: 42,
            params: vec![],
        };
        // 5 GiB payload => total > 0xFFFFFFFF => 0xFFFFFFFF sentinel.
        let h = c.encode_header(5 * 1024 * 1024 * 1024);
        assert_eq!(&h[0..4], &0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(&h[4..6], &2u16.to_le_bytes()); // Data
    }

    #[test]
    fn encode_saturation_boundary_is_exact() {
        let c = Container::default();
        // total == 0xFFFFFFFF exactly => NOT saturated (strict `>`, not `>=`).
        let just_under = 0xFFFF_FFFFu64 - HDR_LEN as u64;
        assert_eq!(
            u32::from_le_bytes(c.encode_header(just_under)[0..4].try_into().unwrap()),
            0xFFFF_FFFF
        );
        // one more byte tips it over — still 0xFFFFFFFF (saturated).
        assert_eq!(
            u32::from_le_bytes(c.encode_header(just_under + 1)[0..4].try_into().unwrap()),
            0xFFFF_FFFF
        );
        // one fewer => exact value 0xFFFFFFFE.
        assert_eq!(
            u32::from_le_bytes(c.encode_header(just_under - 1)[0..4].try_into().unwrap()),
            0xFFFF_FFFE
        );
    }

    #[test]
    fn decode_roundtrips_header() {
        let c = Container {
            kind: ContainerKind::Response,
            code: 0x2001,
            transaction_id: 99,
            params: vec![],
        };
        let h = c.encode_header(0);
        let (got, length) = Container::decode_header(&h).unwrap();
        assert_eq!(length, 12);
        assert_eq!(got.kind, ContainerKind::Response);
        assert_eq!(got.code, 0x2001);
        assert_eq!(got.transaction_id, 99);
        assert!(got.params.is_empty());
    }

    #[test]
    fn decode_all_valid_kinds() {
        for (ty, want) in [
            (1u16, ContainerKind::Command),
            (2, ContainerKind::Data),
            (3, ContainerKind::Response),
            (4, ContainerKind::Event),
        ] {
            let mut b = [0u8; 12];
            b[4..6].copy_from_slice(&ty.to_le_bytes());
            let (c, _) = Container::decode_header(&b).unwrap();
            assert_eq!(c.kind, want);
        }
    }

    #[test]
    fn decode_rejects_bad_container_type() {
        let mut b = [0u8; 12];
        b[4..6].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(
            Container::decode_header(&b),
            Err(ProtoError::BadContainerType(7))
        );
        // type 0 (UNDEFINED) is also rejected.
        let z = [0u8; 12];
        assert_eq!(
            Container::decode_header(&z),
            Err(ProtoError::BadContainerType(0))
        );
    }

    #[test]
    fn decode_truncated_never_panics() {
        for n in 0..12usize {
            let buf = vec![0u8; n];
            match Container::decode_header(&buf) {
                Err(ProtoError::Truncated { need: 12, have }) => assert_eq!(have, n),
                other => panic!("expected Truncated, got {other:?}"),
            }
        }
    }
}

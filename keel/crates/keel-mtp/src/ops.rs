//! The MTP operations Ferry needs, as `impl MtpSession` blocks.
//!
//! Faithful port of go-mtpfs `mtp/ops.go`. Each method builds a command
//! [`Container`], drives it through the transaction engine
//! ([`MtpSession::run_transaction`], ported in `transaction.rs`), and decodes the
//! result — mirroring the Go `Device` receiver methods one-for-one.
//!
//! Scope (plan §2 keel-mtp `ops` table + docs/CONTRACTS.md `MtpSession` surface):
//! only the operations Ferry / go-mtpx actually invoke. The other ops.go
//! functions — `GetObjectPropDesc`, `GetObjectPropsSupported`, `GetDevicePropDesc`,
//! `GetDevicePropValue`, `SetDevicePropValue`, `ResetDevicePropValue`,
//! `GetNumObjects`, and the buggy `GetPartialObject` (ops.go:215-220) — are
//! intentionally NOT ported here: go-mtpx never calls them, they are outside the
//! fixed contract surface, and `GetPartialObject` is superseded by
//! `android.rs::get_partial_object_64` (the plan §3.5 opcode/param fix).
//! `OpenSession`/`CloseSession` (ops.go:19-50) live in `session.rs` because they
//! own the `tid`/`sid` session state and the Configure recovery ladder.
//!
//! DATA DIRECTION CONVENTION (matches Go `runTransaction(req, rep, dest, src, …)`,
//! mtp.go:401): `data_in` is the device→host data phase (`dest`, GetData /
//! GetObject); `data_out` is the host→device data phase (`src`, SendData /
//! SendObject). `write_size` is the byte count of the `data_out` phase.

use std::io::{Cursor, Read, Write};

use keel_proto::{
    decode_prop_value, Container, DeviceInfo, ObjectInfo, ObjectPropCode, OpCode, PropValue,
    ProtoError, StorageInfo, Uint32Array,
};

use crate::error::MtpError;
use crate::session::MtpSession;
use crate::transport::Transport;

// ---------------------------------------------------------------------------
// Small helpers shared by the operations below.
// ---------------------------------------------------------------------------

/// A `Write` adapter that counts the bytes forwarded to an inner writer.
///
/// [`MtpSession::get_object`] wraps the caller's sink in this so it can return the
/// total object byte count (the contract's `Result<u64>`) without depending on
/// what `run_transaction` returns. Go's `GetObject` (ops.go:207-213) returned only
/// an error; the byte total is a keel addition (docs/CONTRACTS.md). Counting at
/// the sink is more accurate than the Go progress callback, which omitted the
/// first data packet's bytes (mtp.go:454 writes `rest` before `bulkRead` starts
/// its own counter) — but that quirk lives in the progress values, not the total.
struct CountingWriter<'a> {
    inner: &'a mut dyn Write,
    count: u64,
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Standard MTP object-property code → PTP data-type selector.
///
/// `GetObjectPropValue`'s response is a bare, untyped value: the wire carries no
/// data-type tag because the type is fixed by the property's `ObjectPropDesc`
/// (MTP spec §5). Go decoded it by having the caller pass a concrete target
/// (`*Uint64Value` for `OPC_ObjectSize`, `*StringValue` for `OPC_ObjectFileName`
/// — go-mtpx helpers.go:21,92) whose Go type drove reflection. keel's contract
/// returns a tagged [`PropValue`], so the type must be recovered from the property
/// code here rather than from a round-trip to `GetObjectPropDesc` (which Go also
/// avoided). The two load-bearing entries for Ferry are `OBJECT_SIZE` (u64) and
/// `OBJECT_FILE_NAME` (str); the rest are standard USB-IF spec facts for
/// completeness. Selectors are the DTC_* codes (const.go:742-754), duplicated as
/// literals here to stay decoupled from `consts`/`datasets` internals.
fn object_prop_datatype(prop: ObjectPropCode) -> Option<u16> {
    Some(match prop.0 {
        0xDC01 => 0x0006, // StorageID: UINT32
        0xDC02 => 0x0004, // ObjectFormat: UINT16
        0xDC03 => 0x0004, // ProtectionStatus: UINT16
        0xDC04 => 0x0008, // ObjectSize: UINT64            <-- Ferry (helpers.go:21)
        0xDC05 => 0x0004, // AssociationType: UINT16
        0xDC06 => 0x0006, // AssociationDesc: UINT32
        0xDC07 => 0xFFFF, // ObjectFileName: STR           <-- Ferry (helpers.go:92, main.go:252)
        0xDC08 => 0xFFFF, // DateCreated: STR
        0xDC09 => 0xFFFF, // DateModified: STR
        0xDC0A => 0xFFFF, // Keywords: STR
        0xDC0B => 0x0006, // ParentObject: UINT32
        0xDC0D => 0x0004, // Hidden: UINT16
        0xDC0E => 0x0004, // SystemObject: UINT16
        0xDC41 => 0x000A, // PersistentUniqueObjectIdentifier: UINT128
        0xDC44 => 0xFFFF, // Name: STR
        0xDC4A => 0xFFFF, // DateAdded: STR
        _ => return None,
    })
}

impl<T: Transport> MtpSession<T> {
    // -----------------------------------------------------------------------
    // Internal data-phase helpers (ops.go GetData:52-63 / SendData:98-107).
    // -----------------------------------------------------------------------

    /// Run a command whose only useful output is its device→host data phase,
    /// collecting that phase into a byte buffer for the caller to decode.
    /// Faithful port of ops.go `GetData` (52-63); the returned response
    /// container is discarded (Go kept only the decoded dataset).
    fn get_data(&mut self, req: Container) -> Result<Vec<u8>, MtpError> {
        let mut buf: Vec<u8> = Vec::new();
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, Some(&mut buf as &mut dyn Write), None, 0, &mut noprog)?;
        Ok(buf)
    }

    /// Encode `payload` and run a command with a host→device data phase.
    /// Faithful port of ops.go `SendData` (98-107). Returns the response
    /// container so callers that need response parameters (SendObjectInfo) can
    /// read them.
    fn send_data(&mut self, req: Container, payload: Vec<u8>) -> Result<Container, MtpError> {
        let size = payload.len() as u64;
        let mut cur = Cursor::new(payload);
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, None, Some(&mut cur as &mut dyn Read), size, &mut noprog)
    }

    // -----------------------------------------------------------------------
    // The operations (docs/CONTRACTS.md MtpSession surface).
    // -----------------------------------------------------------------------

    /// `GetDeviceInfo` (ops.go:65-69).
    pub fn device_info(&mut self) -> Result<DeviceInfo, MtpError> {
        let req = Container {
            code: OpCode::GET_DEVICE_INFO.0,
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        DeviceInfo::decode(&mut cur).map_err(MtpError::Proto)
    }

    /// `GetStorageIDs` (ops.go:71-75) — decodes a `Uint32Array` (types.go:91).
    pub fn storage_ids(&mut self) -> Result<Vec<u32>, MtpError> {
        let req = Container {
            code: OpCode::GET_STORAGE_IDS.0,
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        Ok(Uint32Array::decode(&mut cur).map_err(MtpError::Proto)?.values)
    }

    /// `GetStorageInfo` (ops.go:145-150).
    pub fn storage_info(&mut self, sid: u32) -> Result<StorageInfo, MtpError> {
        let req = Container {
            code: OpCode::GET_STORAGE_INFO.0,
            params: vec![sid],
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        StorageInfo::decode(&mut cur).map_err(MtpError::Proto)
    }

    /// `GetObjectHandles` (ops.go:152-157).
    ///
    /// `format` is the ObjectFormatCode filter: **0 means "all formats"**
    /// (ops.go:155 passes it straight through; go-mtpx's `GOH_ALL_ASSOCS`
    /// name for the 0 constant is a misnomer — 0 is "all", not "associations",
    /// per plan §2 keel-vfs `path` — so keel does not "correct" it). `parent`
    /// 0xFFFFFFFF is the storage root.
    pub fn object_handles(
        &mut self,
        sid: u32,
        format: u16,
        parent: u32,
    ) -> Result<Vec<u32>, MtpError> {
        let req = Container {
            code: OpCode::GET_OBJECT_HANDLES.0,
            params: vec![sid, format as u32, parent],
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        Ok(Uint32Array::decode(&mut cur).map_err(MtpError::Proto)?.values)
    }

    /// `GetObjectInfo` (ops.go:159-164).
    pub fn object_info(&mut self, handle: u32) -> Result<ObjectInfo, MtpError> {
        let req = Container {
            code: OpCode::GET_OBJECT_INFO.0,
            params: vec![handle],
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        ObjectInfo::decode(&mut cur).map_err(MtpError::Proto)
    }

    /// `GetObject` (ops.go:207-213) — streams the object's data phase to `sink`,
    /// reporting cumulative bytes through `progress` (the object `handle` is
    /// threaded into the second callback slot for the vfs/ffi layer's context).
    ///
    /// Returns the total bytes written to `sink`. The >4 GiB path is transparent
    /// here: the transaction engine keeps reading until a short packet regardless
    /// of the 0xFFFFFFFF length sentinel, and re-fetching the real size via
    /// `OPC_ObjectSize` when `ObjectInfo.CompressedSize` is saturated is the vfs
    /// layer's job (go-mtpx helpers.go:20) — ops just streams.
    pub fn get_object(
        &mut self,
        handle: u32,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, u32) -> Result<(), MtpError>,
    ) -> Result<u64, MtpError> {
        let req = Container {
            code: OpCode::GET_OBJECT.0,
            params: vec![handle],
            ..Default::default()
        };
        let mut counter = CountingWriter {
            inner: sink,
            count: 0,
        };
        let mut prog = |sent: u64| progress(sent, handle);
        self.run_transaction(
            req,
            Some(&mut counter as &mut dyn Write),
            None,
            0,
            &mut prog,
        )?;
        Ok(counter.count)
    }

    /// `GetThumb` (0x100A): the device-generated thumbnail image (usually a
    /// small JPEG) for an object. Best-effort — returns `Ok(None)` when the
    /// device has no thumbnail for this handle (folders, documents, or the
    /// `NO_THUMBNAIL_PRESENT` / invalid-format response codes). Only genuine
    /// transport/sync failures propagate as `Err`.
    pub fn get_thumb(&mut self, handle: u32) -> Result<Option<Vec<u8>>, MtpError> {
        let req = Container {
            code: OpCode::GET_THUMB.0,
            params: vec![handle],
            ..Default::default()
        };
        let mut buf: Vec<u8> = Vec::new();
        let mut prog = |_sent: u64| Ok(());
        match self.run_transaction(
            req,
            Some(&mut buf as &mut dyn Write),
            None,
            0,
            &mut prog,
        ) {
            Ok(_resp) => Ok(Some(buf)),
            // Any protocol-level rejection means "no thumbnail available" — the
            // caller falls back to a type glyph. Transport/sync errors (device
            // gone, desync) still propagate.
            Err(MtpError::Rc(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `DeleteObject` (ops.go:176-182).
    ///
    /// Params are `{handle, 0x0}`: the trailing `0` is the ObjectFormatCode slot
    /// (0 = all formats). Go passes it explicitly (ops.go:179) — some devices
    /// reject the single-parameter form, so the `0` is load-bearing, not padding.
    pub fn delete_object(&mut self, handle: u32) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::DELETE_OBJECT.0,
            params: vec![handle, 0x0],
            ..Default::default()
        };
        let mut noprog = |_: u64| Ok::<(), MtpError>(());
        self.run_transaction(req, None, None, 0, &mut noprog)?;
        Ok(())
    }

    /// `SendObjectInfo` (ops.go:184-199) — sends an [`ObjectInfo`] dataset and
    /// returns the **new object handle** the device assigned.
    ///
    /// Go returned `(storageID, parent, handle)` = `rep.Param[0..3]`, but every
    /// go-mtpx caller discards the first two (`_, _, objId, err :=`,
    /// helpers.go:220,255), so the contract returns only the handle
    /// (`rep.Param[2]`). The "need 3 response parameters" guard (ops.go:193-195)
    /// is preserved; a shorter response is a malformed dataset, surfaced as a
    /// non-poisoning `Proto` error (Go used a plain `fmt.Errorf`, which — unlike a
    /// `SyncError` — did not close the connection, mtp.go:388-393).
    pub fn send_object_info(
        &mut self,
        sid: u32,
        parent: u32,
        info: &ObjectInfo,
    ) -> Result<u32, MtpError> {
        let req = Container {
            code: OpCode::SEND_OBJECT_INFO.0,
            params: vec![sid, parent],
            ..Default::default()
        };
        let mut payload = Vec::new();
        info.encode(&mut payload);
        let rep = self.send_data(req, payload)?;
        if rep.params.len() < 3 {
            // ops.go:193-195. No `MtpError` "Other(String)" variant exists (the
            // contract enum is fixed), so a too-short response is reported as a
            // truncated dataset — the closest non-poisoning classification.
            return Err(MtpError::Proto(ProtoError::Truncated {
                need: 3,
                have: rep.params.len(),
            }));
        }
        Ok(rep.params[2])
    }

    /// `SendObject` (ops.go:201-205) — streams `size` bytes from `source` as the
    /// object's data phase. Has no parameters: the object was described by the
    /// immediately-preceding `SendObjectInfo`. No handle exists at this point, so
    /// the progress callback's handle slot is `0` (go-mtpx's own progress closure
    /// ignores it, helpers.go:262).
    pub fn send_object(
        &mut self,
        source: &mut dyn Read,
        size: u64,
        progress: &mut dyn FnMut(u64, u32) -> Result<(), MtpError>,
    ) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::SEND_OBJECT.0,
            ..Default::default()
        };
        let mut prog = |sent: u64| progress(sent, 0);
        self.run_transaction(req, None, Some(source), size, &mut prog)?;
        Ok(())
    }

    /// `GetObjectPropValue` (ops.go:84-89) for the standard properties Ferry
    /// reads. The response is decoded per the property's fixed MTP data type (see
    /// [`object_prop_datatype`]); an unmapped property yields a `Proto` error
    /// rather than a wrong-typed decode.
    pub fn object_prop_value(
        &mut self,
        handle: u32,
        prop: ObjectPropCode,
    ) -> Result<PropValue, MtpError> {
        let selector = object_prop_datatype(prop).ok_or(MtpError::Proto(
            ProtoError::Unsupported("GetObjectPropValue: unknown object-property data type"),
        ))?;
        let req = Container {
            code: OpCode::MTP_GET_OBJECT_PROP_VALUE.0,
            params: vec![handle, prop.0 as u32],
            ..Default::default()
        };
        let buf = self.get_data(req)?;
        let mut cur = &buf[..];
        decode_prop_value(&mut cur, selector).map_err(MtpError::Proto)
    }

    /// `SetObjectPropValue` (ops.go:91-96) — used for rename (property
    /// `OPC_ObjectFileName`, go-mtpx main.go:252). The value is encoded from the
    /// caller-supplied [`PropValue`], which already carries its type, so no
    /// property→type lookup is needed on this path. The response is ignored, as
    /// in Go.
    pub fn set_object_prop_value(
        &mut self,
        handle: u32,
        prop: ObjectPropCode,
        v: &PropValue,
    ) -> Result<(), MtpError> {
        let req = Container {
            code: OpCode::MTP_SET_OBJECT_PROP_VALUE.0,
            params: vec![handle, prop.0 as u32],
            ..Default::default()
        };
        let mut payload = Vec::new();
        v.encode(&mut payload);
        self.send_data(req, payload)?;
        Ok(())
    }
}

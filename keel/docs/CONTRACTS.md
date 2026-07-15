# Keel cross-crate contracts

Fixed API boundaries so crates can be built in parallel. Internals are the
implementing agent's call; **these signatures are not**. The behavioral source
of truth is the plan (`/Users/siddharthsuresh/Developer/plans/FERRY_RUST_KERNEL_PLAN.md`)
plus the Go reference sources listed at the bottom. Fidelity policy: plan §3
(FFI-observable bugs preserved; internal bugs fixed — the plan enumerates both).

## keel-proto (no I/O, no deps beyond `log`)

```rust
// consts.rs — generated tables. u16 newtypes with Display giving spec names.
pub struct OpCode(pub u16);      // OC_*  e.g. OpCode::GET_DEVICE_INFO = OpCode(0x1001)
pub struct RespCode(pub u16);    // RC_*  RespCode::OK = RespCode(0x2001)
pub struct EventCode(pub u16);   // EC_*
pub struct ObjectFormat(pub u16);// OFC_* ObjectFormat::ASSOCIATION = 0x3001
pub struct DevicePropCode(pub u16); // DPC_*
pub struct ObjectPropCode(pub u16); // OPC_* OBJECT_FILE_NAME = 0xDC07, OBJECT_SIZE = 0xDC04
pub struct DataType(pub u16);    // DTC_* incl. arrays/strings/INT128
pub fn code_name(kind: CodeKind, code: u16) -> Option<&'static str>; // debug maps

// container.rs
pub const HDR_LEN: u32 = 12;
pub const MAX_PARAMS: usize = 5;
#[derive(Clone, Debug, Default)]
pub struct Container { pub kind: ContainerKind, pub code: u16,
                       pub transaction_id: u32, pub params: Vec<u32> }
pub enum ContainerKind { Command = 1, Data = 2, Response = 3, Event = 4 }
impl Container {
  pub fn encode_header(&self, payload_len: u64) -> [u8; 12]; // 0xFFFFFFFF saturation
  pub fn decode_header(buf: &[u8]) -> Result<(Container, u32 /*len field*/), ProtoError>;
}

// codec.rs — explicit per-type encode/decode on byte slices (little-endian)
pub fn encode_string(s: &str, out: &mut Vec<u8>);      // UTF-16 w/ surrogates, 254 units, trailing NUL unit; empty = single 0x00 count
pub fn decode_string(buf: &mut &[u8]) -> Result<String, ProtoError>;
pub fn encode_datetime(t: SystemTime, out: &mut Vec<u8>); // "YYYYMMDDThhmmss"
pub fn decode_datetime(s: &str) -> Result<SystemTime, ProtoError>; // Samsung '.', Jolla 'Z', Lumia ±hhmm quirks
// + u8/u16/u32/u64/u128 read/write helpers and array codecs

// datasets.rs
pub struct DeviceInfo { /* all fields from go-mtpfs types.go, same names snake_cased */ }
pub struct StorageInfo { ... }   // max_capability, free_space_in_bytes, storage_description, ...
pub struct ObjectInfo { ... }    // compressed_size: u32 (0xFFFFFFFF = >4GiB sentinel)
pub struct DevicePropDesc { ... } pub struct ObjectPropDesc { ... }
pub enum PropValue { U8(u8) ... U128(u128), I128(i128), Str(String), U16Array(Vec<u16>), U32Array(Vec<u32>), ... }
impl DeviceInfo { pub fn decode(buf: &mut &[u8]) -> Result<Self, ProtoError>; }
// every dataset: decode; ObjectInfo also encode (SendObjectInfo needs it)

// error.rs
pub enum ProtoError { Truncated { need: usize, have: usize }, BadString(...),
                      BadDate(String), Unsupported(&'static str), ... }
pub struct RcError(pub RespCode);  // non-OK response; Display = "RC_<name> (0x2001)" style, match Go's RCError text: "RC error 0x%x <name>"
```

## keel-mtp

```rust
// transport.rs — PRE-WRITTEN, do not modify. keel-usb implements it; tests fake it.
pub trait Transport: Send {
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, TransportError>;
    fn bulk_in(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;
    fn reset(&mut self) -> Result<(), TransportError>;
    fn max_packet_size(&self) -> usize;
    fn close(&mut self);
}
pub enum TransportError { Timeout, DeviceGone, Stall, Io(String) }
// TransportError::DeviceGone Display MUST contain "LIBUSB_ERROR_NO_DEVICE"
// (the FFI error mapper string-matches it — plan keel-ffi/errors).

// session.rs
pub struct MtpSession<T: Transport> { ... }
impl<T: Transport> MtpSession<T> {
  pub fn configure(transport: T) -> Result<Self, MtpError>;  // recovery ladder per plan
  pub fn close(self);
  pub fn device_info(&mut self) -> Result<DeviceInfo, MtpError>;
  pub fn storage_ids(&mut self) -> Result<Vec<u32>, MtpError>;
  pub fn storage_info(&mut self, sid: u32) -> Result<StorageInfo, MtpError>;
  pub fn object_handles(&mut self, sid: u32, format: u16, parent: u32) -> Result<Vec<u32>, MtpError>;
  pub fn object_info(&mut self, handle: u32) -> Result<ObjectInfo, MtpError>;
  pub fn get_object(&mut self, handle: u32, sink: &mut dyn Write,
                    progress: &mut dyn FnMut(u64 /*sent*/, u32 /*handle*/) -> Result<(), MtpError>) -> Result<u64, MtpError>;
  pub fn delete_object(&mut self, handle: u32) -> Result<(), MtpError>; // params {handle, 0}
  pub fn send_object_info(&mut self, sid: u32, parent: u32, info: &ObjectInfo) -> Result<u32 /*new handle*/, MtpError>;
  pub fn send_object(&mut self, source: &mut dyn Read, size: u64,
                     progress: &mut dyn FnMut(u64, u32) -> Result<(), MtpError>) -> Result<(), MtpError>;
  pub fn object_prop_value(&mut self, handle: u32, prop: ObjectPropCode) -> Result<PropValue, MtpError>;
  pub fn set_object_prop_value(&mut self, handle: u32, prop: ObjectPropCode, v: &PropValue) -> Result<(), MtpError>;
  pub fn usb_info(&self) -> &UsbInfo;   // vid/pid/bcd/strings, set at configure
  pub fn reset_device(&mut self) -> Result<(), MtpError>;
}

// error.rs
pub enum MtpError { Rc(RcError), Sync(String) /* poisons session */,
                    Transport(TransportError), Proto(ProtoError), Closed }
```

`android.rs`: 0x95C1–0x95C5 wrappers (post-parity features use them; port now,
including forced SeparateHeader around SendPartialObject and the corrected
`get_partial_object_64`).

## keel-usb

Built on **nusb 0.2.4** (see docs/nusb-api.md — the `.wait()`/`.await` rule is
load-bearing). Empirically proven on the real phone (VID 18d1, USB2 High).

```rust
pub struct UsbTransport { ... }              // implements keel_mtp::Transport
pub struct UsbInfo { pub vendor_id: u16, pub product_id: u16, pub bcd_device: u16,
                     pub manufacturer: String, pub product: String, pub serial: String }
pub fn discover() -> Result<Discovered, DiscoverError>;
pub struct Discovered { pub transport: UsbTransport, pub usb_info: UsbInfo }
pub enum DiscoverError {
    NoDevice,                                   // Display == "no MTP devices found"
    MultipleDevices(usize),                     // Display contains "more than 1 device"
    ExclusiveAccess { owner: Option<String> },  // ptpcamerad/Image Capture/etc holds it (IORegistry-named)
    Access(String),
    Other(String),
}
// keel-vfs maps ExclusiveAccess → VfsError::ExclusiveAccess{owner} → keel-ffi
// errorType ErrorDeviceSetup (parity with Go's libusb claim-fail) with the
// owner named in the message, lighting up Swift's existing isDeviceSetupFailure UX.
```

Quirks (ZLP, XHCI-response-tolerance, SeparateHeader detection, 2s→15s
timeouts, 16 KiB chunking) live *behind* `Transport` where they're per-pipe
(ZLP), and in keel-mtp's transaction layer where they're protocol-level
(SeparateHeader). Split per plan crate tables.

## keel-vfs

Public API mirrors go-mtpx main.go exports one-for-one (snake_case):
`initialize(opts) -> Device`, `fetch_device_info`, `fetch_storages`,
`make_directory`, `file_exists`, `delete_file`, `rename_file`, `walk`,
`upload_files`, `download_files`, `dispose` — with `ProgressInfo`,
`FileInfo`, callback signatures and the full mtpx error enum
(`VfsError::{MtpDetectFailed, Configure, DeviceInfo, StorageInfo, NoStorage,
ListDirectory, FileNotFound, FilePermission, LocalFile, InvalidPath,
FileTransfer, FileObject, SendObject, Mtp(MtpError)}`) — every variant's
Display text matches the Go error strings (the FFI mapper depends on them).
`FileInfo` field set = mtpx structs.go exactly.

## keel-ffi

Exports (all `#[no_mangle] pub unsafe extern "C"`):
`Initialize, FetchDeviceInfo, FetchStorages, MakeDirectory, FileExists,
DeleteFile, RenameFile, Walk, UploadFiles, DownloadFiles, Dispose,
CancelTransfer` — signatures identical to `kernel/the legacy kernel` (`*mut c_char`
JSON in, callback fn-pointer-as-value in `on_cb_result_t*` position).
JSON contract, error mapper order, 500 ms sampler, state rules: plan tables
(keel-ffi section) are normative; `ferry/kernel/send_to_js/*.go` is the
reference implementation to port line-by-line.
Golden fixtures (Go kernel vs real Nothing A059): `keel/fixtures/golden/*.json`
— the exact payload bytes keel-ffi must reproduce (modulo volatile fields:
elapsedTime, speed, dates).

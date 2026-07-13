# nusb 0.2 API Reference for keel-usb (macOS USB-MTP transport)

**Source of truth:** reverse-engineered from `vdavid/mtp-rs`
(`crates/mtp-rs/src/transport/nusb.rs`, 1270 lines) — the only file that
actually drives nusb. This **replaces** any draft written against nusb 0.1;
that API is gone. Everything below is nusb **0.2.x**.

**Pinned version:** mtp-rs declares `nusb = "0.2"`, `Cargo.lock` resolves to
**`nusb 0.2.3`**. Companions: `futures = "0.3"`, `futures-timer = "3.0"`
(the async timer for timeout races), `async-trait = "0.1"`. nusb is used with
**default features** — NOT the `tokio`/`smol` features. That choice makes the
`.wait()` rule below load-bearing.

---

## 0. Golden rule: `MaybeFuture` (`.wait()`) vs genuine async (`.await`)

nusb 0.2 is async-first, but several ops are blocking syscalls (ioctls)
dressed as futures, returning `impl nusb::MaybeFuture<Output = T>`:

- `nusb::list_devices()`
- `DeviceInfo::open()`
- `Device::claim_interface()`
- `Device::set_configuration()`
- `Endpoint::clear_halt()`

These must be **`.wait()`-ed, never `.await`-ed**. `MaybeFuture` implements
`IntoFuture`, so `.await` *compiles* — but without nusb's `tokio`/`smol`
feature it **panics at runtime** ("Awaiting blocking syscall without an async
runtime"). It sails through CI and blows up on a device (nusb #212). Funnel
every such call through one helper:

```rust
use nusb::MaybeFuture;
/// Run a nusb blocking-syscall MaybeFuture to completion. Never `.await` these.
fn blocking<F: MaybeFuture>(op: F) -> F::Output { op.wait() }
```

**Genuinely async (use `.await`):** `Interface::control_in`,
`Interface::control_out`, `Endpoint::next_complete`.

**Blocking but NOT a MaybeFuture (call directly):**
`Endpoint::transfer_blocking`, `Endpoint::wait_next_complete`,
`Endpoint::submit`, `Endpoint::pending`, `Endpoint::cancel_all`,
`Endpoint::allocate`, `Endpoint::max_packet_size`, `Interface::endpoint`,
`Device::active_configuration`, all descriptor accessors.

### Imports (exactly as mtp-rs uses them)

```rust
use nusb::descriptors::{InterfaceDescriptor, TransferType};
use nusb::transfer::{Buffer, Bulk, ControlOut, ControlIn, ControlType,
                     Direction, In, Interrupt, Out, Recipient, TransferError};
use nusb::MaybeFuture;
// also: nusb::{Device, DeviceInfo, Interface, Endpoint, Error, ErrorKind, Speed}
```

---

## 1. Enumeration

```rust
let devices = blocking(nusb::list_devices())?; // Result<impl Iterator<Item = DeviceInfo>, Error>
```

`DeviceInfo` accessors (synchronous, no open needed): `.vendor_id() -> u16`,
`.product_id() -> u16`, `.manufacturer_string() -> Option<&str>`,
`.product_string() -> Option<&str>`, `.serial_number() -> Option<&str>`,
`.class()/.subclass()/.protocol() -> u8`, `.speed() -> Option<Speed>`,
`.bus_id() -> &str`, `.port_chain()` (iterable), `.interfaces()` (summary
InterfaceInfo without opening: `.class()/.subclass()/.protocol() -> u8`,
`.interface_string() -> Option<&str>`), `.open() -> impl MaybeFuture<...>`.

> bcdDevice/bcdUSB: mtp-rs never reads them. nusb exposes `device_version()
> -> u16` upstream but that's unverified against this reference. Our UsbInfo
> needs bcd_device — treat as best-effort; verify on the phone at M2.

`Speed` is `#[non_exhaustive]`: `Low/Full/High/Super/SuperPlus` — match with a
`_ =>` arm. It's the negotiated speed, not a capability.

After `open()`: `device.active_configuration()? -> Configuration` (sync).
`config.interfaces()` yields groups; `.interface_number() -> u8`,
`.alt_settings().next() -> InterfaceDescriptor` with `.class()/.subclass()/
.protocol() -> u8` and `.endpoints()` yielding `EndpointDescriptor`:
`.address() -> u8`, `.direction() -> Direction`, `.transfer_type() -> TransferType`.

### MTP interface identification (endpoint-shape heuristic)

MTP class codes: `0x06` (Still Image) OR `0xFF` (vendor), subclass `0x01`,
protocol `0x01`. Vendor devices don't set standard subclass/protocol, so fall
back to endpoint layout — exactly one bulk-IN + one bulk-OUT + one
interrupt-IN:

```rust
fn has_mtp_endpoint_layout(alt: &InterfaceDescriptor) -> bool {
    let (mut bin, mut bout, mut iin) = (false, false, false);
    for ep in alt.endpoints() {
        match (ep.direction(), ep.transfer_type()) {
            (Direction::In,  TransferType::Bulk)      => bin  = true,
            (Direction::Out, TransferType::Bulk)      => bout = true,
            (Direction::In,  TransferType::Interrupt) => iin  = true,
            _ => {}
        }
    }
    bin && bout && iin
}
```

---

## 2. Blocking bulk transfer with a timeout — THE critical pattern

No single "blocking bulk with timeout" call exists. Three patterns on a typed
`nusb::Endpoint<T, D>`:

```rust
let bulk_in:  Endpoint<Bulk, In>      = interface.endpoint::<Bulk, In>(bulk_in_addr)?;
let bulk_out: Endpoint<Bulk, Out>     = interface.endpoint::<Bulk, Out>(bulk_out_addr)?;
let intr_in:  Endpoint<Interrupt, In> = interface.endpoint::<Interrupt, In>(intr_in_addr)?;
```

Endpoint methods: `.max_packet_size() -> usize`, `.pending() -> usize`,
`.allocate(size) -> Buffer`, `.submit(buf: Buffer)`, `.cancel_all()`,
`blocking(ep.clear_halt())` (requires `pending()==0`).

### Pattern A — bulk OUT one-shot (send a command): `transfer_blocking`

Submits and blocks the thread up to `timeout`; **on timeout cancels internally
→ `Err(Cancelled)`** (map to Timeout, retryable):

```rust
let buf: Buffer = data.to_vec().into();              // Vec<u8> -> Buffer via From
let completion = bulk_out.transfer_blocking(buf, timeout); // -> Completion
completion.status?;  // Result<(), TransferError>
```

### Pattern B — bulk OUT streaming: `submit` + `wait_next_complete`

```rust
let mut buf = ep.allocate(transfer_size);
buf.extend_from_slice(chunk);
ep.submit(buf);
let c = ep.wait_next_complete(timeout).ok_or(Timeout)?; // Option<Completion>, None = timeout
c.status?;
// ZLP to delimit when the final transfer is a multiple of max_packet_size:
ep.submit(Buffer::new(0));
ep.wait_next_complete(timeout).ok_or(Timeout)?;
```

### Pattern C — bulk IN resumable: `submit` + async timer race, leave pending on timeout

`next_complete()` is **cancel-safe** — dropping its future does NOT cancel the
USB transfer. On timeout, DON'T cancel; the transfer stays in-flight and the
next call resumes it (via `pending()`), so no data is lost:

```rust
if ep.pending() == 0 {
    let n = align_to_packet_size(max_size, ep.max_packet_size()); // IN buf = non-zero multiple of MPS
    ep.submit(Buffer::new(n));                                    // Buffer::new(n) = n zeroed bytes
}
let completed = match futures::future::select(
    Box::pin(ep.next_complete()),
    Box::pin(futures_timer::Delay::new(timeout)),
).await {
    futures::future::Either::Left((c, _)) => Some(c),  // finished
    futures::future::Either::Right(_)     => None,     // timeout: LEAVE pending
};
match completed {
    Some(c) => { c.status?; Ok(c.buffer[..c.actual_len].to_vec()) }
    None    => Err(Timeout),  // do NOT cancel; next call resumes
}

fn align_to_packet_size(size: usize, mps: usize) -> usize {
    if mps == 0 { return size.max(1); }
    if size == 0 { return mps; }
    if size % mps == 0 { size } else { ((size / mps) + 1) * mps }
}
```

**Timeout enforcement:** Patterns A/B use nusb's blocking-with-deadline calls;
Pattern C uses an async timer race vs `futures_timer::Delay`. There is NO
`futures_lite::block_on` and NO dedicated runtime.

`Buffer`: `Buffer::new(n)` (zeroed; `Buffer::new(0)` = ZLP), `Vec<u8> -> Buffer`
via `.into()`, `.len()`, `.remaining_capacity()`, `.extend_from_slice()`.
`Completion`: `.status: Result<(), TransferError>`, `.actual_len: usize`,
`.buffer: Buffer` (index `comp.buffer[..comp.actual_len]`).

### `TransferError` mapping

```rust
match err {
    TransferError::Cancelled    => Timeout,       // transfer_blocking cancels on timeout
    TransferError::Disconnected => Disconnected,  // device left the bus (§6)
    TransferError::Stall        => { blocking(ep.clear_halt()); /* then Io */ }
    TransferError::Fault | TransferError::InvalidArgument | TransferError::Unknown(_)
        => Io(err.to_string()),
}
```

A stalled endpoint stays wedged across process restarts until `clear_halt()`.

### Control transfers (endpoint 0) — genuinely async, take a Duration, `.await`

```rust
interface.control_out(ControlOut {
    control_type: ControlType::Class, recipient: Recipient::Interface,
    request: 0x64, value: 0, index: interface_number as u16, data: &payload,
}, Duration::from_millis(300)).await?;                // Result<(), TransferError>

let data: Vec<u8> = interface.control_in(ControlIn {
    control_type: ControlType::Class, recipient: Recipient::Interface,
    request: 0x67, value: 0, index: interface_number as u16, length: 64,
}, Duration::from_millis(300)).await?;                // Result<Vec<u8>, TransferError>
```

---

## 3. Claiming + the nusb #206 fallback

```rust
let interface = match blocking(device.claim_interface(iface_num)) {
    Ok(i) => i,
    // nusb #206: macOS won't publish IOUSBHostInterface services for
    // vendor-class/class-0 devices with no matching driver → claim fails
    // NotFound. SetConfiguration(1) forces IOKit to publish them; re-claim.
    #[cfg(target_os = "macos")]
    Err(e) if matches!(e.kind(), nusb::ErrorKind::NotFound) => {
        blocking(device.set_configuration(1))?;
        blocking(device.claim_interface(iface_num))?
    }
    Err(e) => return Err(e.into()),
};
```

mtp-rs **does** implement this fallback, gated to macOS, triggered by
`ErrorKind::NotFound`. No kernel-driver detach on macOS. Open endpoints after
claim: `interface.endpoint::<Bulk, In>(addr)? -> Result<Endpoint<Bulk,In>, Error>`.

---

## 4. Exclusive access / ptpcamerad

**mtp-rs only CLASSIFIES it — there is ZERO IOKit code in the repo.** Naming
the offending process (ptpcamerad, Image Capture, Photos) is **keel's job to
build from scratch.**

Typed detection (canonical):

```rust
use nusb::ErrorKind as Usb;
match usb.kind() {
    Usb::Busy             => Error::ExclusiveAccess,  // == macOS kIOReturnExclusiveAccess (also Linux EBUSY)
    Usb::PermissionDenied => Error::PermissionDenied,
    Usb::Disconnected | Usb::NotFound => Error::Disconnected,
    Usb::Unsupported      => Error::Unsupported,
    _                     => Error::Io(usb.to_string()),
}
```

**`nusb::ErrorKind::Busy` is the `kIOReturnExclusiveAccess` signal on macOS.**
String fallback also matches `"exclusive access"` / `"device or resource busy"`.
Distinct macOS case: user-client denial surfaces as `"failed to create IOKit
PlugInInterface"` (not exclusive access) — show launch-from-Terminal guidance.

**keel adds:** on `ErrorKind::Busy`, open the device's IORegistry entry and read
the `UsbExclusiveOwner` string property to name the holder. IOKit
(`IOServiceGetMatchingService` + `IORegistryEntryCreateCFProperty`) — entirely
ours; mtp-rs gives no reference for it.

---

## 5. Device reset

mtp-rs uses NO nusb port-reset. It does a **SIC (Still Image Class)
DEVICE_RESET** class control transfer, then resyncs endpoints:

```rust
// 1. SIC DEVICE_RESET (bRequest=0x66, no payload) — returns device to Idle, no re-enumeration.
interface.control_out(ControlOut { control_type: ControlType::Class,
    recipient: Recipient::Interface, request: 0x66, value: 0,
    index: iface_num as u16, data: &[] }, Duration::from_secs(1)).await?;
// 2. Both bulk endpoints: cancel_all → drain to pending()==0 → clear_halt (resets toggles).
// 3. Drain stale bulk-IN containers (300ms idle race).
```

macOS: the class reset returns to Idle WITHOUT bus re-enumeration — what you
want. Avoid a real USB port reset on macOS.

---

## 6. Device-gone (→ our `LIBUSB_ERROR_NO_DEVICE`)

- On a transfer: `TransferError::Disconnected` (from `completion.status`).
- On open/claim/control: `nusb::Error` with `.kind() == ErrorKind::Disconnected`
  (defensively also `ErrorKind::NotFound` post-enumeration).

Map all of these to `TransportError::DeviceGone`, whose Display contains
`LIBUSB_ERROR_NO_DEVICE` (the FFI error mapper string-matches it).

---

## 7. Cancellation

**(a) Cooperative between roundtrips** — `CancelToken` = `Arc<AtomicBool>`,
checked at loop boundaries in long ops; returns Cancelled within one roundtrip.

**(b) Mid-transfer USB cancel** — `ep.cancel_all()` then drain:

```rust
ep.cancel_all();
while ep.pending() > 0 { let _ = ep.next_complete().await; }
```

Full mid-download sequence (order matters, mirrors libmtp): CLASS_CANCEL
control (`bRequest=0x64`, 6-byte payload: event code `0x4001` LE + tid LE) →
drain bulk-IN watching for the Response container (type 3 at bytes [4..6]) →
`cancel_all` + drain → drain interrupt pipe → poll GET_DEVICE_STATUS
(`bRequest=0x67`) until not `Device_Busy` (`0x2019`) — Android doesn't
implement this request, ignore its failure. GET_DEVICE_STATUS comes AFTER the
drains, never between CLASS_CANCEL and the drain.

---

## 8. Divergences to ignore (mtp-rs is cross-platform)

- **Windows WPD backend** (`src/mtp/backend/wpd/*`, `#[cfg(windows)]`, `windows`
  crate) — ignore entirely; unrelated to nusb.
- `is_exclusive_access` multi-platform string matching — for macOS you only need
  `ErrorKind::Busy` + `"exclusive access"`.
- Linux EACCES/udev handling — irrelevant on macOS.
- **No IORegistry code exists** despite doc-comments mentioning UsbExclusiveOwner
  — keel writes that.
- No `Device::reset()` — SIC class reset instead (§5).
- Runtime-agnostic: mtp-rs mixes blocking + async calls and avoids nusb runtime
  features. If keel standardizes on one runtime it *could* enable a nusb feature
  and `.await` the MaybeFutures — if not, obey the `.wait()` rule (§0).
- Each endpoint wrapped in `futures::lock::Mutex` for `Sync` — a structural
  choice; keel can own endpoints exclusively instead.

---

## Reference source paths

- `.../mtp-rs/crates/mtp-rs/src/transport/nusb.rs` — core (all nusb 0.2 usage)
- `.../src/transport/mod.rs` — Transport trait
- `.../src/error.rs` — `is_exclusive_access` string matching
- `.../src/mtp/error.rs` (141–172) — typed `ErrorKind` → error mapping
- `.../src/cancel.rs` — `CancelToken`
- `.../src/ptp/session/streaming.rs` (233–401) — mid-transfer cancel
- `.../crates/mtp-rs-cli/src/cli/error.rs` (150–200) — macOS PlugInInterface detection

# Ferry

⛴️ Ferry is a small, native macOS app for copying files between an Android
phone and a Mac over USB MTP (Android's “File transfer” mode). It's primarily
built for my own use.

The MTP/PTP protocol runs on **keel**, a
from-scratch, pure-Rust kernel with no libusb and no C dependencies — USB goes
straight through [nusb](https://github.com/kevinmehall/nusb) over IOKit. The
interface is SwiftUI with Liquid Glass. Ferry *copies* files; it never deletes
the source or performs a destructive “move”.

## Requirements

- macOS 26.0 or newer
- An Android phone with a USB data cable
- USB mode set to **File transfer / Android Auto**
- For building: Xcode Command Line Tools, Swift 6, Rust/Cargo, and the macOS
  `hdiutil` utility

The included release script builds for the architecture of the Mac on which it
runs. The checked-in binary and generated DMG are intended for Apple
Silicon unless rebuilt on Intel.

## Install the DMG

Build it from source:

```sh
./scripts/make-dmg.sh
```

The result is `dist/Ferry.dmg`. Open it, drag `Ferry.app` to Applications,
then launch it. The app and its Rust kernel are ad-hoc signed for local use;
there is no notarization step.

If macOS blocks the first launch, Control-click the app and choose **Open**,
then confirm. If that option is not offered, remove the download quarantine
after checking that you trust the file:

```sh
xattr -dr com.apple.quarantine /Applications/Ferry.app
```

## Use Ferry

1. Connect and unlock the Android phone.
2. Tap the phone's USB notification and choose **File transfer / Android
   Auto**.
3. Close Android File Transfer, Smart Switch, Kies, or another MTP client if
   one is running; only one app can own the phone's USB session.
4. Ferry detects the phone automatically.
5. Browse the phone, add files to the shelf, and choose **Send to Mac…**, or
   drag files from Finder onto the phone view to upload them.

The app supports browsing, search, thumbnails when the device provides them,
folders, rename, delete with confirmation, drag and drop, queued transfers,
progress, cancellation, Finder reveal, and Quick Look previews. MTP is a
serial protocol, so queued jobs run one at a time.

## Build and test

Build the Rust kernel and a clickable app:

```sh
./scripts/build-keel.sh
swift build -c release
./scripts/make-app.sh
```

For development:

```sh
swift run FerryApp
```

Run the Rust test suite:

```sh
cd keel
cargo test --workspace
```

The probe is useful for diagnosing a connected phone without launching the
UI:

```sh
swift run ferry-probe
```

Set `KEEL_LIB_DIR` when the kernel dylib is somewhere other than the bundled
app location or the architecture-specific `Libraries/` directory.

## Project layout

```text
Sources/FerryApp/    macOS application entry point
Sources/FerryUI/     SwiftUI views and the device-facing session state machine
Sources/KeelKit/     Swift async facade and C ABI bridge
Sources/FerryProbe/  command-line harness for diagnosing a connected phone
keel/                Rust MTP/PTP kernel (7 crates) and its tests
Libraries/           architecture-specific kernel dylibs (checked in for convenience)
scripts/             kernel, app, icon, and DMG build helpers
```

The runtime path is:

```text
SwiftUI → KeelEngine actor → KeelKit FFI → keel.dylib → USB/MTP device
```

The C ABI uses JSON envelopes and callbacks. Ferry serializes kernel calls
because both the callback slots and an MTP session support one operation at a
time. Transfer progress is sampled for a lightweight UI, with the terminal
progress snapshot delivered before completion so short transfers still report
their file count.

## Scope and limitations

- This is USB MTP, not ADB, Wi-Fi transfer, or an iPhone tool.
- The app requires macOS 26 because it uses current SwiftUI and Liquid Glass
  APIs.
- A DMG is built for the host architecture; it is not a universal binary.
- Android device behavior varies by vendor and USB mode.
- “Skip files that already exist” currently applies to top-level downloads;
  files inside downloaded folders are handled by the kernel as a batch.
- Ferry transfers files locally over USB using Android's MTP interface. It has no
  server or cloud component and does not add application-level encryption.
  Security relies on USB authorization and the storage protections provided by
  Android and macOS.

## License and notices

Ferry source code is released under the [MIT License](LICENSE). Third-party
lineage and dependency notes are in [NOTICE.md](NOTICE.md). The Rust workspace
also declares MIT licensing for its crates and uses permissive dependencies.

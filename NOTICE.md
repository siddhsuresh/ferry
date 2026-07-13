# Ferry — notices and attribution

## Project license

Ferry's source code, including the Swift application and the in-tree Rust
crates under `keel/`, is licensed under the MIT License in [LICENSE](LICENSE).

## Current runtime

The application ships Ferry's SwiftUI code and the in-tree Rust
`keel.dylib` MTP/PTP kernel. The kernel uses `nusb` for USB access and links
against macOS system frameworks and libraries supplied by macOS, including
IOKit and CoreFoundation.

Ferry does not bundle the Go MTP kernel, `kalam.dylib`, `libusb`, or the
OpenMTP application.

## Direct Rust dependencies

The direct third-party dependencies declared by the Rust workspace are
permissively licensed:

| Dependency | License expression |
| --- | --- |
| `nusb` 0.2.4 | Apache-2.0 OR MIT |
| `serde` 1.0.228 | MIT OR Apache-2.0 |
| `serde_json` 1.0.150 | MIT OR Apache-2.0 |
| `libc` 0.2.186 | MIT OR Apache-2.0 |
| `log` 0.4.33 | MIT OR Apache-2.0 |
| `env_logger` 0.11.11 | MIT OR Apache-2.0 |
| `futures` 0.3.32 | MIT OR Apache-2.0 |
| `futures-timer` 3.0.4 | MIT/Apache-2.0 |
| `rand` 0.8.7 | MIT OR Apache-2.0 |
| `core-foundation` 0.10.1 | MIT OR Apache-2.0 |
| `io-kit-sys` 0.5.0 | MIT OR Apache-2.0 |

The exact resolved dependency graph is recorded in `keel/Cargo.lock`. License
metadata for that graph can be inspected with:

```sh
cd keel
cargo metadata --format-version 1
```

## Historical attribution

The `keel` implementation is a Rust reimplementation of MTP/PTP behavior
based on the public USB-IF specifications and compatibility observations from
`go-mtpfs`, `go-mtpx`, and OpenMTP by Ganesh Rathinavel. Those projects are
acknowledged as references only; their source code, binaries, and repositories
are not components of the current Ferry distribution.

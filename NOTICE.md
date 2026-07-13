# Ferry — notices

## License

Ferry's source code, including the Swift application and the Rust crates under
`keel/`, is licensed under the MIT License in [LICENSE](LICENSE).

## Dependencies

The Rust portion of Ferry uses the following direct third-party dependencies:

| Dependency | License |
| --- | --- |
| `nusb` 0.2.4 | Apache-2.0 OR MIT |
| `serde` 1.0.228 | MIT OR Apache-2.0 |
| `serde_json` 1.0.150 | MIT OR Apache-2.0 |
| `libc` 0.2.186 | MIT OR Apache-2.0 |
| `log` 0.4.33 | MIT OR Apache-2.0 |
| `env_logger` 0.11.11 | MIT OR Apache-2.0 |
| `futures` 0.3.32 | MIT OR Apache-2.0 |
| `futures-timer` 3.0.4 | MIT OR Apache-2.0 |
| `rand` 0.8.7 | MIT OR Apache-2.0 |
| `core-foundation` 0.10.1 | MIT OR Apache-2.0 |
| `io-kit-sys` 0.5.0 | MIT OR Apache-2.0 |

The complete resolved dependency graph is recorded in `keel/Cargo.lock`. To
inspect it, including the transitive dependencies:

```sh
cd keel
cargo tree                # the full graph
cargo license             # per-crate license roll-up (needs cargo-license)
```

The Swift application (the `Sources/` targets) has **no third-party
dependencies** — it links only Apple's macOS system frameworks and libraries,
including SwiftUI, AppKit, IOKit, and CoreFoundation. These are supplied by
macOS and are not bundled third-party components.

## Attribution

Ferry's MTP/PTP implementation is in-tree Rust code developed from the public
USB-IF specifications and compatibility research informed by OpenMTP,
`go-mtpfs`, and `go-mtpx`. Those projects are acknowledged for attribution;
their source code and binaries are not included in Ferry.

The JSON fixtures under `keel/fixtures/golden/` are behavioral test captures —
recorded output of a prior kernel observed against a real device — used to
verify Ferry's own implementation. They are test data, not third-party source
code or binaries.

#!/bin/zsh
# Builds the Rust MTP kernel (keel) and installs keel.dylib into
# Libraries/<arch>/ where KeelLibrary loads it. Pure Rust, self-contained.
#
# Requires: rustup/cargo. No system libraries (nusb links statically over IOKit).
#
#   ./scripts/build-keel.sh
set -euo pipefail

cd "$(dirname "$0")/.."

ARCH=$(uname -m)          # arm64 on Apple Silicon
[[ "$ARCH" == "x86_64" ]] && ARCH=amd64
OUT="Libraries/$ARCH"
mkdir -p "$OUT"

echo "building keel (release)…"
( cd keel && cargo build --release -p keel-ffi )

cp keel/target/release/libkeel.dylib "$OUT/keel.dylib"
install_name_tool -id "keel.dylib" "$OUT/keel.dylib" 2>/dev/null || true

# dlopen on Apple Silicon requires a signature; ad-hoc is fine locally.
codesign --force --sign - "$OUT/keel.dylib"

echo "installed $OUT/keel.dylib"
otool -L "$OUT/keel.dylib" | sed -n '2,6p'

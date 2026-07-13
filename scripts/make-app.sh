#!/bin/zsh
# Assembles a double-clickable Ferry.app from the release build.
# Usage: ./scripts/make-app.sh [--install]   (--install copies to /Applications)
set -euo pipefail

cd "$(dirname "$0")/.."

# Build the pure-Rust kernel first (keel.dylib → Libraries/<arch>/).
./scripts/build-keel.sh

echo "building release…"
swift build -c release

ARCH=$(uname -m); [[ "$ARCH" == "x86_64" ]] && ARCH=amd64

APP=dist/Ferry.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin"
cp .build/release/FerryApp "$APP/Contents/MacOS/Ferry"

# The self-contained Rust kernel rides along in Resources/bin —
# KeelLibrary.defaultLibraryDirectory() finds it there. Pure Rust, self-contained.
cp assets/AppIcon.icns "$APP/Contents/Resources/"
cp "Libraries/$ARCH/keel.dylib" "$APP/Contents/Resources/bin/"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>              <string>Ferry</string>
    <key>CFBundleDisplayName</key>       <string>Ferry</string>
    <key>CFBundleIdentifier</key>        <string>com.siddharth.ferry</string>
    <key>CFBundleExecutable</key>        <string>Ferry</string>
    <key>CFBundlePackageType</key>       <string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1</string>
    <key>CFBundleVersion</key>            <string>1</string>
    <key>LSMinimumSystemVersion</key>    <string>26.0</string>
    <key>NSHighResolutionCapable</key>   <true/>
    <key>CFBundleIconFile</key>          <string>AppIcon</string>
    <key>UTExportedTypeDeclarations</key>
    <array>
        <dict>
            <key>UTTypeIdentifier</key>          <string>com.siddharth.ferry.phonefile</string>
            <key>UTTypeDescription</key>         <string>Ferry phone file reference</string>
            <key>UTTypeConformsTo</key>          <array><string>public.data</string></array>
            <key>UTTypeTagSpecification</key>    <dict/>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Ad-hoc sign: personal machine only, never distributed.
codesign --force --deep --sign - "$APP"
echo "built $APP"

if [[ "${1:-}" == "--install" ]]; then
    rm -rf /Applications/Ferry.app
    cp -R "$APP" /Applications/
    echo "installed /Applications/Ferry.app"
fi

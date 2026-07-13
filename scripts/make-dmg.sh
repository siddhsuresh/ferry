#!/bin/zsh
# Builds Ferry.app and wraps it in an unsigned/ad-hoc macOS DMG.
# Usage: ./scripts/make-dmg.sh
set -euo pipefail

cd "$(dirname "$0")/.."

./scripts/make-app.sh

STAGING="dist/Ferry-dmg"
DMG="dist/Ferry.dmg"
rm -rf "$STAGING" "$DMG"
mkdir -p "$STAGING"
cp -R "dist/Ferry.app" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

hdiutil create \
    -volname "Ferry" \
    -srcfolder "$STAGING" \
    -ov \
    -format UDZO \
    "$DMG" >/dev/null

rm -rf "$STAGING"
echo "built $DMG"

#!/usr/bin/env bash
# Build "Nerda Clipboard.app" into dist/. With --install, copy it to ~/Applications and start it.
#
# Signing: set SIGN_IDENTITY to a certificate name ("Apple Development: ...") so macOS keeps
# the Accessibility permission across rebuilds. Without it the bundle is signed ad hoc, and
# macOS treats every build as a new app that must be allowed again.
set -euo pipefail

cd "$(dirname "$0")/.."
NAME="Nerda Clipboard"
BUNDLE_ID="nerda.clipboard"
EXE="nerda-clipboard"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="dist/$NAME.app"

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "target/release/$EXE" "$APP/Contents/MacOS/$EXE"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$EXE</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign "${SIGN_IDENTITY:--}" "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
  DEST="$HOME/Applications/$NAME.app"
  mkdir -p "$HOME/Applications"
  pkill -x "$EXE" 2>/dev/null || true
  rm -rf "$DEST"
  cp -R "$APP" "$DEST"
  open "$DEST"
  echo "Installed and started $DEST"
fi

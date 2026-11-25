#!/bin/sh
# Builds "Animalese Typing.app" (a menu-bar app with no Dock icon) in target/release.
#   ./macos/bundle.sh            build for this Mac's CPU
#   ./macos/bundle.sh universal  build one app for both Apple Silicon and Intel
set -eu
cd "$(dirname "$0")/.."

APP="target/release/Animalese Typing.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

if [ "${1:-}" = "universal" ]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    BIN=target/release/animalese-rs-universal
    lipo -create -output "$BIN" \
        target/aarch64-apple-darwin/release/animalese-rs \
        target/x86_64-apple-darwin/release/animalese-rs
else
    cargo build --release
    BIN=target/release/animalese-rs
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/animalese-rs"

# App icon from the 256px PNG.
ICONSET=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256; do
    sips -z $s $s assets/images/icon_256x256.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    if [ $d -le 256 ]; then
        sips -z $d $d assets/images/icon_256x256.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
    fi
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Animalese Typing</string>
    <key>CFBundleDisplayName</key><string>Animalese Typing</string>
    <key>CFBundleIdentifier</key><string>com.github.abbysimple.animalese-rs</string>
    <key>CFBundleExecutable</key><string>animalese-rs</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so macOS can remember the Input Monitoring permission for the app.
codesign --force --deep --sign - "$APP"
echo "Built $APP"

#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile=${1:-debug}
case "${GRADIENT_ICON_APPEARANCE:-dark}" in
  dark) bundle_icon=Gradient ;;
  light) bundle_icon=Gradient-Light ;;
  *) echo 'GRADIENT_ICON_APPEARANCE must be dark or light' >&2; exit 1 ;;
esac
case "$profile" in
  release)
    cargo build --release --locked
    binary=target/release/gradient ;;
  debug)
    cargo build --locked
    binary=target/debug/gradient ;;
  universal)
    cargo build --release --locked --target aarch64-apple-darwin
    cargo build --release --locked --target x86_64-apple-darwin
    mkdir -p target/universal
    lipo -create -output target/universal/gradient \
      target/aarch64-apple-darwin/release/gradient target/x86_64-apple-darwin/release/gradient
    binary=target/universal/gradient ;;
  *) echo 'Usage: scripts/bundle-macos.sh [debug|release|universal]' >&2; exit 1 ;;
esac
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
bundle="$PWD/dist/Gradient.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary" "$bundle/Contents/MacOS/gradient.new"
mv "$bundle/Contents/MacOS/gradient.new" "$bundle/Contents/MacOS/gradient"

for variant in Gradient Gradient-Light; do
  source=assets/icon.png
  [ "$variant" != Gradient-Light ] || source=assets/icon-light.png
  iconset="$PWD/target/$variant.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    sips -z $size $size "$source" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$source" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/$variant.icns"
done

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>gradient</string>
<key>CFBundleIdentifier</key><string>com.misc.gradient</string>
<key>CFBundleName</key><string>Gradient</string>
<key>CFBundleDisplayName</key><string>Gradient</string>
<key>CFBundleIconFile</key><string>$bundle_icon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSCameraUsageDescription</key><string>Websites you allow can use the camera.</string>
<key>NSMicrophoneUsageDescription</key><string>Websites you allow can use the microphone.</string>
<key>NSLocationUsageDescription</key><string>Websites you allow can use your location.</string>
<key>NSLocationWhenInUseUsageDescription</key><string>Websites you allow can use your location.</string>
<key>NSAppTransportSecurity</key><dict>
<key>NSAllowsArbitraryLoadsInWebContent</key><true/>
</dict>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>Web address</string>
<key>CFBundleURLSchemes</key><array><string>http</string><string>https</string></array>
<key>CFBundleTypeRole</key><string>Viewer</string>
</dict></array>
</dict></plist>
PLIST

# SIGNING_IDENTITY (a Developer ID Application certificate) signs for
# distribution: hardened runtime, secure timestamp and the entitlements the
# runtime needs for camera, microphone and location. Otherwise ad hoc.
if [ -n "${SIGNING_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp \
    --entitlements scripts/gradient.entitlements --sign "$SIGNING_IDENTITY" "$bundle"
else
  codesign --force --deep --sign - "$bundle"
fi
echo "$bundle"

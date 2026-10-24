#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile=${1:-debug}
case "$profile" in
  release) cargo build --release --locked ;;
  debug) cargo build --locked ;;
  *) echo 'Usage: scripts/bundle-macos.sh [debug|release]' >&2; exit 1 ;;
esac
bundle="$PWD/dist/Browser.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/browser" "$bundle/Contents/MacOS/browser.new"
mv "$bundle/Contents/MacOS/browser.new" "$bundle/Contents/MacOS/browser"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>browser</string>
<key>CFBundleIdentifier</key><string>dev.rafi.browser</string>
<key>CFBundleName</key><string>Browser</string>
<key>CFBundleDisplayName</key><string>Browser</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
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
codesign --force --deep --sign - "$bundle"
echo "$bundle"

#!/bin/sh
# Builds a universal, Developer ID signed and notarized Gradient ZIP:
#   SIGNING_IDENTITY="Developer ID Application: …" NOTARY_PROFILE=… scripts/release-macos.sh
# NOTARY_PROFILE names credentials saved with `xcrun notarytool store-credentials`.
set -eu
cd "$(dirname "$0")/.."
: "${SIGNING_IDENTITY:?Set SIGNING_IDENTITY to a Developer ID Application certificate}"
: "${NOTARY_PROFILE:?Set NOTARY_PROFILE to a notarytool keychain profile}"
export SIGNING_IDENTITY
app=$(scripts/bundle-macos.sh universal)
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
zip="dist/Gradient-$version-mac.zip"
codesign --verify --strict "$app"

# Notarize a ZIP of the app, staple the ticket to the app, then zip it again.
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
xcrun notarytool submit "$zip" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute "$app"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
shasum -a 256 "$zip"

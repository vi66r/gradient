#!/bin/sh
# Builds a universal Gradient, has Xcode re-sign it with the team's Developer
# ID (cloud-managed, so no local certificate) and notarize it, then zips it:
#   SIGNING_IDENTITY="Apple Development: …" DEVELOPMENT_TEAM=<team ID> scripts/release-macos.sh
# Xcode must be signed in to an account on that team; SIGNING_IDENTITY is any
# of the team's development certificates, used only to sign the archive.
set -eu
cd "$(dirname "$0")/.."
: "${SIGNING_IDENTITY:?Set SIGNING_IDENTITY to an Apple Development certificate on the team}"
: "${DEVELOPMENT_TEAM:?Set DEVELOPMENT_TEAM to the Apple team ID}"
export SIGNING_IDENTITY
app=$(scripts/bundle-macos.sh universal)
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
work="$PWD/target/release-archive"
archive="$work/Gradient.xcarchive"
rm -rf "$work"
mkdir -p "$archive/Products/Applications" "$archive/dSYMs"
ditto "$app" "$archive/Products/Applications/Gradient.app"
VERSION=$version WORK=$work python3 - <<'PY'
import datetime, os, plistlib
work, version = os.environ['WORK'], os.environ['VERSION']
plistlib.dump({'ApplicationProperties': {'ApplicationPath': 'Applications/Gradient.app',
    'Architectures': ['arm64', 'x86_64'], 'CFBundleIdentifier': 'com.misc.gradient',
    'CFBundleShortVersionString': version, 'CFBundleVersion': version,
    'SigningIdentity': os.environ['SIGNING_IDENTITY'], 'Team': os.environ['DEVELOPMENT_TEAM']},
    'ArchiveVersion': 2, 'CreationDate': datetime.datetime.now(), 'Name': 'Gradient',
    'SchemeName': 'Gradient'}, open(f'{work}/Gradient.xcarchive/Info.plist', 'wb'))
plistlib.dump({'method': 'developer-id', 'destination': 'upload', 'signingStyle': 'automatic',
    'teamID': os.environ['DEVELOPMENT_TEAM']}, open(f'{work}/export.plist', 'wb'))
PY
xcodebuild -exportArchive -archivePath "$archive" -exportOptionsPlist "$work/export.plist" \
  -allowProvisioningUpdates

# Apple notarizes the upload; wait up to 20 minutes for the stapled app.
tries=0
until out=$(xcodebuild -exportNotarizedApp -archivePath "$archive" -exportPath "$work/notarized" 2>&1); do
  tries=$((tries + 1))
  case "$out" in
    *"is processing"*) [ $tries -lt 60 ] || { echo 'Notarization is still processing.' >&2; exit 1; } ;;
    *) echo "$out" >&2; exit 1 ;;
  esac
  sleep 20
done
notarized="$work/notarized/Gradient.app"
codesign --verify --deep --strict "$notarized"
xcrun stapler validate "$notarized"
spctl --assess --type execute "$notarized"
zip="dist/Gradient-$version-mac.zip"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$notarized" "$zip"
shasum -a 256 "$zip"

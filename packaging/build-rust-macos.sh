#!/usr/bin/env bash
# Build the macOS application bundle from the Rust workspace, signed when an identity is available.
#
# Usage: packaging/build-rust-macos.sh [output-directory]
#
# KEYSWITCH_SIGN_IDENTITY names the Developer ID to sign with, KEYSWITCH_SIGN_KEYCHAIN the keychain
# holding it. Without an identity the bundle is built unsigned, which is enough to run it here but
# not to hand it to anyone: an unsigned bundle loses its Accessibility permission on every update.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/Cargo.toml" | head -n 1)"
# The oldest macOS the README promises.
minimum_macos="13.0"
sign_identity="${KEYSWITCH_SIGN_IDENTITY:-}"
sign_keychain="${KEYSWITCH_SIGN_KEYCHAIN:-}"
bundle_identifier="io.github.olegius88.KeySwitch"
app_name="KeySwitch"
native_output="$output_dir/macos-native"
bundle="$native_output/$app_name.app"
frozen_model_sources="$project_dir/model/intent_v1/sources"

# shellcheck source=packaging/macos-signing.sh
source "$project_dir/packaging/macos-signing.sh"

if [[ "$(uname -s)" != "Darwin" ]]; then
    printf 'This build runs on macOS only.\n' >&2
    exit 1
fi

export MACOSX_DEPLOYMENT_TARGET="$minimum_macos"
cargo build --release --locked -p ks-mac --manifest-path "$project_dir/Cargo.toml"

rm -rf "$native_output"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/resources"
cp "$project_dir/target/release/$app_name" "$bundle/Contents/MacOS/$app_name"
strip -x "$bundle/Contents/MacOS/$app_name"
# The helper of Wayland and the stylesheet of GTK are Linux only.
(cd "$project_dir/resources" && find . -type f ! -path './wayland-input/*' ! -name style.css -print0 \
    | xargs -0 tar -cf - ) | tar -xf - -C "$bundle/Contents/Resources/resources"
cp "$frozen_model_sources/en_US.lm" "$frozen_model_sources/ru_RU.lm" "$bundle/Contents/Resources/resources/models/"
mkdir -p "$bundle/Contents/Resources/licenses"
cp "$project_dir/LICENSE" "$bundle/Contents/Resources/licenses/LICENSE.KeySwitch.txt"
cp "$frozen_model_sources/COPYRIGHT.onboard-data" "$bundle/Contents/Resources/licenses/COPYRIGHT.onboard-data.txt"
cp "$project_dir/packaging/third-party-licenses/macos.txt" "$bundle/Contents/Resources/licenses/third-party-rust.txt"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key><string>$app_name</string>
    <key>CFBundleExecutable</key><string>$app_name</string>
    <key>CFBundleIdentifier</key><string>$bundle_identifier</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>$app_name</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>$minimum_macos</string>
    <key>LSMultipleInstancesProhibited</key><true/>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist" >/dev/null

# The deployment target decides the minimum, never the image the build happened to run on.
built_for="$(otool -l "$bundle/Contents/MacOS/$app_name" | awk '/cmd LC_BUILD_VERSION/ { b = 1 } b && $1 == "minos" { print $2; exit }')"
if [[ "$(printf '%s\n%s\n' "$built_for" "$minimum_macos" | sort -V | tail -n 1)" != "$minimum_macos" ]]; then
    printf 'The bundle needs macOS %s, newer than %s.\n' "$built_for" "$minimum_macos" >&2
    exit 1
fi

if [[ -n "$sign_identity" ]]; then
    keychain_arguments=()
    [[ -n "$sign_keychain" ]] && keychain_arguments+=(--keychain "$sign_keychain")
    run_signing codesign --force --options runtime --timestamp \
        ${keychain_arguments[@]+"${keychain_arguments[@]}"} \
        --sign "$sign_identity" "$bundle"
    codesign --verify --strict --deep --verbose=2 "$bundle"
fi

# Each architecture ships its own archive, and the name has to say which is which.
architecture="$(uname -m)"
archive="$output_dir/${app_name}-${version}-macos-${architecture}.zip"
rm -f "$archive"
# ditto keeps the signature intact; zip does not.
ditto -c -k --keepParent "$bundle" "$archive"

printf '\nBundle:  %s (%s)\n' "$bundle" "$(du -sh "$bundle" | cut -f1)"
printf 'Archive: %s (%s, %s)\n' "$archive" "$architecture" "$(du -h "$archive" | cut -f1)"
if [[ -n "$sign_identity" ]]; then
    printf 'Signed:  %s\n' "$(codesign -dv "$bundle" 2>&1 | sed -n 's/^Identifier=//p')"
else
    printf 'Signed:  no identity given, the bundle is unsigned\n'
fi

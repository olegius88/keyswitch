#!/usr/bin/env bash
# Builds the Debian package of the Rust KeySwitch: the release binary, the resources it reads and the desktop files.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/Cargo.toml" | head -n 1)"
architecture="$(dpkg --print-architecture)"
target_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
apt_sources="$project_dir/packaging/debian/keyswitch.sources"
apt_sources_path=/etc/apt/sources.list.d/keyswitch.sources
apt_keyring="$project_dir/packaging/keyswitch-archive-keyring.asc"
apt_keyring_path="$(sed -nE 's/^Signed-By:[[:space:]]+([^[:space:]]+)$/\1/p' "$apt_sources" | head -n 1)"
gnome_uuid="keyswitch@io.github.olegius88.KeySwitch"
: "${SOURCE_DATE_EPOCH:=$(git -C "$project_dir" log -1 --format=%ct)}"

# No package without the evidence of its models, and never with a rejected one.
cargo run --release --quiet -p ks-train --bin ks-train --manifest-path "$project_dir/Cargo.toml" -- gates --tree "$project_dir" > /dev/null
cargo build --release -p ks-linux --manifest-path "$project_dir/Cargo.toml"

package_root="$(mktemp -d)"
trap 'rm -rf -- "$package_root"' EXIT
resources="$package_root/usr/lib/keyswitch/resources"
install -d "$package_root/DEBIAN" "$package_root$(dirname "$apt_sources_path")" "$package_root$(dirname "$apt_keyring_path")" \
    "$package_root/usr/bin" "$resources/models" "$resources/wayland-input" "$package_root/usr/share/applications" \
    "$package_root/usr/share/doc/keyswitch/licenses" "$package_root/usr/share/gnome-shell/extensions/$gnome_uuid" \
    "$package_root/usr/share/icons/hicolor/scalable/apps" "$package_root/usr/share/lintian/overrides" \
    "$package_root/usr/share/man/man1" "$output_dir"

sed -e "s/@VERSION@/$version/g" -e "s/@ARCHITECTURE@/$architecture/g" \
    "$project_dir/packaging/debian/control.in" > "$package_root/DEBIAN/control"
printf '%s\n' "$apt_sources_path" > "$package_root/DEBIAN/conffiles"
install -m 0644 "$apt_sources" "$package_root$apt_sources_path"
install -m 0644 "$apt_keyring" "$package_root$apt_keyring_path"

install -m 0755 "$target_dir/release/keyswitch" "$package_root/usr/bin/keyswitch"
for name in identifiers.json protected_tokens.txt lexicon-supplement-ru_RU.json typo-frequency-en_US.bin \
    typo-frequency-ru_RU.bin style.css; do
    install -m 0644 "$project_dir/resources/$name" "$resources/$name"
done
install -m 0644 "$project_dir"/resources/models/* "$resources/models/"
install -m 0644 "$project_dir"/resources/wayland-input/* "$resources/wayland-input/"
install -m 0644 "$project_dir"/model/intent_v1/sources/{en_US,ru_RU}.lm "$resources/models/"
install -m 0644 "$project_dir"/resources/keyswitch*.svg "$package_root/usr/share/icons/hicolor/scalable/apps/"
install -m 0644 "$project_dir/packaging/io.github.olegius88.KeySwitch.desktop" "$package_root/usr/share/applications/"
install -m 0644 "$project_dir/packaging/gnome-shell/$gnome_uuid/metadata.json" "$project_dir/packaging/gnome-shell/$gnome_uuid/extension.js" \
    "$package_root/usr/share/gnome-shell/extensions/$gnome_uuid/"
install -m 0644 "$project_dir/packaging/debian/copyright" "$project_dir/README.md" "$package_root/usr/share/doc/keyswitch/"
install -m 0644 "$project_dir/packaging/debian/lintian-overrides" "$package_root/usr/share/lintian/overrides/keyswitch"
sed "s/@VERSION@/$version/g" "$project_dir/packaging/debian/changelog.in" | gzip -n -9 > "$package_root/usr/share/doc/keyswitch/changelog.gz"
gzip -n -9 -c "$project_dir/packaging/keyswitch.1" > "$package_root/usr/share/man/man1/keyswitch.1.gz"

find "$package_root" -type d -exec chmod 0755 {} +
find "$package_root" -exec touch --date="@$SOURCE_DATE_EPOCH" {} +
package_path="$output_dir/keyswitch_${version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$package_root" "$package_path"
printf 'Built %s, %s bytes; program %s bytes\n' "$package_path" "$(stat -c %s "$package_path")" "$(stat -c %s "$target_dir/release/keyswitch")"

#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/pyproject.toml" | head -n 1)"
module_version="$(sed -nE 's/^__version__ = "([^"]+)"/\1/p' "$project_dir/src/keyswitch/__init__.py" | head -n 1)"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1787749200}"

if [[ -z "$version" || "$version" != "$module_version" ]]; then
    printf 'Version mismatch: pyproject=%s module=%s\n' "$version" "$module_version" >&2
    exit 1
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+~.-][0-9A-Za-z.+~-]+)?$ ]]; then
    printf 'Unsupported Debian package version: %s\n' "$version" >&2
    exit 1
fi

mkdir -p "$project_dir/build" "$output_dir"
stage_dir="$(mktemp -d "$project_dir/build/keyswitch-deb.XXXXXXXX")"
package_root="$stage_dir/keyswitch_${version}_all"
package_path="$output_dir/keyswitch_${version}_all.deb"

install -d \
    "$package_root/DEBIAN" \
    "$package_root/usr/bin" \
    "$package_root/usr/lib/keyswitch/keyswitch" \
    "$package_root/usr/share/applications" \
    "$package_root/usr/share/doc/keyswitch" \
    "$package_root/usr/share/icons/hicolor/scalable/apps" \
    "$package_root/usr/share/man/man1"

sed "s/@VERSION@/$version/g" \
    "$project_dir/packaging/debian/control.in" \
    > "$package_root/DEBIAN/control"
install -m 0644 "$project_dir/packaging/debian/copyright" \
    "$package_root/usr/share/doc/keyswitch/copyright"
install -m 0755 "$project_dir/packaging/keyswitch" \
    "$package_root/usr/bin/keyswitch"
install -m 0644 "$project_dir/packaging/io.github.olegius88.KeySwitch.desktop" \
    "$package_root/usr/share/applications/io.github.olegius88.KeySwitch.desktop"
install -m 0644 "$project_dir/src/keyswitch/resources/keyswitch.svg" \
    "$package_root/usr/share/icons/hicolor/scalable/apps/keyswitch.svg"
install -m 0644 "$project_dir/src/keyswitch/resources/keyswitch-paused.svg" \
    "$package_root/usr/share/icons/hicolor/scalable/apps/keyswitch-paused.svg"
install -m 0644 "$project_dir/README.md" \
    "$package_root/usr/share/doc/keyswitch/README.md"

while IFS= read -r -d '' source_file; do
    relative_path="${source_file#"$project_dir/src/keyswitch/"}"
    install -Dm0644 "$source_file" \
        "$package_root/usr/lib/keyswitch/keyswitch/$relative_path"
done < <(
    find "$project_dir/src/keyswitch" -type f \
        ! -path '*/__pycache__/*' \
        ! -name '*.pyc' \
        -print0
)

sed "s/@VERSION@/$version/g" "$project_dir/packaging/debian/changelog.in" \
    | gzip -n -9 \
    > "$package_root/usr/share/doc/keyswitch/changelog.gz"
gzip -n -9 -c "$project_dir/packaging/keyswitch.1" \
    > "$package_root/usr/share/man/man1/keyswitch.1.gz"

find "$package_root" -type d -exec chmod 0755 {} +
chmod 0644 \
    "$package_root/DEBIAN/control" \
    "$package_root/usr/share/doc/keyswitch/changelog.gz" \
    "$package_root/usr/share/man/man1/keyswitch.1.gz"
find "$package_root" -exec touch --date="@$SOURCE_DATE_EPOCH" {} +
dpkg-deb --root-owner-group --build "$package_root" "$package_path"

printf 'Built %s\n' "$package_path"

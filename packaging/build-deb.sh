#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/pyproject.toml" | head -n 1)"
module_version="$(sed -nE 's/^__version__ = "([^"]+)"/\1/p' "$project_dir/src/keyswitch/__init__.py" | head -n 1)"
architecture="${DEB_HOST_ARCH:-$(dpkg --print-architecture)}"
nuitka_root="${KEYSWITCH_NUITKA_ROOT:-$project_dir/.nuitka}"
nuitka_version="4.2"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1787749200}"

if [[ -z "$version" || "$version" != "$module_version" ]]; then
    printf 'Version mismatch: pyproject=%s module=%s\n' "$version" "$module_version" >&2
    exit 1
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+~.-][0-9A-Za-z.+~-]+)?$ ]]; then
    printf 'Unsupported Debian package version: %s\n' "$version" >&2
    exit 1
fi

for required_command in dpkg dpkg-deb gcc patch patchelf python3; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing native build command: %s\n' "$required_command" >&2
        exit 1
    fi
done

installed_nuitka_version="$({
    PYTHONPATH="$nuitka_root" python3 -c \
        'from nuitka.Version import getNuitkaVersion; print(getNuitkaVersion())'
} 2>/dev/null || true)"
if [[ "$installed_nuitka_version" != "$nuitka_version" ]]; then
    printf 'Nuitka %s is required in %s (found: %s).\n' \
        "$nuitka_version" "$nuitka_root" "${installed_nuitka_version:-none}" >&2
    printf 'Run: ./tools/install-build-tools.sh %q\n' "$nuitka_root" >&2
    exit 1
fi

mkdir -p "$project_dir/build" "$output_dir"
stage_dir="$(mktemp -d "$project_dir/build/keyswitch-deb.XXXXXXXX")"
package_root="$stage_dir/keyswitch_${version}_${architecture}"
package_path="$output_dir/keyswitch_${version}_${architecture}.deb"
native_output="$stage_dir/native"
native_dist="$native_output/keyswitch_entry.dist"
pygobject_site="$stage_dir/pygobject-site"

gi_source="$(python3 -c \
    'from pathlib import Path; import gi; print(Path(gi.__file__).resolve().parent)')"
install -d "$native_output" "$pygobject_site"
cp -a "$gi_source" "$pygobject_site/gi"

# PyGObject 3.56 creates a compatibility alias through globals(). Nuitka cannot
# expose that dynamic name while freezing the GLib override, so stage a narrow
# source patch when (and only when) the vulnerable upstream block is present.
if patch --batch --forward --dry-run --strip=1 --directory="$pygobject_site" \
    < "$project_dir/packaging/pygobject-nuitka.patch" >/dev/null 2>&1; then
    patch --batch --forward --strip=1 --directory="$pygobject_site" \
        < "$project_dir/packaging/pygobject-nuitka.patch" >/dev/null
fi

PYTHONPATH="$pygobject_site:$nuitka_root:$project_dir/src" \
python3 -m nuitka \
    --mode=standalone \
    --lto=no \
    --output-dir="$native_output" \
    --output-filename=keyswitch-bin \
    --include-package-data=keyswitch \
    --nofollow-import-to='keyswitch.windows_*' \
    --nofollow-import-to=tests \
    --noinclude-dlls='libbz2.so*' \
    --noinclude-dlls='libcrypto.so*' \
    --noinclude-dlls='libexpat.so*' \
    --noinclude-dlls='libffi.so*' \
    --noinclude-dlls='liblzma.so*' \
    --noinclude-dlls='libssl.so*' \
    --noinclude-dlls='libzstd.so*' \
    --no-progressbar \
    --report="$native_output/compilation-report.xml" \
    "$project_dir/packaging/keyswitch_entry.py"

if [[ ! -x "$native_dist/keyswitch-bin" ]]; then
    printf 'Nuitka did not produce the expected native executable.\n' >&2
    exit 1
fi
if find "$native_dist" -type f \
    \( -name '*.py' -o -name '*.pyc' -o -name '*.pyo' \) \
    -print -quit | grep -q .; then
    printf 'Native distribution unexpectedly contains Python source or bytecode.\n' >&2
    exit 1
fi
if [[ "$("$native_dist/keyswitch-bin" --version)" != "KeySwitch $version" ]]; then
    printf 'Native executable version smoke test failed.\n' >&2
    exit 1
fi

install -d \
    "$package_root/DEBIAN" \
    "$package_root/usr/bin" \
    "$package_root/usr/lib/keyswitch" \
    "$package_root/usr/share/applications" \
    "$package_root/usr/share/doc/keyswitch" \
    "$package_root/usr/share/doc/keyswitch/licenses" \
    "$package_root/usr/share/icons/hicolor/scalable/apps" \
    "$package_root/usr/share/lintian/overrides" \
    "$package_root/usr/share/man/man1"

sed \
    -e "s/@VERSION@/$version/g" \
    -e "s/@ARCHITECTURE@/$architecture/g" \
    "$project_dir/packaging/debian/control.in" \
    > "$package_root/DEBIAN/control"
install -m 0644 "$project_dir/packaging/debian/copyright" \
    "$package_root/usr/share/doc/keyswitch/copyright"
install -m 0644 "$project_dir/packaging/debian/lintian-overrides" \
    "$package_root/usr/share/lintian/overrides/keyswitch"
install -m 0755 "$project_dir/packaging/keyswitch" \
    "$package_root/usr/bin/keyswitch"
cp -a "$native_dist/." "$package_root/usr/lib/keyswitch/"
find "$package_root/usr/lib/keyswitch" -type f -exec chmod 0644 {} +
chmod 0755 "$package_root/usr/lib/keyswitch/keyswitch-bin"
install -m 0644 "$project_dir/packaging/io.github.olegius88.KeySwitch.desktop" \
    "$package_root/usr/share/applications/io.github.olegius88.KeySwitch.desktop"
for icon in "$project_dir"/src/keyswitch/resources/keyswitch*.svg; do
    install -m 0644 "$icon" \
        "$package_root/usr/share/icons/hicolor/scalable/apps/$(basename "$icon")"
done
install -m 0644 "$project_dir/README.md" \
    "$package_root/usr/share/doc/keyswitch/README.md"

install_license() {
    local destination="$1"
    shift
    local candidate
    for candidate in "$@"; do
        if [[ -f "$candidate" ]]; then
            install -m 0644 "$candidate" \
                "$package_root/usr/share/doc/keyswitch/licenses/$destination"
            return
        fi
    done
}

install_license copyright.cpython /usr/share/doc/python3/copyright
install_license copyright.pygobject /usr/share/doc/python3-gi/copyright
install_license copyright.dbus-python /usr/share/doc/python3-dbus/copyright
install_license copyright.bzip2 /usr/share/doc/libbz2-1.0/copyright
install_license copyright.expat /usr/share/doc/libexpat1/copyright
install_license copyright.libffi /usr/share/doc/libffi8/copyright
install_license copyright.liblzma /usr/share/doc/liblzma5/copyright
install_license copyright.openssl \
    /usr/share/doc/libssl3t64/copyright \
    /usr/share/doc/libssl3/copyright
install_license copyright.zstd /usr/share/doc/libzstd1/copyright

nuitka_license="$(find "$nuitka_root" \
    -path '*/licenses/LICENSE.txt' -print -quit)"
nuitka_runtime_license="$(find "$nuitka_root" \
    -path '*/licenses/LICENSE-RUNTIME.txt' -print -quit)"
install_license copyright.nuitka "$nuitka_license"
install_license copyright.nuitka-runtime "$nuitka_runtime_license"

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

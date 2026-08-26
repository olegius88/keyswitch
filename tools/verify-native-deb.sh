#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package="${1:-}"

if [[ -z "$package" || ! -f "$package" ]]; then
    printf 'Usage: %s /path/to/keyswitch_VERSION_ARCH.deb\n' "$0" >&2
    exit 2
fi

for required_command in dpkg dpkg-deb file ldd; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing package verification command: %s\n' "$required_command" >&2
        exit 1
    fi
done

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' \
    "$project_dir/pyproject.toml" | head -n 1)"
architecture="${DEB_HOST_ARCH:-$(dpkg --print-architecture)}"
staging="$(mktemp -d "$project_dir/build/keyswitch-native-verify.XXXXXXXX")"

cleanup() {
    if [[ -d "$staging" && "$staging" == "$project_dir/build/keyswitch-native-verify."* ]]; then
        find "$staging" -depth -delete
    fi
}
trap cleanup EXIT

test "$(dpkg-deb --field "$package" Package)" = "keyswitch"
test "$(dpkg-deb --field "$package" Version)" = "$version"
test "$(dpkg-deb --field "$package" Architecture)" = "$architecture"

depends="$(dpkg-deb --field "$package" Depends)"
if grep -Eq '(^|[,[:space:]])python3([[:space:],:]|$)' <<<"$depends"; then
    printf 'Native package unexpectedly depends on the Python interpreter.\n' >&2
    exit 1
fi

dpkg-deb --extract "$package" "$staging"
binary="$staging/usr/lib/keyswitch/keyswitch-bin"
launcher="$staging/usr/bin/keyswitch"

test -x "$binary"
test -x "$launcher"
grep -q '/usr/lib/keyswitch/keyswitch-bin' "$launcher"
if ! file "$binary" | grep -Eq 'ELF .* (executable|shared object)'; then
    printf 'Packaged application is not an ELF executable: %s\n' \
        "$(file "$binary")" >&2
    exit 1
fi

unexpected_python="$(find "$staging/usr/lib/keyswitch" -type f \
    \( -name '*.py' -o -name '*.pyc' -o -name '*.pyo' \) -print -quit)"
if [[ -n "$unexpected_python" ]]; then
    printf 'Packaged application contains Python source or bytecode: %s\n' \
        "$unexpected_python" >&2
    exit 1
fi

test "$("$binary" --version)" = "KeySwitch $version"
elf_count=0
while IFS= read -r -d '' candidate; do
    if ! file -b "$candidate" | grep -q '^ELF '; then
        continue
    fi
    elf_count=$((elf_count + 1))
    linked_libraries="$(ldd "$candidate")"
    if grep -q 'not found' <<<"$linked_libraries"; then
        printf 'Packaged ELF has unresolved shared libraries: %s\n%s\n' \
            "$candidate" "$linked_libraries" >&2
        exit 1
    fi
done < <(find "$staging/usr/lib/keyswitch" -type f -print0)
test "$elf_count" -gt 0

printf 'NATIVE_DEB_OK package=%s architecture=%s elf_files=%d binary=%s\n' \
    "$package" "$architecture" "$elf_count" "$(file -b "$binary")"

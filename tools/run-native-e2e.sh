#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package="${1:-}"

if [[ -z "$package" || ! -f "$package" ]]; then
    printf 'Usage: %s /path/to/keyswitch_VERSION_ARCH.deb\n' "$0" >&2
    exit 2
fi
if [[ -z "${DISPLAY:-}" ]]; then
    printf 'Native E2E requires an active X11 DISPLAY.\n' >&2
    exit 1
fi

staging="$(mktemp -d "$project_dir/build/keyswitch-native-e2e.XXXXXXXX")"
cleanup() {
    if [[ -d "$staging" && "$staging" == "$project_dir/build/keyswitch-native-e2e."* ]]; then
        find "$staging" -depth -delete
    fi
}
trap cleanup EXIT

dpkg-deb --extract "$package" "$staging"
binary="$staging/usr/lib/keyswitch/keyswitch-bin"
test -x "$binary"

GTK_A11Y=none PYTHONPATH="$project_dir/src" \
    python3 "$project_dir/tests/e2e_native_package.py" "$binary"

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

for required_command in dpkg-deb mktemp realpath rm stat; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing native E2E command: %s\n' "$required_command" >&2
        exit 1
    fi
done

staging="$(mktemp -d /tmp/keyswitch-native-e2e.XXXXXXXX)"
staging_identity="$(stat --format='%d:%i:%u' -- "$staging")"
readonly staging staging_identity
cleanup() {
    local original_status="${1:-0}"
    local cleanup_status=0
    local resolved=""
    local current_identity=""

    trap - EXIT
    if [[ -z "$staging" ]] \
        || [[ ! "$staging" =~ ^/tmp/keyswitch-native-e2e\.[A-Za-z0-9]{8}$ ]]; then
        printf 'Refusing to clean an invalid native-E2E path: %q\n' \
            "$staging" >&2
        cleanup_status=1
    elif [[ -L "$staging" ]]; then
        printf 'Refusing to clean a symlinked native-E2E path: %s\n' \
            "$staging" >&2
        cleanup_status=1
    elif [[ -e "$staging" ]]; then
        if [[ ! -d "$staging" ]]; then
            printf 'Refusing to clean a non-directory native-E2E path: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif ! resolved="$(realpath --canonicalize-existing -- "$staging")"; then
            printf 'Cannot resolve native-E2E path before cleanup: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif [[ "$resolved" != "$staging" || "$resolved" != /tmp/* ]]; then
            printf 'Refusing to clean a redirected native-E2E path: %s -> %s\n' \
                "$staging" "$resolved" >&2
            cleanup_status=1
        elif ! current_identity="$(stat --format='%d:%i:%u' -- "$staging")"; then
            printf 'Cannot identify native-E2E path before cleanup: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif [[ "$current_identity" != "$staging_identity" ]]; then
            printf 'Refusing to clean a replaced native-E2E path: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif ! rm -rf -- "$staging"; then
            printf 'Failed to clean native-E2E path: %s\n' "$staging" >&2
            cleanup_status=1
        fi
    fi

    if ((original_status != 0)); then
        return "$original_status"
    fi
    return "$cleanup_status"
}
trap 'cleanup "$?"' EXIT

dpkg-deb --extract "$package" "$staging"
binary="$staging/usr/lib/keyswitch/keyswitch-bin"
test -x "$binary"

GTK_A11Y=none PYTHONPATH="$project_dir/src" \
    python3 "$project_dir/tests/e2e_native_package.py" "$binary"

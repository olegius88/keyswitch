#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package="${1:-}"
expected_intent_model="$project_dir/src/keyswitch/resources/models/layout_intent_v1.ksm"
intent_model_max_bytes=$((14 * 1024 * 1024))
intent_manifest_max_bytes=$((1024 * 1024))
intent_payload_max_bytes=$((12 * 1024 * 1024))
intent_fingerprint_max_count=$((1 << 20))
frozen_model_sources="$project_dir/model/intent_v1/sources"
expected_english_model="$frozen_model_sources/en_US.lm"
expected_russian_model="$frozen_model_sources/ru_RU.lm"
expected_onboard_copyright="$frozen_model_sources/COPYRIGHT.onboard-data"

verify_kslm_packaging_bounds() {
    local model_path="$1"
    python3 -c '
import json
from pathlib import Path
import struct
import sys

path = Path(sys.argv[1])
maximum_container = int(sys.argv[2])
maximum_manifest = int(sys.argv[3])
maximum_payload = int(sys.argv[4])
maximum_fingerprints = int(sys.argv[5])
header = struct.Struct("<4sHHIII32s")
with path.open("rb") as stream:
    data = stream.read(maximum_container + 1)
if len(data) > maximum_container:
    raise SystemExit("KSLM container exceeds the 14 MiB packaging limit")
if len(data) < header.size:
    raise SystemExit("KSLM header is truncated")
magic, schema, flags, manifest_length, payload_length, _crc, _digest = (
    header.unpack_from(data)
)
if magic != b"KSLM":
    raise SystemExit("KSLM magic is invalid")
if schema != 4:
    raise SystemExit("KSLM schema is unsupported")
if flags != 0:
    raise SystemExit("KSLM header flags are unsupported")
if not 2 <= manifest_length <= maximum_manifest:
    raise SystemExit("KSLM embedded manifest exceeds the 1 MiB packaging limit")
if not 0 < payload_length <= maximum_payload:
    raise SystemExit("KSLM payload exceeds the 12 MiB packaging limit")
if len(data) != header.size + manifest_length + payload_length:
    raise SystemExit("KSLM header lengths do not match the complete container")
try:
    manifest = json.loads(
        data[header.size : header.size + manifest_length].decode("utf-8")
    )
except (UnicodeDecodeError, json.JSONDecodeError) as error:
    raise SystemExit(f"KSLM embedded manifest is invalid: {error}") from error
if not isinstance(manifest, dict):
    raise SystemExit("KSLM embedded manifest must be an object")
fingerprints = manifest.get("supported_fingerprint_count")
dimension = manifest.get("dimension")
if (
    isinstance(fingerprints, bool)
    or not isinstance(fingerprints, int)
    or not 0 <= fingerprints <= maximum_fingerprints
):
    raise SystemExit("KSLM fingerprint count exceeds the 2^20 packaging limit")
if isinstance(dimension, bool) or not isinstance(dimension, int) or dimension <= 0:
    raise SystemExit("KSLM dimension is invalid")
if payload_length != (dimension * 2) + (fingerprints * 8):
    raise SystemExit("KSLM payload shape does not match its embedded manifest")
' "$model_path" \
        "$intent_model_max_bytes" \
        "$intent_manifest_max_bytes" \
        "$intent_payload_max_bytes" \
        "$intent_fingerprint_max_count"
}

if [[ -z "$package" || ! -f "$package" ]]; then
    printf 'Usage: %s /path/to/keyswitch_VERSION_ARCH.deb\n' "$0" >&2
    exit 2
fi

for required_command in cmp dpkg dpkg-deb file ldd mktemp python3 realpath rm sha256sum stat timeout; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing package verification command: %s\n' "$required_command" >&2
        exit 1
    fi
done
if ! (cd "$frozen_model_sources" && sha256sum --check --status SHA256SUMS); then
    printf 'Frozen Onboard model sources do not match SHA256SUMS.\n' >&2
    exit 1
fi

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' \
    "$project_dir/pyproject.toml" | head -n 1)"
architecture="${DEB_HOST_ARCH:-$(dpkg --print-architecture)}"
staging="$(mktemp -d /tmp/keyswitch-native-verify.XXXXXXXX)"
staging_identity="$(stat --format='%d:%i:%u' -- "$staging")"
readonly staging staging_identity

cleanup() {
    local original_status="${1:-0}"
    local cleanup_status=0
    local resolved=""
    local current_identity=""

    trap - EXIT
    if [[ -z "$staging" ]] \
        || [[ ! "$staging" =~ ^/tmp/keyswitch-native-verify\.[A-Za-z0-9]{8}$ ]]; then
        printf 'Refusing to clean an invalid package-verification path: %q\n' \
            "$staging" >&2
        cleanup_status=1
    elif [[ -L "$staging" ]]; then
        printf 'Refusing to clean a symlinked package-verification path: %s\n' \
            "$staging" >&2
        cleanup_status=1
    elif [[ -e "$staging" ]]; then
        if [[ ! -d "$staging" ]]; then
            printf 'Refusing to clean a non-directory package-verification path: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif ! resolved="$(realpath --canonicalize-existing -- "$staging")"; then
            printf 'Cannot resolve package-verification path before cleanup: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif [[ "$resolved" != "$staging" || "$resolved" != /tmp/* ]]; then
            printf 'Refusing to clean a redirected package-verification path: %s -> %s\n' \
                "$staging" "$resolved" >&2
            cleanup_status=1
        elif ! current_identity="$(stat --format='%d:%i:%u' -- "$staging")"; then
            printf 'Cannot identify package-verification path before cleanup: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif [[ "$current_identity" != "$staging_identity" ]]; then
            printf 'Refusing to clean a replaced package-verification path: %s\n' \
                "$staging" >&2
            cleanup_status=1
        elif ! rm -rf -- "$staging"; then
            printf 'Failed to clean package-verification path: %s\n' \
                "$staging" >&2
            cleanup_status=1
        fi
    fi

    if ((original_status != 0)); then
        return "$original_status"
    fi
    return "$cleanup_status"
}
trap 'cleanup "$?"' EXIT

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
intent_model="$staging/usr/lib/keyswitch/keyswitch/resources/models/layout_intent_v1.ksm"
english_model="$staging/usr/lib/keyswitch/keyswitch/resources/models/en_US.lm"
russian_model="$staging/usr/lib/keyswitch/keyswitch/resources/models/ru_RU.lm"
onboard_copyright="$staging/usr/share/doc/keyswitch/licenses/copyright.onboard-data"

test -x "$binary"
test -x "$launcher"
test -s "$intent_model"
test "$(head -c 4 "$intent_model")" = "KSLM"
verify_kslm_packaging_bounds "$intent_model"
cmp -s "$expected_intent_model" "$intent_model"
test -s "$english_model"
test -s "$russian_model"
cmp -s "$expected_english_model" "$english_model"
cmp -s "$expected_russian_model" "$russian_model"
test -s "$onboard_copyright"
test -s "$expected_onboard_copyright"
cmp -s "$expected_onboard_copyright" "$onboard_copyright"
grep -Fqx 'Files: models/*' "$onboard_copyright"
grep -Fqx 'Copyright: 2013, 2014, marmuta <marmvta@gmail.com>' "$onboard_copyright"
grep -Fqx '  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>' \
    "$onboard_copyright"
grep -Fqx 'License: GPL-3+' "$onboard_copyright"
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

diagnostic_json="$staging/packaged-diagnose.json"
diagnostic_stderr="$staging/packaged-diagnose.stderr"
diagnostic_status=0
KEYSWITCH_DATA_DIR="$staging/packaged-diagnostic-data" \
KEYSWITCH_INTENT_MODEL_PATH= \
timeout 30s "$binary" --diagnose \
    >"$diagnostic_json" 2>"$diagnostic_stderr" \
    || diagnostic_status=$?
if ((diagnostic_status != 0 && diagnostic_status != 1)); then
    printf 'Packaged diagnostic failed with unexpected status %d.\n' \
        "$diagnostic_status" >&2
    sed -n '1,80p' "$diagnostic_stderr" >&2
    exit 1
fi
if ! python3 -c '
import hashlib
import json
from pathlib import Path
import struct
import sys

payload_path = Path(sys.argv[1])
expected_path = Path(sys.argv[2]).resolve(strict=True)
maximum_model_bytes = int(sys.argv[3])
maximum_json_bytes = int(sys.argv[4])


def bounded_read(path: Path, maximum_bytes: int, label: str) -> bytes:
    with path.open("rb") as stream:
        result = stream.read(maximum_bytes + 1)
    if len(result) > maximum_bytes:
        raise ValueError(f"{label} exceeds {maximum_bytes} bytes")
    return result


payload = json.loads(
    bounded_read(payload_path, maximum_json_bytes, "packaged diagnostics").decode(
        "utf-8"
    )
)
expected_bytes = bounded_read(expected_path, maximum_model_bytes, "KSLM model")
header = struct.Struct("<4sHHIII32s")
if len(expected_bytes) < header.size:
    raise ValueError("KSLM header is truncated")
magic, _schema, _flags, manifest_length, payload_length, _crc, _digest = (
    header.unpack_from(expected_bytes)
)
if (
    magic != b"KSLM"
    or not 2 <= manifest_length <= maximum_json_bytes
    or len(expected_bytes) != header.size + manifest_length + payload_length
):
    raise ValueError("KSLM diagnostic snapshot has invalid bounds")
embedded = json.loads(
    expected_bytes[header.size : header.size + manifest_length].decode("utf-8")
)
expected_version = embedded.get("model_version") if isinstance(embedded, dict) else None
intent = payload.get("intent_model")
valid = (
    isinstance(intent, dict)
    and intent.get("available") is True
    and isinstance(intent.get("path"), str)
    and Path(intent["path"]).resolve(strict=True) == expected_path
    and intent.get("checksum")
    == hashlib.sha256(expected_bytes).hexdigest()
    and isinstance(expected_version, str)
    and intent.get("version") == expected_version
)
raise SystemExit(0 if valid else 1)
' "$diagnostic_json" "$intent_model" \
        "$intent_model_max_bytes" "$intent_manifest_max_bytes"; then
    printf 'Packaged diagnostic did not load the exact bundled intent model.\n' >&2
    sed -n '1,120p' "$diagnostic_json" >&2
    sed -n '1,80p' "$diagnostic_stderr" >&2
    exit 1
fi

printf 'NATIVE_DEB_OK package=%s architecture=%s elf_files=%d intent_model=%s language_models=en_US.lm,ru_RU.lm binary=%s\n' \
    "$package" "$architecture" "$elf_count" \
    "$(basename "$intent_model")" "$(file -b "$binary")"

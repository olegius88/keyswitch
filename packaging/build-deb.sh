#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/pyproject.toml" | head -n 1)"
module_version="$(sed -nE 's/^__version__ = "([^"]+)"/\1/p' "$project_dir/src/keyswitch/__init__.py" | head -n 1)"
architecture="${DEB_HOST_ARCH:-$(dpkg --print-architecture)}"
nuitka_root="${KEYSWITCH_NUITKA_ROOT:-$project_dir/.nuitka}"
nuitka_version="4.2"
intent_model="$project_dir/src/keyswitch/resources/models/layout_intent_v1.ksm"
intent_manifest="$project_dir/model/intent_v1/manifest.json"
intent_model_max_bytes=$((14 * 1024 * 1024))
intent_manifest_max_bytes=$((1024 * 1024))
intent_payload_max_bytes=$((12 * 1024 * 1024))
intent_fingerprint_max_count=$((1 << 20))
frozen_model_sources="$project_dir/model/intent_v1/sources"
frozen_english_model="$frozen_model_sources/en_US.lm"
frozen_russian_model="$frozen_model_sources/ru_RU.lm"
onboard_copyright="$frozen_model_sources/COPYRIGHT.onboard-data"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1787749200}"

PYTHONPATH="$project_dir/src" python3 "$project_dir/tools/verify_context_model.py"

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

if [[ -z "$version" || "$version" != "$module_version" ]]; then
    printf 'Version mismatch: pyproject=%s module=%s\n' "$version" "$module_version" >&2
    exit 1
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+~.-][0-9A-Za-z.+~-]+)?$ ]]; then
    printf 'Unsupported Debian package version: %s\n' "$version" >&2
    exit 1
fi

if [[ ! -s "$intent_model" || "$(head -c 4 "$intent_model")" != "KSLM" ]]; then
    printf 'Required bundled intent model is missing or invalid: %s\n' \
        "$intent_model" >&2
    exit 1
fi
if [[ ! -s "$intent_manifest" ]]; then
    printf 'Required intent-model commit manifest is missing: %s\n' \
        "$intent_manifest" >&2
    exit 1
fi
if [[ ! -s "$onboard_copyright" ]]; then
    printf 'Required Onboard model copyright is missing: %s\n' \
        "$onboard_copyright" >&2
    exit 1
fi
for attribution_line in \
    'Files: models/*' \
    'Copyright: 2013, 2014, marmuta <marmvta@gmail.com>' \
    '  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>' \
    'License: GPL-3+'; do
    if ! grep -Fqx "$attribution_line" "$onboard_copyright"; then
        printf 'Onboard model copyright is missing the required attribution: %s\n' \
            "$attribution_line" >&2
        exit 1
    fi
done

for required_command in cmp dpkg dpkg-deb gcc mktemp patch patchelf python3 realpath rm sha256sum stat timeout; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing native build command: %s\n' "$required_command" >&2
        exit 1
    fi
done
if ! (cd "$frozen_model_sources" && sha256sum --check --status SHA256SUMS); then
    printf 'Frozen Onboard model sources do not match SHA256SUMS.\n' >&2
    exit 1
fi
if ! verify_kslm_packaging_bounds "$intent_model"; then
    printf 'Bundled intent model violates the fail-closed KSLM packaging bounds.\n' >&2
    exit 1
fi
mkdir -p -- "$project_dir/build" "$output_dir"
intent_quality_report="$output_dir/keyswitch-intent-evaluation.json"
# A strict report produced earlier in the same release contour may stand in
# for the half-hour evaluation, but only after tools/verify_intent_strict_report.py
# proves that every gate passed and every hash it recorded (artifact, config,
# frozen sources and the complete model toolchain) still equals the current
# file. Anything else fails the build instead of silently re-running.
reusable_strict_report="${KEYSWITCH_INTENT_STRICT_REPORT:-}"
if [[ -n "$reusable_strict_report" ]]; then
    if [[ ! -f "$reusable_strict_report" ]]; then
        printf 'Reusable strict report does not exist: %s\n' \
            "$reusable_strict_report" >&2
        exit 1
    fi
    if ! python3 "$project_dir/tools/verify_intent_strict_report.py" \
        --report "$reusable_strict_report" \
        --artifact "$intent_model" \
        --manifest "$intent_manifest" \
        --config "$project_dir/model/intent_v1/config.json" \
        --project-root "$project_dir" >/dev/null; then
        printf 'Reusable strict report is not bound to the current tree: %s\n' \
            "$reusable_strict_report" >&2
        exit 1
    fi
    if [[ "$(realpath -- "$reusable_strict_report")" \
        != "$(realpath -m -- "$intent_quality_report")" ]]; then
        install -m 0644 -- "$reusable_strict_report" "$intent_quality_report"
    fi
    printf 'Reusing verified strict intent-model report: %s\n' \
        "$reusable_strict_report"
elif ! PYTHONPATH="$project_dir/src" python3 \
    "$project_dir/tools/evaluate_intent_model.py" \
    --config "$project_dir/model/intent_v1/config.json" \
    --en-model "$project_dir/model/intent_v1/sources/en_US.lm" \
    --ru-model "$project_dir/model/intent_v1/sources/ru_RU.lm" \
    --artifact "$intent_model" \
    --manifest "$intent_manifest" \
    --strict >"$intent_quality_report"; then
    printf 'Bundled intent model failed strict provenance or quality gates.\n' >&2
    python3 - "$intent_quality_report" <<'PY'
import json
from pathlib import Path
import sys

report_path = Path(sys.argv[1])
try:
    if report_path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("report exceeds the 8 MiB diagnostic bound")
    report = json.loads(report_path.read_bytes())
except (OSError, ValueError, json.JSONDecodeError) as error:
    print(f"Strict evaluator report is unavailable: {error}", file=sys.stderr)
else:
    gates = report.get("strict_gates")
    if isinstance(gates, dict):
        failed = sorted(name for name, value in gates.items() if value is not True)
        print(
            "Failed strict intent-model gates: "
            + (", ".join(failed) if failed else "invalid strict result"),
            file=sys.stderr,
        )
    else:
        phase = report.get("phase", "unknown")
        print(f"Strict intent-model evaluation stopped in phase: {phase}", file=sys.stderr)
PY
    exit 1
fi

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

stage_dir="$(mktemp -d /tmp/keyswitch-deb.XXXXXXXX)"
stage_identity="$(stat --format='%d:%i:%u' -- "$stage_dir")"
readonly stage_dir stage_identity

cleanup_stage_dir() {
    local original_status="${1:-0}"
    local cleanup_status=0
    local resolved=""
    local current_identity=""

    trap - EXIT
    if [[ -z "$stage_dir" ]] \
        || [[ ! "$stage_dir" =~ ^/tmp/keyswitch-deb\.[A-Za-z0-9]{8}$ ]]; then
        printf 'Refusing to clean an invalid Debian staging path: %q\n' \
            "$stage_dir" >&2
        cleanup_status=1
    elif [[ -L "$stage_dir" ]]; then
        printf 'Refusing to clean a symlinked Debian staging path: %s\n' \
            "$stage_dir" >&2
        cleanup_status=1
    elif [[ -e "$stage_dir" ]]; then
        if [[ ! -d "$stage_dir" ]]; then
            printf 'Refusing to clean a non-directory Debian staging path: %s\n' \
                "$stage_dir" >&2
            cleanup_status=1
        elif ! resolved="$(realpath --canonicalize-existing -- "$stage_dir")"; then
            printf 'Cannot resolve Debian staging path before cleanup: %s\n' \
                "$stage_dir" >&2
            cleanup_status=1
        elif [[ "$resolved" != "$stage_dir" || "$resolved" != /tmp/* ]]; then
            printf 'Refusing to clean a redirected Debian staging path: %s -> %s\n' \
                "$stage_dir" "$resolved" >&2
            cleanup_status=1
        elif ! current_identity="$(stat --format='%d:%i:%u' -- "$stage_dir")"; then
            printf 'Cannot identify Debian staging path before cleanup: %s\n' \
                "$stage_dir" >&2
            cleanup_status=1
        elif [[ "$current_identity" != "$stage_identity" ]]; then
            printf 'Refusing to clean a replaced Debian staging path: %s\n' \
                "$stage_dir" >&2
            cleanup_status=1
        elif ! rm -rf -- "$stage_dir"; then
            printf 'Failed to clean Debian staging path: %s\n' "$stage_dir" >&2
            cleanup_status=1
        fi
    fi

    if ((original_status != 0)); then
        return "$original_status"
    fi
    return "$cleanup_status"
}
trap 'cleanup_stage_dir "$?"' EXIT

package_root="$stage_dir/keyswitch_${version}_${architecture}"
package_path="$output_dir/keyswitch_${version}_${architecture}.deb"
native_output="$stage_dir/native"
native_dist="$native_output/keyswitch_entry.dist"
pygobject_site="$stage_dir/pygobject-site"

verify_native_intent_model() {
    local binary="$1"
    local expected_model="$2"
    local diagnostic_json="$3"
    local diagnostic_stderr="$4"
    local diagnostic_data="${diagnostic_json}.data"
    local diagnostic_status=0

    KEYSWITCH_DATA_DIR="$diagnostic_data" \
    KEYSWITCH_INTENT_MODEL_PATH= \
    timeout 30s "$binary" --diagnose \
        >"$diagnostic_json" 2>"$diagnostic_stderr" \
        || diagnostic_status=$?
    if ((diagnostic_status != 0 && diagnostic_status != 1)); then
        printf 'Native diagnostic failed with unexpected status %d.\n' \
            "$diagnostic_status" >&2
        sed -n '1,80p' "$diagnostic_stderr" >&2
        return 1
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
    bounded_read(payload_path, maximum_json_bytes, "native diagnostics").decode(
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
' "$diagnostic_json" "$expected_model" \
        "$intent_model_max_bytes" "$intent_manifest_max_bytes"; then
        printf 'Native diagnostic did not load the exact bundled intent model.\n' >&2
        sed -n '1,120p' "$diagnostic_json" >&2
        sed -n '1,80p' "$diagnostic_stderr" >&2
        return 1
    fi
}

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

# Package data includes KSLM; explicit inputs add the frozen EN/RU scorers.
# Post-build byte comparisons verify all three model files.
PYTHONPATH="$pygobject_site:$nuitka_root:$project_dir/src" \
python3 -m nuitka \
    --mode=standalone \
    --lto=no \
    --output-dir="$native_output" \
    --output-filename=keyswitch-bin \
    --include-package-data=keyswitch \
    --include-data-files="$frozen_english_model=keyswitch/resources/models/en_US.lm" \
    --include-data-files="$frozen_russian_model=keyswitch/resources/models/ru_RU.lm" \
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
bundled_intent_model="$native_dist/keyswitch/resources/models/layout_intent_v1.ksm"
if ! cmp -s "$project_dir/src/keyswitch/resources/models/context_policy_v1.json" \
    "$native_dist/keyswitch/resources/models/context_policy_v1.json"; then
    printf 'Native distribution does not contain the exact contextual model.\n' >&2
    exit 1
fi
bundled_english_model="$native_dist/keyswitch/resources/models/en_US.lm"
bundled_russian_model="$native_dist/keyswitch/resources/models/ru_RU.lm"
if [[ ! -s "$bundled_intent_model" ]] \
    || [[ "$(head -c 4 "$bundled_intent_model")" != "KSLM" ]] \
    || ! cmp -s "$intent_model" "$bundled_intent_model"; then
    printf 'Native distribution does not contain the exact bundled intent model.\n' >&2
    exit 1
fi
if [[ ! -s "$bundled_english_model" ]] \
    || ! cmp -s "$frozen_english_model" "$bundled_english_model" \
    || [[ ! -s "$bundled_russian_model" ]] \
    || ! cmp -s "$frozen_russian_model" "$bundled_russian_model"; then
    printf 'Native distribution does not contain the exact frozen EN/RU language models.\n' >&2
    exit 1
fi
if ! verify_kslm_packaging_bounds "$bundled_intent_model"; then
    printf 'Native distribution contains an out-of-bounds KSLM container.\n' >&2
    exit 1
fi
verify_native_intent_model \
    "$native_dist/keyswitch-bin" \
    "$bundled_intent_model" \
    "$stage_dir/native-diagnose.json" \
    "$stage_dir/native-diagnose.stderr"

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
install_license copyright.onboard-data "$onboard_copyright"

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

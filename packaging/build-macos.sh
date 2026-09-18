#!/usr/bin/env bash
# Build the macOS application bundle, signed when an identity is available.
#
# The bundle is built the same way the Debian package and the Windows installer
# are: Nuitka compiles the same sources, the same model files are carried inside,
# and the same verification tools run first. What differs is the container and
# the signature.
#
# Usage: packaging/build-macos.sh [output-directory]
#
# KEYSWITCH_SIGN_IDENTITY names the Developer ID to sign with. Without it the
# bundle is built unsigned, which is enough to run it here but not to hand it to
# anyone: an unsigned bundle loses its Accessibility permission on every update.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$project_dir/dist}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/pyproject.toml" | head -n 1)"
module_version="$(sed -nE 's/^__version__ = "([^"]+)"/\1/p' "$project_dir/src/keyswitch/__init__.py" | head -n 1)"
# Nuitka may live in its own directory, as it does on a development machine, or
# be installed into the interpreter, as it is on a build runner.
nuitka_root="${KEYSWITCH_NUITKA_ROOT:-$HOME/.nuitka}"
nuitka_version="4.2"
python_bin="${KEYSWITCH_PYTHON:-python3}"
frozen_model_sources="$project_dir/model/intent_v1/sources"
frozen_english_model="$frozen_model_sources/en_US.lm"
frozen_russian_model="$frozen_model_sources/ru_RU.lm"
sign_identity="${KEYSWITCH_SIGN_IDENTITY:-}"
sign_keychain="${KEYSWITCH_SIGN_KEYCHAIN:-}"
bundle_identifier="io.github.olegius88.KeySwitch"
entitlements="$project_dir/packaging/keyswitch.entitlements"
app_name="KeySwitch"
native_output="$output_dir/macos-native"
bundle="$native_output/$app_name.app"
# The executable may not be called "keyswitch": macOS ignores letter case, and
# the bundle also carries a "keyswitch" directory of package data beside it.
binary_name="keyswitch-bin"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1787749200}"

if [[ "$(uname -s)" != "Darwin" ]]; then
    printf 'This build runs on macOS only.\n' >&2
    exit 1
fi
if [[ "$version" != "$module_version" ]]; then
    printf 'Version mismatch: pyproject %s, module %s.\n' "$version" "$module_version" >&2
    exit 1
fi

nuitka_path=""
[[ -d "$nuitka_root" ]] && nuitka_path="$nuitka_root"
installed_nuitka_version="$({
    PYTHONPATH="$nuitka_path" "$python_bin" -c \
        'from nuitka.Version import getNuitkaVersion; print(getNuitkaVersion())'
} 2>/dev/null || true)"
if [[ "$installed_nuitka_version" != "$nuitka_version" ]]; then
    printf 'Nuitka %s is required in %s (found: %s).\n' \
        "$nuitka_version" "$nuitka_root" "${installed_nuitka_version:-none}" >&2
    printf 'Run: %s -m pip install --target %q "Nuitka==%s" "ordered-set==4.1.0"\n' \
        "$python_bin" "$nuitka_root" "$nuitka_version" >&2
    exit 1
fi

# The same model checks the other two builds run, before anything is compiled.
PYTHONPATH="$project_dir/src" "$python_bin" "$project_dir/tools/verify_context_model.py"
PYTHONPATH="$project_dir/src" "$python_bin" "$project_dir/tools/verify_boundary_model.py"
PYTHONPATH="$project_dir/src" "$python_bin" "$project_dir/tools/verify_boundary_v2.py"
PYTHONPATH="$project_dir/src" "$python_bin" "$project_dir/tools/verify_prefix_model.py"
PYTHONPATH="$project_dir/src:$project_dir/tools" "$python_bin" "$project_dir/tools/verify_ortho_model.py"

rm -rf "$native_output"
mkdir -p "$native_output"

# Expanded through the "+" form below: the bash macOS ships treats an empty
# array under "set -u" as an unset variable and stops the build.
sign_arguments=()
if [[ -n "$sign_identity" ]]; then
    sign_arguments+=(--macos-sign-identity="$sign_identity" --macos-sign-notarization)
fi

# "app-dist" is a standalone build that macOS receives as a bundle.
# "ui-element" keeps KeySwitch out of the dock while still letting it open the
# settings window, which is what a keyboard helper should look like.
PYTHONPATH="${nuitka_path:+$nuitka_path:}$project_dir/src" \
"$python_bin" -m nuitka \
    --mode=app-dist \
    --lto=no \
    --output-dir="$native_output" \
    --output-filename="$binary_name" \
    --macos-app-name="$app_name" \
    --macos-app-version="$version" \
    --macos-app-mode=ui-element \
    --macos-signed-app-name="$bundle_identifier" \
    --macos-prohibit-multiple-instances \
    ${sign_arguments[@]+"${sign_arguments[@]}"} \
    --include-package-data=keyswitch \
    --include-data-files="$frozen_english_model=keyswitch/resources/models/en_US.lm" \
    --include-data-files="$frozen_russian_model=keyswitch/resources/models/ru_RU.lm" \
    --nofollow-import-to='keyswitch.windows_*' \
    --nofollow-import-to='keyswitch.app' \
    --nofollow-import-to='keyswitch.ui' \
    --nofollow-import-to='keyswitch.tray' \
    --nofollow-import-to='keyswitch.learning_prompt' \
    --nofollow-import-to='keyswitch.x11_backend' \
    --nofollow-import-to='keyswitch.atspi_context' \
    --nofollow-import-to=tests \
    --nofollow-import-to=gi \
    --no-progressbar \
    --assume-yes-for-downloads \
    --report="$native_output/compilation-report.xml" \
    "$project_dir/packaging/keyswitch_macos_entry.py"

# Nuitka names the bundle after the entry script; the product has its own name.
built_bundle="$(find "$native_output" -maxdepth 1 -name '*.app' -print -quit)"
if [[ -z "$built_bundle" ]]; then
    printf 'Nuitka produced no bundle in %s.\n' "$native_output" >&2
    exit 1
fi
if [[ "$built_bundle" != "$bundle" ]]; then
    rm -rf "$bundle"
    mv "$built_bundle" "$bundle"
fi

if [[ -n "$sign_identity" ]]; then
    keychain_arguments=()
    [[ -n "$sign_keychain" ]] && keychain_arguments+=(--keychain "$sign_keychain")
    # Nuitka signs the bundle; this re-signs it as one piece with a timestamp,
    # so the signature survives being moved and can be notarized.
    codesign --force --deep --options runtime --timestamp \
        --entitlements "$entitlements" \
        ${keychain_arguments[@]+"${keychain_arguments[@]}"} \
        --sign "$sign_identity" "$bundle"
    codesign --verify --strict --deep --verbose=2 "$bundle"
fi

# Each architecture ships its own archive: a bundle built on Apple silicon does
# not run on an Intel Mac, and the name has to say which is which.
architecture="$(uname -m)"
archive="$output_dir/${app_name}-${version}-macos-${architecture}.zip"
rm -f "$archive"
# ditto keeps the bundle's symbolic links and its signature intact; zip does not.
ditto -c -k --keepParent "$bundle" "$archive"

printf '\nBundle:  %s\n' "$bundle"
printf 'Archive: %s (%s)\n' "$archive" "$architecture"
if [[ -n "$sign_identity" ]]; then
    printf 'Signed:  %s\n' "$sign_identity"
else
    printf 'Signed:  no identity given, the bundle is unsigned\n'
fi

#!/usr/bin/env bash
# Installs cargo-llvm-cov, the tool the measure of coverage runs under, from the archive its author publishes for the
# system this runs on, and only if the archive is the one whose digest is written here. Building it from source took
# every job that measures about a minute. A system with no archive here, or an archive that cannot be fetched, gets the
# build from source; an archive with another digest stops the job.
# https://github.com/taiki-e/cargo-llvm-cov/releases
set -euo pipefail

version=0.9.1
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        archive=cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz
        digest=b3f68e625481fed9b16444174f3fa5ebcdbde4a1878803a35eabe2dcefcdc41a
        ;;
    Darwin-*)
        archive=cargo-llvm-cov-universal-apple-darwin.tar.gz
        digest=cc00420e3a5500e4d603399fde223d76d84c1446102d1b60a9fbbbce1c1c2b52
        ;;
    MINGW*-x86_64 | MSYS*-x86_64)
        archive=cargo-llvm-cov-x86_64-pc-windows-msvc.tar.gz
        digest=15eefd5b90fcdd4006b727c6e710d70a61aff509fcd7bb37f9c2a35c4f024948
        ;;
    *) archive="" ;;
esac

from_source() {
    printf '%s; building cargo-llvm-cov %s from source\n' "$1" "$version" >&2
    cargo install cargo-llvm-cov --version "$version" --locked
    exit 0
}

[[ -n "$archive" ]] || from_source "No archive for $(uname -s) on $(uname -m)"
folder="$(mktemp -d)"
trap 'rm -rf -- "$folder"' EXIT
curl --fail --silent --show-error --location --retry 3 --output "$folder/$archive" \
    "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v$version/$archive" || from_source "The archive could not be fetched"
if command -v sha256sum > /dev/null; then
    found="$(sha256sum "$folder/$archive" | cut -d ' ' -f 1)"
else
    found="$(shasum -a 256 "$folder/$archive" | cut -d ' ' -f 1)"
fi
if [[ "$found" != "$digest" ]]; then
    printf 'The archive %s has the digest %s, not %s\n' "$archive" "$found" "$digest" >&2
    exit 1
fi
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
# On Windows CARGO_HOME is written as Windows writes a path, and tar reads `C:` as the name of another machine.
if command -v cygpath > /dev/null; then
    bin="$(cygpath --unix "$bin")"
fi
mkdir -p -- "$bin"
tar -xzf "$folder/$archive" -C "$bin"
printf 'Installed cargo-llvm-cov %s from %s\n' "$version" "$archive"

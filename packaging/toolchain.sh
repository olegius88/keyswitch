#!/usr/bin/env bash
# The stable compiler CI builds and checks the program with: the version written here, not whatever `stable` names on the
# day a job runs. A new compiler brings new lints and builds other code: on 2026-11-12 Rust 1.100.0 came out between a
# release and the next run of the workflow, and four jobs failed on code nobody had changed; a release built that
# evening would have gone out from another compiler than its candidate. The version is raised here, in a change of its
# own, once every check passes with the new one.
#   toolchain.sh [clippy] [target...]
# makes that version the default of this machine, with clippy when it is named and the standard library of each target.
set -euo pipefail

version=1.99.1
components=()
targets=()
for name in "$@"; do
    if [[ "$name" == clippy ]]; then
        components+=(--component clippy)
    else
        targets+=("$name")
    fi
done
# An empty list is written this way for the bash of macOS, which takes its expansion for an unset name.
rustup toolchain install "$version" --profile minimal ${components[@]+"${components[@]}"}
rustup default "$version"
if ((${#targets[@]})); then
    rustup target add "${targets[@]}"
fi
rustc --version

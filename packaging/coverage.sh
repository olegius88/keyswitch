#!/usr/bin/env bash
# Measures test coverage on Linux: every test, the live tests in a private desktop, and the replays CI runs. The report is
# written to the file named by the first argument, and `ks-train coverage-gate` reads it.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
report="$(realpath -m -- "${1:?usage: coverage.sh <report.json>}")"
cd "$project_dir"

# Branch coverage needs a nightly compiler; the program itself is built with stable.
# https://github.com/taiki-e/cargo-llvm-cov#get-coverage-of-external-tests
export RUSTUP_TOOLCHAIN="${KS_COVERAGE_TOOLCHAIN:-nightly-2026-11-05}"
source <(cargo llvm-cov show-env --sh --branch)
cargo llvm-cov clean --workspace

status=0
# A failing step does not stop the others: the report then says what each of them did reach, and the run still fails.
step() {
    printf '\n== %s\n' "$*"
    if ! "$@"; then
        printf 'FAILED: %s\n' "$*"
        status=1
    fi
}

jobs="${KS_COVERAGE_JOBS:-4}"
step cargo test --profile coverage --workspace --no-fail-fast -j "$jobs"
step env KS_E2E_PROFILE=coverage packaging/e2e.sh

train() {
    "${CARGO_LLVM_COV_TARGET_DIR}/coverage/ks-train" "$@" --tree "$project_dir" > /dev/null
}
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
step cargo build --profile coverage -p ks-train --bins -j "$jobs"
step train boundary --verify
step train ortho --verify
step train ortho --inputs
step train archive
step train gates
step train licenses --verify
for family in boundary-v1 context-v2 ortho-v2 prefix-v1 context-corpus; do
    step train retired "$family" --replay
done
step train intent --verify
step train intent --preseal --output "$scratch/preseal.json"
step train intent --freeze --output "$scratch/development.json"
step train intent --evaluate --strict --output "$scratch/strict.json"

cargo llvm-cov report --profile coverage --branch --json --output-path "$report"
exit "$status"

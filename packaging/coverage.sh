#!/usr/bin/env bash
# Measures test coverage on Linux: every test, the live tests in a private desktop, and the replays CI runs. The report is
# written to the file named by the first argument, and `ks-train coverage-gate` reads it. The second argument names one
# part of the run: CI measures the eight parts on eight runners at once, each with its own report, and the gate joins
# those reports as it joins the reports of the three systems. Without it every step runs here, one after another.
# KS_COVERAGE_DRY_RUN=1 prints the steps of a part without running them.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
usage='usage: coverage.sh <report.json> [all|trainer-1|trainer-2|trainer-3|trainer-4|trainer-rest|models|engine|rest]'
report="$(realpath -m -- "${1:?$usage}")"
part="${2:-all}"
case "$part" in
    all | trainer-1 | trainer-2 | trainer-3 | trainer-4 | trainer-rest | models | engine | rest) ;;
    *)
        printf '%s\n' "$usage" >&2
        exit 2
        ;;
esac
dry_run="${KS_COVERAGE_DRY_RUN:-}"
cd "$project_dir"

# Branch coverage needs a nightly compiler; the program itself is built with stable.
# https://github.com/taiki-e/cargo-llvm-cov#get-coverage-of-external-tests
export RUSTUP_TOOLCHAIN="${KS_COVERAGE_TOOLCHAIN:-nightly-2026-11-05}"
# A small function of a library is otherwise compiled into whatever calls it, and a program that does not call it keeps an
# empty record that llvm-cov may read first: tested code would read as never run.
export RUSTFLAGS="${RUSTFLAGS:-} -Zcross-crate-inline-threshold=never"

# The tests of ks-train in the groups every runner of CI takes one of: `long_1` to `long_4`, and `rest_tests`.
source "$project_dir/packaging/test-parts.sh"

if [[ -z "$dry_run" ]]; then
    source <(cargo llvm-cov show-env --sh --branch)
    cargo llvm-cov clean --workspace
fi

status=0
# A failing step does not stop the others: the report then says what each of them did reach, and the run still fails.
step() {
    printf '\n== %s\n' "$*"
    if [[ -n "$dry_run" ]]; then
        return
    fi
    if ! "$@"; then
        printf 'FAILED: %s\n' "$*"
        status=1
    fi
}

jobs="${KS_COVERAGE_JOBS:-4}"
# `cargo test` of ks-train: the targets the leading options select (`--lib`, `--bins`, `--doc`), and the integration
# tests the names after them.
test_trainer() {
    local selection=() name
    while [[ "${1:-}" == --* ]]; do
        selection+=("$1")
        shift
    done
    for name in "$@"; do
        selection+=(--test "$name")
    done
    step cargo test --profile coverage -p ks-train --no-fail-fast -j "$jobs" "${selection[@]}"
}

train() {
    "${CARGO_LLVM_COV_TARGET_DIR}/coverage/ks-train" "$@" --tree "$project_dir" > /dev/null
}
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
# The models the repository rebuilds, by the instrumented trainer.
replays() {
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
}

# The parts take a runner about the same time each. The tests of ks-replay drive the engine through every recorded trace
# and every walk, and take as long as all the other crates together, so they are a part of their own; `rest` is every
# other crate but ks-train, and the live tests.
case "$part" in
    all)
        step cargo test --profile coverage --workspace --no-fail-fast -j "$jobs"
        step env KS_E2E_PROFILE=coverage packaging/e2e.sh
        replays
        ;;
    trainer-1) test_trainer "${long_1[@]}" ;;
    trainer-2) test_trainer "${long_2[@]}" ;;
    trainer-3) test_trainer "${long_3[@]}" ;;
    trainer-4) test_trainer "${long_4[@]}" ;;
    trainer-rest)
        test_trainer --lib --bins "${rest_tests[@]}"
        test_trainer --doc
        ;;
    models) replays ;;
    engine) step cargo test --profile coverage -p ks-replay --no-fail-fast -j "$jobs" ;;
    rest)
        step cargo test --profile coverage --workspace --exclude ks-train --exclude ks-replay --no-fail-fast -j "$jobs"
        step env KS_E2E_PROFILE=coverage packaging/e2e.sh
        ;;
esac

if [[ -z "$dry_run" ]]; then
    cargo llvm-cov report --profile coverage --branch --json --output-path "$report"
fi
exit "$status"

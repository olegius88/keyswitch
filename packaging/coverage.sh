#!/usr/bin/env bash
# Measures test coverage on Linux: every test, the live tests in a private desktop, and the replays CI runs. The report is
# written to the file named by the first argument, and `ks-train coverage-gate` reads it. The second argument names one
# part of the run: CI measures the four parts on four runners at once, each with its own report, and the gate joins those
# reports as it joins the reports of the three systems. Without it every step runs here, one after another.
# KS_COVERAGE_DRY_RUN=1 prints the steps of a part without running them.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
usage='usage: coverage.sh <report.json> [all|trainer-1|trainer-2|models|rest]'
report="$(realpath -m -- "${1:?$usage}")"
part="${2:-all}"
case "$part" in
    all | trainer-1 | trainer-2 | models | rest) ;;
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

# The integration tests of ks-train take most of the run. The longest are dealt out by name, three of them beside the
# replays of `models`, so that each part takes about twenty minutes of a four-core runner; every other test of ks-train
# runs in `rest`, so a new test file is measured without being listed here.
trainer_1=(fit_small cycle typing)
trainer_2=(models_kept corpora fit_whole)
models_tests=(curricula receipt commands)

# The parts name the targets of ks-train one by one, so the script stops on a target they would not run as a plain
# `cargo test` does (an example, a bench, a target with `test = false`, a library without doc tests), on a listed test that
# is gone and on a test listed twice. It checks this before measuring anything, in each part and in the whole run alike,
# so that a list gone stale stops a local run before it reaches CI.
found="$(cargo metadata --no-deps --format-version 1 | jq -r '
    .packages[] | select(.name == "ks-train") | .targets[] | select(.kind != ["custom-build"])
    | if (.kind - ["lib", "bin", "test"]) != [] or .test != true or (.kind == ["lib"] and .doctest != true)
      then error("packaging/coverage.sh does not deal out the target \(.name) of ks-train (\(.kind | join(",")))")
      elif .kind == ["test"] then .name
      else empty end')"
trainer_tests=()
if [[ -n "$found" ]]; then
    mapfile -t trainer_tests <<< "$found"
fi
declare -A listed=() known=()
problems=()
for name in "${trainer_1[@]}" "${trainer_2[@]}" "${models_tests[@]}"; do
    if [[ -n "${listed[$name]:-}" ]]; then
        problems+=("$name is listed twice")
    fi
    listed[$name]=1
done
rest_tests=()
for name in "${trainer_tests[@]}"; do
    known[$name]=1
    if [[ -z "${listed[$name]:-}" ]]; then
        rest_tests+=("$name")
    fi
done
for name in "${!listed[@]}"; do
    if [[ -z "${known[$name]:-}" ]]; then
        problems+=("$name is not a test of ks-train")
    fi
done
if ((${#problems[@]})); then
    printf 'packaging/coverage.sh: %s\n' "${problems[@]}" >&2
    exit 1
fi

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

case "$part" in
    all)
        step cargo test --profile coverage --workspace --no-fail-fast -j "$jobs"
        step env KS_E2E_PROFILE=coverage packaging/e2e.sh
        replays
        ;;
    trainer-1) test_trainer "${trainer_1[@]}" ;;
    trainer-2) test_trainer "${trainer_2[@]}" ;;
    models)
        test_trainer "${models_tests[@]}"
        replays
        ;;
    rest)
        step cargo test --profile coverage --workspace --exclude ks-train --no-fail-fast -j "$jobs"
        test_trainer --lib --bins "${rest_tests[@]}"
        test_trainer --doc
        step env KS_E2E_PROFILE=coverage packaging/e2e.sh
        ;;
esac

if [[ -z "$dry_run" ]]; then
    cargo llvm-cov report --profile coverage --branch --json --output-path "$report"
fi
exit "$status"

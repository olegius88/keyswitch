#!/usr/bin/env bash
# Every test of the engine, the trainer and LogCourier on the stable compiler, built as a release is but with its checks
# on (the `parity` profile). The measure of coverage runs the same tests, on a nightly compiler and with counters in the
# code; the program is built with stable, and a test that passes only beside the counters, or only on that nightly, would
# pass there unseen. CI runs this on every tag it checks and builds whole, in five parts on five runners at once (a tag
# that publishes the files of a candidate rests on the run of that candidate); `ks-release` runs the same tests on this
# machine before a release that no candidate went before.
# The argument names one part; without it every step runs here, one after another. KS_STABLE_DRY_RUN=1 prints the steps
# of a part without running them.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
usage='usage: stable-tests.sh [all|long-1|long-2|long-3|long-4|rest]'
part="${1:-all}"
case "$part" in
    all | long-1 | long-2 | long-3 | long-4 | rest) ;;
    *)
        printf '%s\n' "$usage" >&2
        exit 2
        ;;
esac
dry_run="${KS_STABLE_DRY_RUN:-}"
cd "$project_dir"

# The tests of ks-train in the groups every runner of CI takes one of: `long_1` to `long_4`, and `rest_tests`.
source "$project_dir/packaging/test-parts.sh"

step() {
    printf '\n== %s\n' "$*"
    if [[ -z "$dry_run" ]]; then
        "$@"
    fi
}

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
    step cargo test --profile parity -p ks-train "${selection[@]}"
}

# Every crate a release tests but ks-train, and what the groups of ks-train leave.
rest() {
    step cargo test --profile parity -p ks-text -p ks-lang -p ks-context -p ks-engine -p ks-config -p ks-replay -p logcourier
    test_trainer --lib --bins "${rest_tests[@]}"
    test_trainer --doc
}

case "$part" in
    all) step cargo test --profile parity -p ks-text -p ks-lang -p ks-context -p ks-engine -p ks-config -p ks-replay -p ks-train -p logcourier ;;
    long-1) test_trainer "${long_1[@]}" ;;
    long-2) test_trainer "${long_2[@]}" ;;
    long-3) test_trainer "${long_3[@]}" ;;
    long-4) test_trainer "${long_4[@]}" ;;
    rest) rest ;;
esac

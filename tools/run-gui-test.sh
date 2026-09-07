#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$project_dir/tools/gui-test-env.sh"
screen='-screen 0 1280x800x24'
if [[ "${1:-}" == --noreset ]]; then
    screen+=' -noreset'
    shift
fi
if [[ "${1:-}" == -- ]]; then
    shift
fi
if (($# == 0)); then
    printf 'Usage: %s [--noreset] -- command [args...]\n' "$0" >&2
    exit 2
fi
# Activated D-Bus services must inherit the private DISPLAY, not the desktop's.
exec xvfb-run -a -s "$screen" dbus-run-session -- "$@"

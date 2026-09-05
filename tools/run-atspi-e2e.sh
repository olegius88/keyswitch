#!/usr/bin/env bash
# Isolate both the X server and the activation environment of its a11y bus.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
task_atspi_runtime=$(mktemp -d /tmp/keyswitch-atspi-e2e.XXXXXX)
readonly task_atspi_runtime
cleanup() {
    if [[ "$task_atspi_runtime" == /tmp/keyswitch-atspi-e2e.?????? &&
          -d "$task_atspi_runtime" && ! -L "$task_atspi_runtime" ]]; then
        rm -rf -- "$task_atspi_runtime"
    fi
}
trap cleanup EXIT

# D-Bus must start INSIDE Xvfb: activated services inherit the daemon's
# DISPLAY, not the client's. Sharing the outer display/runtime can make
# parallel accessibility launchers bind the same socket.
XDG_RUNTIME_DIR="$task_atspi_runtime" xvfb-run -a dbus-run-session -- \
    env GIO_USE_VFS=local PYTHONPATH=src python3 tests/e2e_context_access.py

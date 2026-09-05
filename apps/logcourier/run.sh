#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
logcourier_qt_libs="$PWD/.local/qt-deps/unpacked/usr/lib/x86_64-linux-gnu"
if [[ -d "$logcourier_qt_libs" ]]; then
    export LD_LIBRARY_PATH="$logcourier_qt_libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
if [[ ! -x .venv/bin/python ]]; then
    python3 -m venv .venv
fi
if ! .venv/bin/python -c 'import logcourier, PySide6' >/dev/null 2>&1; then
    .venv/bin/python -m pip install -e .
fi
exec .venv/bin/python -m logcourier "$@"

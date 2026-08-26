#!/usr/bin/env bash
set -euo pipefail

typing_root="${KEYSWITCH_TYPING_ROOT:-}"
if [[ -n "$typing_root" ]]; then
  export PYTHONPATH="$typing_root${PYTHONPATH:+:$PYTHONPATH}"
fi

exec python3 -m mypy \
  --python-executable /usr/bin/python3 \
  --no-incremental \
  "$@"

#!/usr/bin/env bash
set -euo pipefail

typing_root="${KEYSWITCH_TYPING_ROOT:-}"
python=python3
if [[ -n "$typing_root" ]]; then
  export PYTHONPATH="$typing_root${PYTHONPATH:+:$PYTHONPATH}"
  # The interpreter tools/install-typing-tools.sh installed the compiled mypy for.
  if [[ -x "$typing_root/python3" ]]; then
    python="$typing_root/python3"
  fi
fi

exec "$python" -m mypy \
  --python-executable /usr/bin/python3 \
  --no-incremental \
  "$@"

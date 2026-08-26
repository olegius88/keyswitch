#!/usr/bin/env bash
set -euo pipefail

target="${1:-.typing}"

python3 -m pip install --upgrade --target "$target" \
  "mypy==2.3.1" \
  "typing_extensions>=4.6"
python3 -m pip install --upgrade --target "$target" --no-deps \
  "PyGObject-stubs==2.17.0"

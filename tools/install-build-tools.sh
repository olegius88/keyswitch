#!/usr/bin/env bash
set -euo pipefail

target="${1:-.nuitka}"

python3 -m pip install --upgrade --target "$target" \
  "Nuitka==4.1.3" \
  "ordered-set==4.1.0"

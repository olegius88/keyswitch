#!/usr/bin/env bash
set -euo pipefail

target="${1:-.nuitka}"

python3 -m pip install --upgrade --target "$target" \
  "Nuitka==4.2" \
  "ordered-set==4.1.0"

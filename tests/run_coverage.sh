#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

export PYTHONPATH="$project_root/src${PYTHONPATH:+:$PYTHONPATH}"
export GTK_THEME="${GTK_THEME:-Adwaita}"
export GTK_A11Y="${GTK_A11Y:-none}"
export PYTHONWARNINGS="${PYTHONWARNINGS:-ignore::DeprecationWarning}"

python3 -m coverage erase
python3 -m coverage run -m unittest discover -s tests -v
python3 -m coverage report

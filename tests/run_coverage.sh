#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
source "$project_root/tools/gui-test-env.sh"

export PYTHONPATH="$project_root/src${PYTHONPATH:+:$PYTHONPATH}"
export GTK_THEME="${GTK_THEME:-Adwaita}"
export GTK_A11Y="${GTK_A11Y:-none}"
export PYTHONWARNINGS="${PYTHONWARNINGS:-ignore::DeprecationWarning}"

# KEYSWITCH_TEST_PART=K/N runs part K of N of the modules (tools/run_test_modules.py --part), for N
# runners sharing the suite: each leaves its data in .coverage, and the report is made where the parts'
# data files are joined, since no part covers every line.
part="${KEYSWITCH_TEST_PART:-}"

python3 -m coverage erase
# Every module in a process of its own, as many at a time as there are cores (tools/run_test_modules.py);
# the modules that open windows, read the clipboard or listen to AT-SPI on the shared display take turns.
# The data files of every module are joined whether or not one failed, so none is left behind.
status=0
python3 tools/run_test_modules.py ${part:+--part "$part"} \
  --serial test_context_access.py --serial test_input_integrity.py --serial test_learning_prompt.py \
  --serial test_tray_app.py --serial test_ui.py --serial test_x11_backend.py \
  'test_*.py' -- python3 -m coverage run --parallel-mode -m unittest discover -s tests -v -p || status=$?
python3 -m coverage combine --quiet
if ((status != 0)); then
  exit "$status"
fi
if [[ -n "$part" ]]; then
  printf 'Part %s of the modules ran; the coverage report joins every part.\n' "$part"
  exit 0
fi
python3 -m coverage report

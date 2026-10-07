#!/usr/bin/env bash
set -euo pipefail

target="${1:-.typing}"

python3 -m pip install --upgrade --target "$target" \
  "mypy==2.3.1" \
  "Pillow==12.3.0" \
  "typing_extensions>=4.6"
python3 -m pip install --upgrade --target "$target" --no-deps \
  "PyGObject-stubs==2.17.0"
# NumPy's own stubs type the context-v3 trainer's back ends. Those of NumPy 2.3 and later use syntax
# of Python 3.12, which mypy refuses for the project's target version (3.10), so the stubs come from
# 2.2.6, the last release for 3.10, as a wheel for 3.12 that nothing imports: training itself runs
# the NumPy of tools/install-training-accelerators.sh.
python3 -m pip install --upgrade --target "$target" --no-deps --only-binary=:all: --python-version 3.12 \
  "numpy==2.2.6"
# mypy is compiled for the interpreter that installed it; tools/typecheck.sh runs it with that one even
# when python3 on PATH later names another.
ln -sfn "$(python3 -c 'import sys; print(sys.executable)')" "$target/python3"

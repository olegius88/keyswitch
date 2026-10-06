#!/usr/bin/env bash
# Installs what the context-v3 trainer's back ends need into build/training-site (ignored by git),
# for the python3 on PATH; tools/context_action_pipeline.py looks there first.
#   tools/install-training-accelerators.sh          NumPy: the cpu back end (cloud sessions included)
#   tools/install-training-accelerators.sh --cuda   also CuPy, NVRTC, the CUDA runtime and CCCL headers:
#                                                   the gpu back end (an NVIDIA driver for CUDA 13)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="$root/build/training-site"
packages=("numpy==2.5.3")
if [[ "${1:-}" == "--cuda" ]]; then
  packages+=("cupy-cuda13x==14.2.0" "cuda-toolkit[nvrtc,cudart,cccl]==13.4.2")
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--cuda]" >&2
  exit 2
fi
python3 -m pip install --upgrade --target "$target" "${packages[@]}"

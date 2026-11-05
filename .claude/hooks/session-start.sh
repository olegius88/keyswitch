#!/bin/bash
# Prepares a Claude Code on the web container for model training and tests: the Rust toolchain and what the trainer and
# the Linux program link or load.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

project_root="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$project_root"

packages=(at-spi2-core dbus hunspell-en-us hunspell-ru libadwaita-1-dev libgtk-4-dev libhunspell-1.7-0 onboard-data xvfb)
sudo apt-get update -qq
sudo apt-get install -y -qq "${packages[@]}"
rustup toolchain install stable --profile minimal
cargo build --profile train -p ks-train

#!/usr/bin/env bash
# Builds the Linux program from this tree and starts it, passing the arguments on: `./run.sh`, `./run.sh --diagnose`.
# The program reads the tree's resources and the frozen word lists that the package installs beside them
# (packaging/build-rust-deb.sh), so a run from here decides as the installed program does.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# rustup puts cargo here; a shell opened before the installation does not have it on its path yet.
cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
if ! command -v cargo > /dev/null && [[ -x "$cargo_bin/cargo" ]]; then
    PATH="$cargo_bin:$PATH"
fi
if ! command -v cargo > /dev/null; then
    printf '%s\n' "Не найден cargo. Установите Rust через rustup (https://rustup.rs):" \
        "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    exit 1
fi

# What the CI builds the program with (.github/workflows/ci.yml); the C compiler is the linker Rust calls.
# libgtk-4-dev brings pkg-config with it.
missing=()
command -v cc > /dev/null || missing+=(build-essential)
pkg-config --exists gtk4 gtk4-x11 2> /dev/null || missing+=(libgtk-4-dev)
pkg-config --exists libadwaita-1 2> /dev/null || missing+=(libadwaita-1-dev)
if (( ${#missing[@]} > 0 )); then
    printf '%s\n' "Для сборки не хватает пакетов. Установите их:" "  sudo apt install ${missing[*]}" >&2
    exit 1
fi
# The windows use libadwaita 1.5 (the feature v1_5 in crates/ks-linux/Cargo.toml).
if ! pkg-config --exists 'libadwaita-1 >= 1.5'; then
    printf 'Нужна libadwaita 1.5 или новее (она есть в Ubuntu 24.04 и новее), а установлена %s.\n' \
        "$(pkg-config --modversion libadwaita-1)" >&2
    exit 1
fi

cd "$project_dir"
export KEYSWITCH_RESOURCES="$project_dir/resources"
export KEYSWITCH_MODEL_PATH="$project_dir/model/intent_v1/sources"
exec cargo run --release -p ks-linux --bin keyswitch -- "$@"

#!/bin/bash
# Prepares a Claude Code on the web container for model training and tests.
# Installs the same system lexicons, Hunspell and compiler as the CI job, the
# pinned typing tools, and exports the environment the trainers expect.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

project_root="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$project_root"

# The package list of the Tests job (.github/workflows/tests.yml): reference
# lexicons (onboard-data), reference morphology (hunspell-*), the C compiler
# for tools/context_optimizer.c, and GTK/Xvfb for tests/run_coverage.sh. Their
# lexicon bytes are checked against model/intent_v1/config.json by
# tools/training_doctor.py.
packages=(
  at-spi2-core
  build-essential
  ccache
  dbus-x11
  desktop-file-utils
  file
  gir1.2-adw-1
  gir1.2-atspi-2.0
  gir1.2-gtk-4.0
  hunspell-en-us
  hunspell-ru
  libglib2.0-bin
  libhunspell-1.7-0
  libx11-6
  libxkbcommon0
  libxtst6
  lintian
  onboard-data
  patch
  patchelf
  python3-coverage
  python3-dbus
  python3-dev
  python3-gi
  python3-pip
  x11-xkb-utils
  xauth
  xvfb
)
missing=()
for package in "${packages[@]}"; do
  if ! dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q "install ok installed"; then
    missing+=("$package")
  fi
done
if [ "${#missing[@]}" -gt 0 ]; then
  sudo_cmd=()
  if [ "$(id -u)" -ne 0 ]; then
    sudo_cmd=(sudo)
  fi
  "${sudo_cmd[@]}" apt-get update -qq
  DEBIAN_FRONTEND=noninteractive "${sudo_cmd[@]}" apt-get install --yes -qq \
    --no-install-recommends "${missing[@]}"
fi

# Pinned mypy for tools/typecheck.sh; .typing/ is ignored by git.
if ! PYTHONPATH="$project_root/.typing" python3 -c "import mypy" 2>/dev/null; then
  ./tools/install-typing-tools.sh "$project_root/.typing" >/dev/null
fi

if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  {
    echo "export PYTHONPATH=\"$project_root/src\${PYTHONPATH:+:\$PYTHONPATH}\""
    echo "export KEYSWITCH_TYPING_ROOT=\"$project_root/.typing\""
  } >> "$CLAUDE_ENV_FILE"
fi

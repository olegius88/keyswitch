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

# The apt bindings (python3-gi, python3-dbus, python3-coverage) are built for
# the distribution's Python, while a cloud image may point python3 at another
# build. Put the first system Python that imports them first on PATH as
# python3, so the GTK tests, the trainers and pip all use the same one.
python_bin="${HOME:?}/.local/share/keyswitch-cloud/bin"
interpreter=""
for candidate in /usr/bin/python3 /usr/bin/python3.14 /usr/bin/python3.13 /usr/bin/python3.12; do
  if [ -x "$candidate" ] && "$candidate" -c "import gi, dbus, coverage" 2>/dev/null; then
    interpreter="$candidate"
    break
  fi
done
if [ -z "$interpreter" ]; then
  echo "session-start: no system Python imports gi, dbus and coverage" >&2
  exit 1
fi
mkdir -p "$python_bin"
ln -sfn "$interpreter" "$python_bin/python3"
export PATH="$python_bin:$PATH"

# Pinned mypy for tools/typecheck.sh, built for that interpreter; .typing/ is
# ignored by git.
if ! PYTHONPATH="$project_root/.typing" python3 -m mypy --version >/dev/null 2>&1 \
    || [ ! -d "$project_root/.typing/gi-stubs" ] || [ ! -d "$project_root/.typing/numpy" ]; then
  rm -rf "$project_root/.typing"
  ./tools/install-typing-tools.sh "$project_root/.typing" >/dev/null
fi

# NumPy for the context-v3 trainer's cpu back end, in build/training-site (ignored by git); exported
# below, so that every worktree of the session trains with it.
training_site="$project_root/build/training-site"
if ! PYTHONPATH="$training_site" python3 -c "import numpy" >/dev/null 2>&1; then
  KEYSWITCH_TRAINING_SITE="$training_site" ./tools/install-training-accelerators.sh >/dev/null
fi

if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  {
    echo "export PATH=\"$python_bin:\$PATH\""
    echo "export PYTHONPATH=\"$project_root/src\${PYTHONPATH:+:\$PYTHONPATH}\""
    echo "export KEYSWITCH_TYPING_ROOT=\"$project_root/.typing\""
    echo "export KEYSWITCH_TRAINING_SITE=\"$training_site\""
  } >> "$CLAUDE_ENV_FILE"
fi

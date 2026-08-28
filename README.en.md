# KeySwitch

[Русский](README.md) · [**English**](README.en.md)

[![GitHub release](https://img.shields.io/github/v/release/olegius88/keyswitch)](https://github.com/olegius88/keyswitch/releases/latest)
[![Tests](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml)
[![Native packages](https://github.com/olegius88/keyswitch/actions/workflows/release.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/release.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KeySwitch is a desktop application for Windows 10/11 x64 and Ubuntu/Xubuntu
X11 that automatically corrects words typed using the wrong keyboard layout.
It serves a similar purpose to Punto Switcher and EveryLang, while running
entirely locally and using the active EN/RU system layout pair.

## Features

- global input observation through `WH_KEYBOARD_LL` on Windows and XRecord in
  regular Linux X11 applications;
- automatic English and Russian word detection after 1.5 seconds without
  input, as well as after Space, Enter, Tab or punctuation; idle correction
  can be disabled independently in settings;
- ensemble detection using frequency lexicons, Hunspell morphology, character
  n-grams and recent context;
- conservative guards for URLs, paths, code, abbreviations, ambiguous words
  and common technical terms;
- local explicit learning: after manual conversion with `Pause/Break`, a prompt
  appears above the input field; `Enter` immediately adds the word to the
  rules, `Esc` rejects the offer, and undoing a false correction records a
  rejection;
- system layout switching and correction of the already typed word through
  Win32 `SendInput` or XTEST;
- respect for manual layout selection: the first completed word after the user
  switches languages is left unchanged; this behavior can be disabled;
- case preservation: `Ghbdtn` becomes `Привет`;
- manual conversion of the last word with `Pause`;
- undo of the last correction for 10 seconds with `Ctrl+Alt+Z`;
- global pause with `Ctrl+Alt+P`;
- application exclusions selected from the active window, the installed
  application catalog, a Windows `.exe` picker or entered manually, plus word
  exclusions;
- local history containing only completed corrections;
- notifications, sound, light/dark themes and startup after OS login;
- a live `EN/RU` or country-flag layout indicator; either a left or right click
  opens settings, pause, sound, notifications, history, exclusions, about and
  quit actions;
- a native full settings window with overview, test field, automatic
  correction, hotkeys, exclusions, history and backend diagnostics;
- single-instance protection: launching KeySwitch again activates the existing
  application window.
- automatic stable GitHub Release checks after startup and every six hours;
  Windows downloads the verified Setup EXE, installs it silently and restarts
  KeySwitch, while Ubuntu reports the release and hands installation to the
  system package manager.

KeySwitch does not record the complete keystroke stream. Only the current word
is kept in memory. When history is enabled, it stores correction pairs such as
`ghbdtn → привет` and nothing else.

## Install on Windows

Download `KeySwitch-Setup-0.5.0-x64.exe` from the
[latest release](https://github.com/olegius88/keyswitch/releases/latest) and run
it. The per-user installation goes to `%LOCALAPPDATA%\Programs\KeySwitch` and
does not require administrator privileges. The release also includes the
portable `KeySwitch-0.5.0-windows-x64.zip` archive.

After launch, KeySwitch appears in the notification area. Left- or right-click
the `EN/RU` or flag icon to open its menu. Startup, start minimized, indicator,
sound and notifications can be changed on the Appearance and System page.

Automatic checks and installation are enabled by default on the Updates page.
Thirty seconds after startup and every six hours thereafter, KeySwitch reads
the latest stable release from
[`olegius88/keyswitch`](https://github.com/olegius88/keyswitch/releases), then
validates the exact Setup EXE name, URL, size and GitHub API SHA-256 digest. If
a newer version exists, the per-user installer runs silently; KeySwitch exits
and starts minimized again after installation. Automatic checking and
installation can be disabled independently.

To run from source on Windows:

```powershell
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[windows]"
.venv\Scripts\keyswitch
```

For the complete detection model when running from source, point
`KEYSWITCH_MODEL_PATH` to a directory containing Onboard's `en_US.lm` and
`ru_RU.lm`. The Setup EXE and portable ZIP bundle these models automatically.

## Quick start on Ubuntu

The currently verified environment is Ubuntu 26.04.1 LTS with XFCE, X11 and
the `us,ru` system layouts.

```bash
git clone https://github.com/olegius88/keyswitch.git
cd keyswitch
./run.sh
```

The application window includes a test field. Switch to EN, type `ghbdtn` and
stop: after about 1.5 seconds the field should contain `привет` and the active
layout should be RU. You can also press Space for an immediate check. For the
reverse direction, switch to RU and type `hello` using the same physical keys.

To teach KeySwitch a personal exception, type the word and press `Pause/Break`.
After the manual replacement, a prompt above the input position asks whether
to add the word to switching rules. Pressing `Enter` activates the rule
immediately; `Esc` dismisses it. The Local learning switch disables the whole
mechanism. If the prompt is not confirmed, a rule can still become active after
the configured number of repeated manual conversions.

Probe the system backend without opening the application window:

```bash
./run.sh --diagnose
```

## Install the Debian package

Download `keyswitch_0.5.0_amd64.deb` from the
[latest release](https://github.com/olegius88/keyswitch/releases/latest), then
install it with:

```bash
sudo apt install ./keyswitch_0.5.0_amd64.deb
```

The package installs the required system dependencies and adds KeySwitch to the
application menu. Nuitka compiles the application into an architecture-specific
ELF executable: `/usr/lib/keyswitch` contains no application `.py` sources or
`.pyc` bytecode, and the package does not depend on the system Python
interpreter. The native runtime includes `libpython`, so the `amd64` package
cannot be installed on a different architecture.

Release checking also works on Ubuntu, but KeySwitch does not silently install
a system DEB itself: that requires APT authorization and a configured package
repository. When a new version is found, the app shows a notification and an
Open release button; installation remains an explicit APT action. This is a
Linux system-privilege boundary, not a limitation of release discovery.

## Install from source for the current user

```bash
./install.sh
keyswitch
```

The installer does not require root access. It places the application,
launcher, desktop entry and icons under `~/.local`. XDG Autostart is enabled by
default after the first launch, so KeySwitch starts at the next desktop login
after a reboot. This can be changed on the Appearance and System settings page.

To uninstall while preserving settings and history:

```bash
./uninstall.sh
```

## System dependencies

The required components may already be present on a standard current Ubuntu
installation. If the installer reports missing dependencies, install them with:

```bash
sudo apt install at-spi2-core python3-gi python3-dbus gir1.2-gtk-4.0 gir1.2-adw-1 \
  gir1.2-atspi-2.0 \
  libx11-6 libxtst6 libxkbcommon0 libhunspell-1.7-0 \
  hunspell-en-us hunspell-ru onboard-data
```

Check the active XKB layout pair with:

```bash
setxkbmap -query
```

This release uses the first two XKB groups. For the expected scenario, the
output should contain `layout: us,ru` or the same pair in reverse order. The
language models in the settings are currently bound to the EN, RU order.

## Settings and data

On Windows:

- settings: `%APPDATA%\KeySwitch\config.json`;
- history, learning data, custom dictionaries and log:
  `%LOCALAPPDATA%\KeySwitch`;
- autostart: the per-user
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` registry key.

On Linux:

- settings: `~/.config/keyswitch/config.json`;
- correction history: `~/.local/share/keyswitch/history.jsonl`;
- explicitly learned rules and rejected corrections:
  `~/.local/share/keyswitch/learning.json`;
- optional per-user Hunspell dictionaries:
  `~/.local/share/keyswitch/dictionaries/<locale>.aff/.dic`;
- application and error log: `~/.local/share/keyswitch/keyswitch.log`;
- autostart entry: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

`KeePassXC`, `1Password` and `Bitwarden` are excluded by default. A global
observer does not know the semantics of an individual input field, so other
sensitive applications should be added by `.exe` name on Windows or by
`WM_CLASS` on Linux using the Exclusions settings page.

## Development and verification

All Python application, test and tool code passes an enhanced `mypy --strict`
profile. It rejects untyped definitions, explicit `Any`, `Any` from unfollowed
imports and untyped decorators, and additionally checks unreachable branches,
possibly undefined values and unused awaitables. Run it locally on Ubuntu with:

```bash
sudo apt install python3-pip
./tools/install-typing-tools.sh .typing
KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh
```

To keep the `dbus-python` boundary strict, the repository carries a narrow
contract for only the API surface it actually uses under `typings/dbus`.

Run the complete unit and GTK interaction suite with the mandatory 100% line
and branch coverage gate (GTK needs an active X11 display or Xvfb):

```bash
./tests/run_coverage.sh
```

For headless execution, run the same command inside `dbus-run-session` and
`xvfb-run`; GitHub Actions uses that exact setup. The report stops the build if
coverage drops below 100%.

On Windows, a separate E2E starts a real `WH_KEYBOARD_LL` hook, types scan codes
through `SendInput` into a Tk field and verifies both directions, the learning
prompt, `Enter` confirmation, automatic reuse of the learned rule, the final
layout and history:

```powershell
$env:PYTHONPATH = "src"
py tests/e2e_windows.py
```

Run the reproducible 40,000-word frequency/broad-dictionary corpus and
defensive cases with:

```bash
PYTHONPATH=src tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
```

Run the real end-to-end test in an active X11 session with:

```bash
PYTHONPATH=src python3 tests/e2e_x11.py
```

A successful run prints `E2E_OK` after real corrections, `Pause/Break` +
`Enter` learning and verification of the one-word guard following a manual
layout switch inside a GTK Entry. An
additional integration test exports a real StatusNotifierItem and DBusMenu on
an isolated session bus:

```bash
dbus-run-session -- env PYTHONPATH=src python3 tests/e2e_tray_menu.py
```

It prints `TRAY_MENU_E2E_OK` after registering the indicator, reading every
menu item and activating Settings. The architecture and acceptance criteria
are documented in [DESIGN.md](DESIGN.md). The
[detection research](docs/detection-research.en.md) compares the verified
mechanisms in existing products and explains the model.

Build the reproducible native Debian package with:

```bash
sudo apt install build-essential ccache patch patchelf python3-dev python3-pip
./tools/install-build-tools.sh .nuitka
KEYSWITCH_NUITKA_ROOT=.nuitka ./packaging/build-deb.sh
package="dist/keyswitch_0.5.0_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
```

Build the native Windows artifacts on Windows with Python, Nuitka and Inno
Setup:

```powershell
./packaging/build-windows.ps1 `
  -ModelDirectory build/windows-models/models `
  -ModelLicense build/windows-models/COPYRIGHT.onboard-data
```

This produces a standalone `KeySwitch.exe` without application `.py` files, a
portable ZIP and a per-user Setup EXE. The `en_US.lm` and `ru_RU.lm` files come
from the Onboard package together with its license.

After stopping any already running instance, test the executable extracted
from the package in an active X11 session with:

```bash
dbus-run-session -- ./tools/run-native-e2e.sh "$package"
```

It verifies six real corrections, a manually selected layout left unchanged
for one word, XKB group changes, history, StatusNotifierItem registration and
the popup DBusMenu contents. Pushing a `v*` tag makes GitHub Actions validate
Linux and Windows independently, build the DEB, Windows Setup EXE and portable
ZIP, silently install and smoke-test the installed application, create one
`SHA256SUMS` file and publish all artifacts in a GitHub Release.

## Limitations

- The Linux backend is designed for X11. In a native Wayland session, KeySwitch
  reports an explicit diagnostic error instead of pretending that global
  correction is available.
- The default automatic language models target the EN/RU layout pair.
- Applications with custom input handling and remote desktops may process
  synthetic events differently; these applications can be added to the
  exclusions list.
- On Windows, UIPI prevents a regular process from injecting input into a
  window running at a higher integrity level. KeySwitch needs a matching level
  for that target window.
- The Windows 0.5.0 Setup EXE is not yet signed with a publisher certificate.

## License

KeySwitch is distributed under the
[GNU General Public License 3.0 or later](LICENSE).

## Primary specifications

- [Microsoft LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
  — low-level keyboard hook and required message loop;
- [Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
  — synthetic input and the UIPI limitation;
- [Microsoft ToUnicodeEx](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-tounicodeex)
  — translating a virtual key with a selected layout;
- [Microsoft Run and RunOnce keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)
  — per-user startup after Windows login;
- [X.Org RECORD Extension Library](https://www.x.org/releases/current/doc/libXtst/recordlib.pdf)
  — context creation and event capture;
- [X.Org XTEST Extension Library](https://www.x.org/releases/current/doc/libXtst/xtestlib.pdf)
  — synthetic KeyPress and KeyRelease events;
- [X.Org XKB Library Specification](https://www.x.org/releases/current/doc/libX11/XKB/xkblib.html)
  — groups, Shift levels and keycode-to-keysym conversion;
- [Freedesktop Autostart Specification](https://specifications.freedesktop.org/autostart/0.5/)
  — the per-user `~/.config/autostart/*.desktop` entry;
- [Freedesktop StatusNotifierItem](https://specifications.freedesktop.org/status-notifier-item/latest-single/)
  — desktop panel indicator integration;
- [Canonical DBusMenu interface](https://sources.debian.org/src/libdbusmenu/18.10.20180917~bzr492%2Brepack1-2/libdbusmenu-glib/dbus-menu.xml/)
  — native desktop-indicator popup menu;
- [Nuitka User Manual](https://nuitka.net/user-documentation/user-manual.html)
  — compilation into a standalone executable;
- [official Ubuntu 26.04 LTS release notes](https://documentation.ubuntu.com/release-notes/26.04/).

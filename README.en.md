# KeySwitch

[Русский](README.md) · [**English**](README.en.md)

[![GitHub release](https://img.shields.io/github/v/release/olegius88/keyswitch)](https://github.com/olegius88/keyswitch/releases/latest)
[![Debian package](https://github.com/olegius88/keyswitch/actions/workflows/release.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/release.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KeySwitch is a desktop application for Ubuntu and Xubuntu that automatically
corrects words typed using the wrong keyboard layout. It serves a similar
purpose to Punto Switcher and EveryLang, while running entirely locally and
using the active pair of system XKB layouts.

## Features

- global input observation in regular X11 applications;
- automatic English and Russian word detection after Space, Enter, Tab or
  punctuation;
- XKB group switching together with correction of the already typed word;
- case preservation: `Ghbdtn` becomes `Привет`;
- manual conversion of the last word with `Pause`;
- undo of the last correction for 10 seconds with `Ctrl+Alt+Z`;
- global pause with `Ctrl+Alt+P`;
- application exclusions based on `WM_CLASS` and word exclusions;
- local history containing only completed corrections;
- notifications, sound, light/dark themes and XDG Autostart integration;
- a StatusNotifier indicator for XFCE/KDE-compatible panels;
- a complete settings window with overview, test field and backend diagnostics.

KeySwitch does not record the complete keystroke stream. Only the current word
is kept in memory. When history is enabled, it stores correction pairs such as
`ghbdtn → привет` and nothing else.

## Quick start

The currently verified environment is Ubuntu 26.04.1 LTS with XFCE, X11 and
the `us,ru` system layouts.

```bash
git clone https://github.com/olegius88/keyswitch.git
cd keyswitch
./run.sh
```

The application window includes a test field. Switch to EN, type `ghbdtn` and
press Space: the field should contain `привет `. For the reverse test, switch
to RU and type `hello` using the same physical keys: the resulting `руддщ `
will be replaced with `hello `.

Probe the system backend without opening the application window:

```bash
./run.sh --diagnose
```

## Install the Debian package

Download `keyswitch_0.1.0_all.deb` from the
[latest release](https://github.com/olegius88/keyswitch/releases/latest), then
install it with:

```bash
sudo apt install ./keyswitch_0.1.0_all.deb
```

The package installs the required system dependencies and adds KeySwitch to the
application menu.

## Install from source for the current user

```bash
./install.sh
keyswitch
```

The installer does not require root access. It places the application,
launcher, desktop entry and icons under `~/.local`. Autostart remains disabled
until it is enabled on the Appearance and System settings page.

To uninstall while preserving settings and history:

```bash
./uninstall.sh
```

## System dependencies

The required components may already be present on a standard current Ubuntu
installation. If the installer reports missing dependencies, install them with:

```bash
sudo apt install python3-gi python3-dbus gir1.2-gtk-4.0 gir1.2-adw-1 \
  libx11-6 libxtst6 libxkbcommon0 onboard-data
```

Check the active XKB layout pair with:

```bash
setxkbmap -query
```

This release uses the first two XKB groups. For the expected scenario, the
output should contain `layout: us,ru` or the same pair in reverse order. The
language models in the settings are currently bound to the EN, RU order.

## Settings and data

- settings: `~/.config/keyswitch/config.json`;
- correction history: `~/.local/share/keyswitch/history.jsonl`;
- application and error log: `~/.local/share/keyswitch/keyswitch.log`;
- autostart entry: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

`KeePassXC`, `1Password` and `Bitwarden` are excluded by default. X11 does not
provide the global observer with the semantics of an individual input field,
so other sensitive applications should be added by their `WM_CLASS` on the
Exclusions settings page.

## Development and verification

Run the unit tests with:

```bash
PYTHONPATH=src python3 -m unittest discover -s tests -v
```

Run the real end-to-end test in an active X11 session with:

```bash
PYTHONPATH=src python3 tests/e2e_x11.py
```

A successful run prints `E2E_OK` after two real corrections inside a GTK Entry.
The architecture and acceptance criteria are documented in
[DESIGN.md](DESIGN.md).

Build the Debian package with:

```bash
./packaging/build-deb.sh
dpkg-deb --info dist/keyswitch_0.1.0_all.deb
```

Pushing a `v*` tag makes GitHub Actions repeat the unit tests, build the package,
create `SHA256SUMS` and publish both files in a GitHub Release.

## Limitations

- The current backend is designed for X11. In a native Wayland session,
  KeySwitch reports an explicit diagnostic error instead of pretending that
  global correction is available.
- The default automatic language models target the EN/RU layout pair.
- Applications with custom input handling and remote desktops may process
  synthetic XTEST events differently; these applications can be added to the
  exclusions list.

## License

KeySwitch is distributed under the
[GNU General Public License 3.0 or later](LICENSE).

## Primary specifications

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
- [official Ubuntu 26.04 LTS release notes](https://documentation.ubuntu.com/release-notes/26.04/).

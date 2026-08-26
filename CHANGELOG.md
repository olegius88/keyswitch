# Changelog

All notable changes to KeySwitch are documented in this file.

## 0.2.0 — 2026-08-27

- Enable XDG Autostart by default and keep its desktop entry synchronized.
- Add active-window capture, installed-application search and removable rows
  for application exclusions.
- Add a live tray layout indicator with selectable `EN/RU` and country-flag
  styles.
- Add a native left-click DBusMenu with settings, correction toggles, history,
  exclusions, about and quit actions.
- Replace exact-word-only detection with a precision-first ensemble of
  frequency lexicons, Hunspell morphology and smoothed character n-grams.
- Add per-application short-term language context, technical-token guards and
  conservative handling of ambiguous words.
- Learn explicit rules after repeated manual conversions and remember an
  automatic correction rejected with Undo; ordinary typed text is not stored.
- Correct words whose physical letter keys are punctuation in the other
  layout, including `,fpf` to `база`, while preserving punctuation boundaries.
- Add a reproducible 40,000-word detector benchmark with strict CI quality
  gates.
- Enforce an enhanced `mypy --strict` profile across application code, tests
  and developer tools, including typed GTK, D-Bus and ctypes boundaries.
- Compile the application with pinned Nuitka into an architecture-specific
  ELF Debian package without application Python source or bytecode files.
- Add package-structure, dynamic-link and packaged-native X11/DBusMenu E2E
  gates to both test and release workflows.

## 0.1.0 — 2026-08-26

- First public release for Ubuntu X11.
- Automatic bidirectional correction between English and Russian layouts.
- Manual conversion, correction undo and global pause hotkeys.
- GTK 4 and Libadwaita settings window with eight configuration pages.
- Application and word exclusions, local correction history and diagnostics.
- XDG autostart and StatusNotifierItem desktop integration.
- Reproducible `.deb` package build and tag-driven GitHub Release workflow.

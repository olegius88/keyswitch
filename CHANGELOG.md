# Changelog

All notable changes to KeySwitch are documented in this file.

## Unreleased

## 0.4.0 — 2026-08-28

- Correct a likely wrong-layout word after 1.5 seconds without input, before a
  separator is typed, with a dedicated option to disable idle correction.
- Render Windows country flags at the maximum notification-area icon size
  without the former purple frame.
- Add an EveryLang-style learning prompt after a `Pause/Break` manual
  conversion. `Enter` immediately confirms the word as a switching rule,
  while Escape, unrelated input or an eight-second timeout dismisses it.
- Position the prompt at the text caret when the platform exposes it, with a
  pointer fallback on X11, and restore the original Windows target after the
  focused prompt closes.
- Keep repeated manual conversions as a configurable fallback and retain the
  strict 100% line/branch coverage gate for the expanded core and GTK UI.

## 0.3.0 — 2026-08-27

- Add a native Windows x64 backend using `WH_KEYBOARD_LL`, `ToUnicodeEx`,
  `WM_INPUTLANGCHANGEREQUEST` and `SendInput` without clipboard replacement.
- Preserve manual EN/RU layout choices and the same correction, manual-convert,
  undo, learning, context and exclusion semantics on both supported platforms.
- Add a Windows settings application with nine sections, live diagnostics,
  history, a real test field and complete controls for the shared settings.
- Add `.exe` file picking, active-window targeting and registered App Paths for
  application exclusions on Windows.
- Add per-user Windows logon autostart and a dynamic notification-area icon in
  either `EN/RU` or country-flag style; either mouse button opens the full menu.
- Prevent duplicate Windows instances and focus the existing KeySwitch window
  when the executable is launched again.
- Compile a source-free Windows standalone distribution with pinned Nuitka and
  publish both an Inno Setup installer and a portable ZIP.
- Pin native builds to stable Nuitka 4.2, which officially supports the Python
  3.14 runtime shipped by the current Ubuntu release.
- Bundle licensed Onboard EN/RU language models and all applicable runtime
  license texts in the Windows distribution.
- Add a real bidirectional Windows hook/`SendInput`/Tk correction E2E and make
  tag releases wait for both Debian and Windows artifacts before publishing
  unified checksums.
- Make silent installer tests verify that uninstall removes the application
  files and its per-user autostart registry value.
- Keep the platform-neutral Python contracts under the enhanced strict mypy
  profile and retain the 100% line/branch unit coverage gate.

## 0.2.1 — 2026-08-27

- Respect a manual XKB layout change by leaving the next completed word
  unchanged, with an opt-out switch in the autocorrection settings.

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

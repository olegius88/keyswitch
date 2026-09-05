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

[Documentation map](docs/README.md) ·
[Input troubleshooting](docs/troubleshooting.md) ·
[Verification, builds and releases](docs/verification.md) (guides in Russian)

## Features

- global input observation through `WH_KEYBOARD_LL` on Windows and XRecord in
  regular Linux X11 applications;
- automatic English and Russian word detection after a typing pause (1.5
  seconds by default, configurable), as well as after Space or
  punctuation; idle correction can be disabled independently in settings;
- early layout switching: as soon as the beginning of a word is impossible in
  the current language and clearly continues in the other one (for example
  `ghbd`), the layout is switched and the prefix rewritten without waiting for
  the end of the word; the minimum prefix length is configurable (4 by
  default) and the feature can be disabled. An active contextual assistant in
  `assist` mode suppresses prefix replacement; it is available in `off`/`shadow`
  or when the assistant is disabled or unavailable;
- precision-first hybrid detection using hard guards, frequency lexicons,
  Hunspell morphology, character n-grams, recent context and a lightweight
  first-party linear model;
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
- manual conversion with `Pause` of the unfinished word, of symbols typed
  after a word boundary (for example `"` meant as `@`), or of the last word;
  once anything else was typed after it, `Pause` only switches the layout;
- undo of the last correction for 10 seconds with `Ctrl+Alt+Z`;
- global pause with `Ctrl+Alt+P`;
- application exclusions selected from the active window, the installed
  application catalog, a Windows `.exe` picker or entered manually, plus word
  exclusions;
- local history containing only completed corrections;
- notifications, sound, light/dark themes and startup after OS login;
- a live `EN/RU` or country-flag layout indicator; either a left or right click
  opens a menu that selects the language opposite to the current one, alongside
  settings, pause, sound, notifications, history, exclusions, about and quit;
- a native full settings window with overview, test field, automatic
  correction, hotkeys, exclusions, history and backend diagnostics;
- scrollable settings pages; Windows marks changed settings with a color and
  offers individual reset buttons. Both platforms offer a full settings reset
  that preserves correction history and learned rules;
- single-instance protection: launching KeySwitch again activates the existing
  application window;
- automatic stable GitHub Release checks after startup and every six hours;
  Windows downloads the verified Setup EXE, installs it silently and restarts
  KeySwitch, while Ubuntu reports the release and opens its page for manual
  DEB installation.

KeySwitch does not record the complete keystroke stream. The current word and
up to 512 recent context characters are held in RAM. When history is enabled,
it stores correction pairs such as `ghbdtn → привет` with their timestamp,
application and score. Linear inference is fully local and model weights are
not updated from ordinary typing.

Technical logging also writes evaluated words and decision reasons, including
unchanged words. It may contain private text; enable it temporarily for diagnosis.

The contextual assistant is a separate local four-action classifier, not an
LLM: keep, convert, wait or suggest. It uses recent text and application
identity; for example, it can wait for the next word in `e 'njuj` → `у этого`.
`assist` is enabled by default; `shadow` leaves automatic decisions to the
baseline detector. Optional reading of existing field text through OS
accessibility is off by default. Quality is not established for real chats.
The expanded corpus exposed limitations of both the shipping model and a new
candidate; the candidate failed its quality conditions and **is not deployed**.
The shipping contextual weights remain those introduced in 0.15.0.
See [settings, training, privacy and limits](docs/context-assistant.md).

On Windows, Enter/Tab is intercepted before delivery: the word is corrected
first, then the action is sent exactly once. For example, `ghbdtn` + Enter
submits `привет` when settings and safety guards permit correction.
A learning prompt owns Enter exclusively while it is active.
Shift/Ctrl/Alt shortcuts are not intercepted. A correction failure or a focus
change cancels submission and is reported in diagnostics. On X11 these actions
have already reached the application, so use Space, idle correction or Pause
before submitting or changing fields.
See the [EN/RU input maturity matrix](docs/input-maturity.md) for regression
scenarios and platform limitations.

## Install on Windows

Download `KeySwitch-Setup-0.16.1-x64.exe` from the
[latest release](https://github.com/olegius88/keyswitch/releases/latest) and run
it. The per-user installation goes to `%LOCALAPPDATA%\Programs\KeySwitch` and
does not require administrator privileges. The release also includes the
portable `KeySwitch-0.16.1-windows-x64.zip` archive.

After launch, KeySwitch appears in the notification area. Left- or right-click
the `EN/RU` or flag icon to open its menu. Its Switch to action always offers
the language opposite to the current one. Startup, start minimized, indicator,
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
$env:KEYSWITCH_MODEL_PATH = "$PWD\model\intent_v1\sources"
.venv\Scripts\keyswitch
```

For the complete detection model when running from source, point
`KEYSWITCH_MODEL_PATH` to `model/intent_v1/sources`; it contains the exact
frozen `en_US.lm` and `ru_RU.lm` used for training. KeySwitch's own
`layout_intent_v1.ksm` is already part of its resources. The Setup EXE and
portable ZIP bundle the lexicons, KSLM and contextual model automatically.

## Quick start on Ubuntu

The currently verified environment is Ubuntu 26.04.1 LTS with XFCE, X11 and
the `us,ru` system layouts.

```bash
git clone https://github.com/olegius88/keyswitch.git
cd keyswitch
./run.sh
```

The application window includes a test field. Select EN, type `test `, then
`ghbdtn` and stop: the default idle check runs after about 1.5 seconds to
convert it to `привет` and select RU. Space requests an immediate check.
The initial `test ` completes the default one-word protection after a manual
language choice; that protection is intentional, not a lost event.
For the reverse direction, select RU, type `тест `, then `руддщ` (the physical
keys for `hello`). `Pause` requests manual conversion of the current word;
technical logging explains why automatic correction was skipped.

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

Download `keyswitch_0.16.1_amd64.deb` from the
[latest release](https://github.com/olegius88/keyswitch/releases/latest), then
install it with:

```bash
sudo apt install ./keyswitch_0.16.1_amd64.deb
```

The package installs the required system dependencies and adds KeySwitch to the
application menu. Nuitka compiles the application into an architecture-specific
ELF executable: `/usr/lib/keyswitch` contains no application `.py` sources or
`.pyc` bytecode, and the package does not depend on the system Python
interpreter. The native runtime includes `libpython`, so the `amd64` package
cannot be installed on a different architecture.

Release checking also works on Ubuntu, but KeySwitch does not silently install
a system DEB itself. When a new version is found, the app shows a notification
and an Open release button; installation remains an explicit APT action with
the necessary permissions. Installing a downloaded `.deb` needs no separate
KeySwitch repository; the app does not configure automatic APT updates.

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

This release uses the first two XKB groups. Default settings require
`layout: us,ru`: `detection.language_models` maps `["en_US", "ru_RU"]` by group
index. Reversing the layouts is not equivalent without matching language-model
configuration; the standard setup is `us,ru`.

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
- startup and optional technical-diagnostics log:
  `~/.local/share/keyswitch/keyswitch.log`;
- autostart entry: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

These are default paths. Linux honors `XDG_CONFIG_HOME`/`XDG_DATA_HOME`;
`KEYSWITCH_CONFIG_DIR` and `KEYSWITCH_DATA_DIR` override settings and data
directories on both platforms, for example for isolated testing.

`KeePassXC`, `1Password` and `Bitwarden` are excluded by default. A global
observer alone does not know field semantics. Optional accessibility reading
excludes recognized protected fields but is not a universal password detector.
Other
sensitive applications should be added by `.exe` name on Windows or by
`WM_CLASS` on Linux using the Exclusions settings page.

Detailed technical logging can be enabled on the Maintenance page on Windows
or the About page on Linux. It records the decision reason, scores for both
interpretations, the skip reason (`skipped_reason`) and the detector's shadow
verdict for protected or disabled cases, context, the source and age of a
manual layout change, whether a layout change was made by the engine itself
or by the user, deferred pause corrections with their cause, early-switch
events, the learning prompt lifecycle and the values of changed settings.
Because it may
contain typed words and application names, technical mode is disabled by
default. Engine diagnostic events replace excluded-application text with
`<redacted>`; this is not a general personal-data scrubber for all error messages.
The ordinary log uses 1 MiB per file and two backups; technical mode uses
5 MiB and five backups (`keyswitch.log.1` … `.5`), up to six files in total.
Enabling technical mode rotates a nonempty current file. Disabling it does
not immediately erase existing records or old backups. After reproducing the
issue, disable the mode, review files for private data and save the current log
and backups covering the incident. `context_decision` does not include the
surrounding phrase; `correction_applied` confirms event submission, not the
final text. See [troubleshooting](docs/troubleshooting.md).

The Local linear model switch on the automatic-correction page disables only
the baseline KSLM classifier; dictionaries, hard guards and explicitly learned
rules continue to work. The contextual assistant has a separate setting and
is not disabled by this switch. Diagnostics show the bundled KSLM version and abbreviated
SHA-256, or the reason for a safe fallback to the deterministic ensemble.
Neither words nor model features are sent over the network.

Confidence threshold and aggressive recognition configure the baseline
detector's fallback heuristics, not the trained models' fixed thresholds.
They can indirectly change the baseline decision seen by the assistant.
Minimum length is not a universal ban on short words: explicit rules,
reviewed short-word exceptions and contextual decisions can still apply.

The following describes the baseline detector without contextual intervention
in `off`/`shadow`. In `assist`, a separate model uses its lexical evidence and
decision; KSLM certification does not certify that contextual policy.

After user rules and the hard guards for valid source words, code and addresses,
KSLM is the sole statistical decision: only its calibrated trigger/direction
threshold is applied. Membership coverage and language-model scores remain
diagnostics and cannot veto a positive decision already certified at that
threshold. A negative result is not handed to the heuristic for a second
chance. The deterministic ensemble is used only for short tokens, a disabled
model, or a missing artifact.

The linear layer runs only when at least one of the two normalized
interpretations contains five or more characters. Shorter words remain with
the deterministic heuristics and user rules: this removes the most ambiguous
tail from probabilistic decisions without disabling correction as a whole.
The trainer also excludes deletion typos that fall below this limit; the same
limit is recorded in the hash-bound model policy and covered by tests.

The configured minimum length does not block a narrow reviewed exception list
for frequent two-letter function words; it currently contains Russian-layout
`ша` to English `if`. The exception requires an exact target-language lexicon
hit and at least a 100x target/source frequency ratio. An explicit manual
layout change has higher priority and protects the entire next word, including
pause correction and previously learned rules. Normal detection resumes after
the word boundary. Re-observing a layout the engine just selected within 1.5
seconds does not create new protection as an external manual change would.
An explicit menu language choice itself protects the next word, as does Pause
when no replaceable word remains. A change to another layout stays manual.
Real three- and four-character bilingual collision pairs from frozen Onboard
data are retained only in the safety corpus: they never train the classifier,
but prove the valid-source pre-model guard across every trigger.

`layout_intent_v1.ksm` is the first classifier generation stored in KSLM
schema 4. Alongside int16 weights, it holds sorted uint64 fingerprints for exact
character-feature membership in an independent hash namespace, not an
occupancy bitset of weight buckets. The loader checks the schema, shape,
ordering, CRC32 and SHA-256, bounds the complete file at 14 MiB, and safely
disables the layer after any validation failure. Within that limit, the
canonical embedded manifest is capped at 1 MiB, the payload at 12 MiB, and the
exact membership-fingerprint count at `2^20`.
The schema numbers are independent: the training config uses
`schema_version: 13`, the container uses KSLM schema 4, and the external
publication `manifest.json` uses `schema_version: 1`.

The offline model uses 2,097,152 hash buckets and permits at most 64 epochs with
deterministic early stopping. The frozen EN/RU lexicons are used in full after
filtering; `maximum_words_per_language` must be zero so global truncation before
the split cannot depend on held-out identities.
The offline trainer discovers the logical CPUs available through process
affinity and uses all of them by default for feature extraction and independent
scoring. `--workers N` caps the worker-process count, while `--workers 1`
enables single-process diagnostics. Worker output is reduced in original row
order, so the worker count cannot alter online-FTRL ordering or candidate
bytes. The online FTRL update itself remains sequential because every step
depends on the state produced by the preceding example.
Feature schema v5 is derived only from the source
and alternative raw tokens: signed character 1–5-grams, direction, length,
and trigger. Context fields, every `WordScore`, and every language-model score
field are ignored by the classifier. Real short-lived context remains in the
conservative detector heuristic and is not counted a second time in the linear
score. The trainer invokes the same runtime extractor with the same seeds and
n-gram orders, giving exact train/serve feature parity. A train-only EN/RU scorer
remains as separate, checked provenance but is not a classifier input.
Physical signatures are partitioned under
`keyswitch:intent-v20:physical-signature`. Before row generation, the independent
candidate phase quarantines every identity or typo signature owned by different
pre-sealed splits/languages, or overlapping a protected/safety token. Sealed-test
rows and their quarantine are built only after the exact candidate SHA has been
atomically claimed; the merge removes test signatures exposed by candidate
rows, quarantine or safety data and never changes candidate rows.

Schema 13 additionally consumes the byte-frozen
`unknown-typo-development-v20.json`. It was built model-blind before training
from 5,000 EN and 5,000 RU Hunspell-unknown typos and compacted to one record
per physical signature. An independent role namespace deterministically assigns
each language half as 3,500/500/500/500 words across
train/development/calibration/threshold; this corpus has no test role. On load,
the trainer verifies file size and SHA-256, both Hunspell dictionary provenance
records, layout-pair physical equivalence, uniqueness, exact role sizes, and
the SHA-256 of all 120,000 re-expanded symmetric rows.
Only the train role receives the config-bound `3.0` weight, equal to the
ordinary frequency-derived weight ceiling; all three evaluation roles retain
weight `1.0`. Critical unknown typos therefore remain influential during
optimization without silently relaxing the unweighted quality measurements.
The complete post-merge audit forbids role overlap and overlap with the
original lexical/safety corpus.
The independent v20 holdout uses distinct rank/choice namespaces and is never
used for training or threshold selection.

Calibration, threshold selection and sealed-test evaluation use neutral context
as the primary slice. Fixed non-empty label-independent context-stress profiles
additionally prove feature-schema-v5 context invariance: changing
`context_delta` or `context_group` must not change the vector, logit or decision,
and every profile passes the same per-trigger quality gates.

Before the sealed test is opened, separate EN→RU/RU→EN thresholds are selected
jointly for every trigger; both directional operating curves must contain both
labels in the overall and typo slices. The aggregate trigger slice must pass the
complete selection policy with neutral context: overall
precision at least 0.9995, recall at least 0.956 and specificity at least 0.999;
the typo slice uses the same strengthened selection precision of 0.9995,
recall at least 0.91 and
specificity at least 0.999. The 12 primary selection FPR checks (six triggers
times overall and typo slices) use a Bonferroni correction: the family-wise 95%
Wilson upper endpoint uses per-comparison confidence 0.9958333333333333 and a
pinned z-score of 2.8652602385321333 and may not exceed 0.001. Signed gate
evidence separately records the method, comparison count, confidence, z-score
and computed endpoint; the independent sealed test keeps the ordinary 95%
Wilson endpoint. Training config schema 13 additionally requires zero
selection false positives per trigger across both the overall and typo slices
and still requires the strengthened family-wise Wilson bound. This absolute
budget is checked before the sealed
test is materialized. After
directional selection, schema 13 deterministically chooses on the threshold
split the greatest common calibrated-logit margin that preserves the complete
selection policy. The margin is bounded by a config-bound 2.0 cap fixed before v11
from the model-blind unknown-typo development corpus. Under schema 13 those
frozen signatures have independent roles, so the effective value is selected
only on their threshold subset, recorded for every trigger, and threshold
metrics are recomputed against the hardened boundaries. The sealed test does
not participate in margin selection.
For `pause`, selection recall is 0.91, typo recall is 0.86, and the same
0.001 bound applies, and its logit threshold in
each direction is at least 0.5 above the strictest non-pause threshold in that
same direction. An infeasible selection policy stops training before sealed-test
evaluation. The final sealed test keeps independent minimum precision 0.999,
recall 0.95, typo recall 0.90, and pause recall 0.90/0.85. The precision
difference and one percentage point of recall form a pre-seal transfer reserve
without weakening the closed gate. Before the seal is claimed,
the safety audit and selection veto must also pass, and the trainer serializes
and reloads a trial KSLM through the runtime implementation to prove numeric
bounds, quantization parity, and payload/fingerprint limits. The safety gate
checks the actual production policy with runtime guards; direct model
predictions and membership coverage remain non-gating raw diagnostics.

The strict evaluator separately runs the real `LanguageDetector` under neutral
context and six reachable extrema of production context arithmetic (deltas from
-2.05 through +2.30) over sealed, sealed-typo, unknown-typo, safety, and
source-known slices. Every profile must preserve precision, specificity and the
Wilson FPR bound, have no more total false positives than either fallback or
neutral, and respect the fixed asymmetric recall policy. The finite
safety/source-known sets use the stronger exact-zero invariant: those rows must
not reach the model, while every unknown-typo row must reach it.

KSLM schema 4 stores independent monotonic Platt calibration for EN→RU and
RU→EN plus the exact `threshold_logit` for every trigger/direction pair. Both
calibrators are
fit only on the calibration split and correct a systematic inter-direction
score shift without changing the ordering within either direction. Runtime
selects the threshold by trigger and physical direction, then compares the
direction-calibrated logit directly with it; its sigmoid-derived
confidence is diagnostic only and does not participate in the decision. The
frozen external-evaluation policy pins sizes and SHA-256 digests of both
languages' Hunspell `.dic`/`.aff` files; expected SHA-256 digests of the
lexical-disjoint, unknown-typo development, and independent unknown-typo
holdout corpora; at least 5,000 words per language; and the canonical set of
all six triggers. The holdout uses distinct rank/choice namespaces and excludes
every sealed and development signature.
V11 passed its internal gates but was rejected before the independent external
holdout because the strict evaluator could not fail-closed index the new frozen
`hunspell-unknown-*` row family. `rejection-v11.json` preserves the cause and
exact hashes. V12 fixed the index but was rejected because the evaluator built
its base exclusion index after merging the development corpus and could not
reproduce its frozen provenance. V13 fixed that domain separation and reached
the independent holdout, but produced 4 false positives among 10,000 negatives
for each ordinary trigger; its 0.001028128 Wilson upper endpoint exceeded the
0.001 limit. `rejection-v12.json` and `rejection-v13.json` preserve the exact
causes and hashes. V14 fixed a zero-FP selection budget before rotating every
namespace and passed strict evaluation with 0 false positives among 60,000
unknown-typo negatives. V15 rotated every namespace again after the trainer
became multiprocess and passed strict evaluation on its new holdout with 12
false positives among 60,000 negatives (2 per trigger slice, Wilson upper
endpoint 0.000728996 against the 0.001 limit) and recall 0.96935. After the
FTRL loop moved into the native kernel, v16 and v17 failed the pre-sealed gate
(zero-false-positive recall on their threshold splits was 0.9479 and 0.9458
against the 0.956 minimum; no registry was claimed). V18 passed the internal
gates and the independent holdout (5 false positives among 60,000, recall
0.9425) but was rejected by the `fallback_regression` strict gate: one false
positive introduced by the model relative to the deterministic fallback on the
5,000-row sealed sample; `rejection-v18.json` records the decision. V19 failed the pre-sealed gate again
(recall 0.9463), while v20 passed every internal gate, the holdout (6 false
positives among 60,000, recall 0.9445) and all 30 strict gates and became the
current certified artifact.
`holdout-v20-preseal.json` pins its SHA-256, namespaces, sizes and zero overlap
before a v20 model is loaded or evaluated; `model_loaded=false` and
`metrics_evaluated=false` make that phase explicit.

The manifest binds hashes of the config, frozen sources, trainer, external
evaluator, preseal generator/receipt, development-corpus freezer, intent
runtime, layouts, language scorer, detector, frozen hard-negative source and
protected-token list to the
Python/platform identity, candidate/full datasets, both quarantines, excluded
test signatures and train-only scorer provenance. The receipt binds the exact
KSLM payload/runtime parameters and is published from an fsynced temporary file
through an atomic no-replace hard link. Until that receipt is valid, the
evaluator neither builds sealed-test rows nor prints their metrics, regardless
of the `--strict` flag.
After successful gates, pre-written and synced report, artifact and manifest
files are published in that order; the manifest is the final commit marker and
a process error rolls back destinations already replaced. The exact two-run
procedure for byte-comparing all three outputs is in the
[model card](model/intent_v1/MODEL_CARD.en.md#reproducibility).
The repository also contains a Russian
[release runbook](docs/intent-model-runbook.md) and a copy-paste
[training cookbook](docs/intent-model-cookbook.md) covering preseal, training,
strict evaluation, replay, and packaging.

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
coverage drops below 100% within [.coveragerc](.coveragerc). Native Win32
wrappers and the Windows UI are excluded from this metric and have separate
Windows tests; it is not 100% coverage of every platform file.

Contextual models have separate checks, not covered by a KSLM report:

```bash
PYTHONPATH=src python3 tools/verify_context_model.py
PYTHONPATH=src python3 tools/verify_context_v2.py
```

These are fast artifact/provenance checks. Training replay, final-text
evaluation, AT-SPI E2E and their environment requirements are documented in
the [verification guide](docs/verification.md).

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
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
PYTHONPATH=src tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
PYTHONPATH=src tools/evaluate_intent_model.py --strict
```

The second command validates bundled `layout_intent_v1.ksm`, KSLM schema 4,
checksums, frozen-source and toolchain provenance, the protected-token hash,
quarantine and train-only scorer, non-vacuous Wilson gates, production guarded
safety, and separate precision-first thresholds for idle and completed-word
decisions. Raw-model safety and membership slices remain diagnostic. This
lexical-synthetic set is a regression gate, not an estimate of quality on real
user input.

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
package="dist/keyswitch_0.16.1_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
```

The build runs strict KSLM evaluation and both fast contextual checks first.
A verified strict report can be reused through `KEYSWITCH_INTENT_STRICT_REPORT`,
as described in the [build guide](docs/verification.md).
The DEB bundles KSLM, the contextual model and the exact frozen `en_US.lm`/`ru_RU.lm`
from `model/intent_v1/sources`. The in-package directory takes precedence over
system `onboard-data`, so runtime n-gram scores use the release-tested snapshot;
the verifier requires both files to be byte-identical.

Build the native Windows artifacts on Windows with Python, Nuitka and Inno
Setup:

```powershell
./packaging/build-windows.ps1
```

This produces a standalone `KeySwitch.exe` without application `.py` files, a
portable ZIP and a per-user Setup EXE. In a fresh clone the command defaults to
the frozen `en_US.lm`, `ru_RU.lm` and license in `model/intent_v1/sources`;
optional `-ModelDirectory` and `-ModelLicense` overrides accept only staged
copies with the exact config-bound paths, sizes and SHA-256 digests;
`SHA256SUMS` must also match config byte for byte. All external JSON/KSLM
inputs are read with explicit bounds. Before Nuitka a portable provenance pass
rebuilds both dataset phases and validates the seal registry, frozen sources,
toolchain, artifact and complete embedded manifest; a separate check then
validates the full threshold/sealed/context/typo/safety/veto quality matrix.
After Nuitka it runs the
finished `KeySwitch.exe --diagnose` without a model override and requires
`intent_model.available=true` with the expected version and SHA-256.

After stopping any already running instance, test the executable extracted
from the package in an active X11 session with:

```bash
dbus-run-session -- ./tools/run-native-e2e.sh "$package"
```

It verifies six real corrections, a manually selected layout left unchanged
for one word, XKB group changes, history, StatusNotifierItem registration and
the popup DBusMenu contents. Pushing a `v*` tag makes GitHub Actions validate
Linux and Windows independently, build the DEB, Windows Setup EXE and portable
ZIP, silently install it, diagnose the exact bundled model, smoke-test the
installed UI, create one
`SHA256SUMS` file and publish all artifacts in a GitHub Release.

The baseline-model/application Linux pipeline — KSLM provenance and strict
evaluation, its replay evidence, type checking, coverage, detector gates, X11/tray E2E, the DEB
build, its verifier and the packaged-binary E2E — runs as one process detached
from the terminal:

```bash
python3 tools/release_pipeline.py start --profile release
python3 tools/release_pipeline.py status
python3 tools/release_pipeline.py wait
```

Phases form a dependency graph and run concurrently; the scheduler admits the
next phase only when enough RAM stays available after the peaks that already
running phases may still reach and a reserve (`--jobs`,
`--memory-reserve-mib`). Every run lives in
`dist/release-pipeline/<stamp>-<profile>/`: `state.json` is updated while the
run proceeds, `summary.json` and `SUMMARY.md` appear at the end next to the
phase logs, strict reports and the DEB. `quick` checks the environment, KSLM
inputs, typing, coverage, detector and release metadata; `app` adds strict
KSLM, X11/tray and native DEB tests; `release` adds KSLM/corpus replay.
No profile directly runs contextual-model retraining, contextual engine replay
or native AT-SPI E2E; CI runs these separately. The local pipeline is therefore
not equivalent to all CI checks. `--replay-dir`
adopts replays that are still running or already finished, and
`python3 tools/release_pipeline.py phases` prints the phases, memory budgets
and dependencies.

Release publication is a separate command, **only after an explicit release
decision**. It commits and pushes all working-tree changes, so review
`git status` and the diff first. The script updates a fixed list of versioned
files, closes the `Unreleased` section of `CHANGELOG.md`, checks
`RELEASE_NOTES.md`, runs the verification contour, commits, tags, pushes and
waits until the workflow publishes the DEB, the Windows Setup EXE, the ZIP and
`SHA256SUMS`:

```bash
python3 tools/release.py --version X.Y.Z --dry-run  # replace X.Y.Z; no writes
python3 tools/release.py --version X.Y.Z            # commit, tag, push, publish
```

`X.Y.Z` means an unused version. The author must prepare `## Unreleased`
entries (or an already closed section for that version) and matching release
notes with the asset names. `--dry-run` checks preparation but runs no tests
or builds. A retry **does not automatically resume at the last failed phase**:
verification normally runs again; an existing tag is accepted only at the
same clean HEAD. Pushed tags or published releases are not rolled back.
See [release and recovery procedures](docs/verification.md).

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
- The Windows 0.16.1 Setup EXE is not yet signed with a publisher certificate.

## License

KeySwitch is distributed under the
[GNU General Public License 3.0 or later](LICENSE).
The bundled model is built from a frozen snapshot of Onboard `models/*` from
`onboard-data` `1.4.3+git20260213+ds-2`. Its original declaration records
GPL-3+ and credits marmuta (2013, 2014) and Francesco Fumanti (2011, 2012).
The exact `.lm` files, `SHA256SUMS` and unchanged `COPYRIGHT.onboard-data` live
under `model/intent_v1/sources`; the copyright file is included in both the
Debian and Windows license bundles.

The separate expanded-context research corpus uses Tatoeba's public CC0
export. [Source provenance and numeric-cache licensing](model/context_v2/sources/README.md)
are documented separately; they do not replace the Onboard or project license.
This research corpus is not bundled as a runtime dictionary.

## Primary specifications

- [Google Research: FTRL-Proximal](https://research.google/pubs/ad-click-prediction-a-view-from-the-trenches/)
  — the sparse linear model's offline training algorithm;
- [AISTATS/PMLR: Beta calibration](https://proceedings.mlr.press/v54/kull17a.html)
  — primary work on an alternative binary calibration method; Layout Intent v1
  uses a simpler Platt transform fitted on a separate split;
- [ACL Anthology: character n-grams for short segments](https://aclanthology.org/L10-1193/)
  — research on n-gram identification of short text;
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

# Changelog

All notable changes to KeySwitch are documented in this file.

## Unreleased

## 0.11.0 — 2026-09-04

- Every page of the Windows settings window scrolls. A page taller than the
  window carries its own scrollbar and answers the wheel and PageUp/PageDown,
  instead of hiding the settings below the fold; the wheel over a number field
  or a drop-down scrolls the page rather than silently changing the value.
  Descriptions wrap to the width the column really has, so the pages survive a
  narrow window too.
- A setting that differs from its shipped default is marked the way an editor
  marks an edited line: an accent bar beside it, an accented title and a
  "Сброс" button that restores that one value. The gutters holding them are
  reserved, so nothing moves as the markers appear. Setting rows also share
  the page background now instead of the grey band the platform theme drew
  behind them.
- A digit or a symbol typed inside a word no longer throws the word away.
  `зь2` used to leave an empty buffer, so `Pause` had nothing to convert and
  only switched the layout; it now becomes `pm2`. Automatic correction is
  unchanged — the detector still treats a token carrying a digit as code and
  never touches it on its own.
- Whenever the engine does drop an unfinished word — a shortcut, a caret move,
  a layout change mid-word, a focus change — the technical log records
  `word_discarded` with the reason, so a correction that never happened can be
  explained from the log instead of guessed at.
- The log file is installed on the root logger directly instead of through
  `logging.basicConfig`, which does nothing at all once anything else has
  configured logging first — and does it silently, leaving an empty
  `keyswitch.log` for a whole session. The maintenance page now states whether
  the journal is really being written and how large it is, and the same state
  is part of the diagnostics report.
- Every line of the technical log carries the version that wrote it, and each
  session opens with a banner naming the version, the platform and the
  rotation budget. A rotated file that outlives an update can no longer be
  read as if it came from the version installed now.

## 0.10.0 — 2026-09-04

- The log file rotates by the mode it is in: ordinary operation keeps 1 MB in
  3 files, and the diagnostics ("developer") log, which writes a line per
  evaluated word, keeps 5 MB in 6 files. Turning the diagnostics mode on now
  starts a fresh file, so the log attached to a report contains that session
  only; turning it off keeps what was recorded. Both platforms share one
  `keyswitch.logsetup` module instead of duplicating the handler.
- A button on the diagnostics page opens the folder that holds the log, in the
  file manager on Linux and in Explorer on Windows. The page also states the
  rotation budget.
- `tools/release.py` performs a release in one command: it propagates the
  version to every file that spells it, closes the changelog section, checks
  the release notes, runs the verification contour, commits, tags, pushes, and
  waits for the workflow and the published packages. Every step recognises the
  work it has already done, so a re-run after a failure continues where it
  stopped; anything that does not hold stops the run with a message naming the
  file to fix.

## 0.9.1 — 2026-09-03

- Windows keeps a keyboard layout per window, so the layout that arrived
  together with another window (Telegram → TeamViewer, an IDE → the browser)
  was logged as a manual switch and protected the next word from
  autocorrection. The engine now tracks the focused window and treats such a
  change as that window's own layout; a layout the engine selects itself also
  drops an older manual pick instead of reviving it minutes later.
- Windows of KeySwitch itself (settings, the learning prompt) never count as
  a layout change. The Windows learning prompt is shown as a non-activating
  popup: it no longer takes the caret away from the editor or swallows the
  next typed key. Enter and Esc still answer it through the global hook, and
  because the editor keeps the focus they now also reach the editor itself.
- Moving to another window drops the unfinished word and marks the last
  committed one stale, so a typing pause or `Pause` never rewrites text in
  the wrong window.
- Learning is offered only for something that reads as a word (at least two
  letters): a lone letter converted to punctuation (`б` → `,`) is neither
  offered nor counted towards an automatic rule.
- The undo hotkey pressed while an early-switched word is still being typed
  reverts that prefix instead of the previous correction and leaves the rest
  of the word alone (also with manual layout respect off). An early switch
  abandoned without a boundary (navigation key, another window) is still
  recorded in the history and can be undone.
- Early switching accepts prefixes typed on punctuation keys that are letters
  in the other layout (`nt,z` → `тебя`).
- Technical log: layout observations carry `focus_changed`; new events
  `focus_changed`, `layout_change_ignored` and `early_switch_undo_scheduled`;
  `manual_conversion_scheduled` reports `learnable`.

## 0.9.0 — 2026-09-03

- `Pause` converts only what was typed after the last word boundary: the
  unfinished word, layout-dependent symbols such as the Russian quote meant as
  `@`, or the last completed word while nothing else was typed after it;
  otherwise the key only switches the layout and protects the next word.
- Early layout switching: once a prefix (4 letters by default, configurable
  3–8) has no continuation in the current language and clearly continues in
  the other one, the layout is switched and the prefix rewritten as soon as
  the last letter's key is released (letters pressed before that release are
  absorbed). A letter that arrives in the old layout within 0.5 s is converted
  on its own;
  the finished word is recorded as one correction at the boundary.
  `detection.early_switch` and `detection.early_switch_min_length` settings,
  rows in both settings UIs and a slowly typed X11 E2E case.
- Configurable typing pause (`detection.pause_delay_seconds`, 0.3–5 s, default
  1.5). The engine wakes exactly when the delay elapses instead of on a 0.5 s
  grid, and key presses without a release older than 3 s no longer block pause
  correction forever.
- A change to the layout the engine itself just selected, observed within
  1.5 s, is no longer treated as a manual switch, so a correction, `Pause` or
  the menu action does not protect the following word by mistake; switching
  away from it stays manual. Only key presses (not releases) update the
  observed layout.
- Technical log: `word_evaluation` now carries `skipped_reason`, protection
  details, the context that was used, `idle_ms` and a shadow detector verdict
  for protected or disabled words; `correction_applied` records the mode,
  deleted characters, previous layout and injection time; new events for
  deferred pause corrections, pruned stale presses, early-switch evaluations,
  late strokes, layout switches without a word, learning prompt lifecycle and
  attributed layout changes; `setting_changed` includes scalar values.

## 0.8.0 — 2026-09-02

- Accept the v20 layout-intent candidate `intent-v1-6ece07f881ec`, the first
  certified under the compiled FTRL kernel and the multi-process evaluator.
  The trainer change altered the toolchain identity behind v15, so the split,
  registry, hard-negative source and holdout namespaces were rotated per
  candidate: v16, v17 and v19 failed the pre-sealed gate (zero-false-positive
  recall on their threshold splits 0.9479, 0.9458 and 0.9463 against the 0.956
  minimum; no registry was claimed), v18 passed its internal gates and holdout
  but was rejected by the `fallback_regression` strict gate (one model-introduced
  false positive on the 5,000-row sealed sample; `rejection-v18.json`), and v20
  passed everything. Freeze `unknown-typo-development-v20.json` (SHA-256
  `61e02546fb05c2502b2535c512b0e11fad13042d25b1f4f70cff621a4e35686f`), preseal
  `holdout-v20-preseal.json` with zero overlap against 288,869 sealed and
  10,000 development signatures, select epoch 45 of 49 on development with
  765,166 nonzero weights and 1,029,480 membership fingerprints. The
  independent strict report, SHA-256
  `01cc92bfc293019377019ecdcd965af11a61ac3da8cdf3837915459bf9f1d525`,
  passed all 30 gates: on the 60,000-negative model-blind holdout the ensemble
  produced 6 false positives (1 per trigger slice, Wilson upper endpoint
  0.000566269 against the 0.001 limit, none introduced by the model, 42
  fallback false positives prevented) with recall 0.944483; every ordinary
  sealed trigger shows 1 false positive among 21,338 negatives with recall
  0.954539 and Pause shows none with recall 0.946712. The published artifact
  has SHA-256
  `85deddb83e041f52622b794cf919770994d71a9f1c50af482be4f6574c4163cd`; the
  manifest and test-report SHA-256 values are
  `9c39b615ba90b94107be6bef0140ce9387e493bb6aae195f4a8d116021283da9` and
  `f3c44b42c96ce654042d17c822d92bd3202a9d1b12d6b28e34e394531a10fa94`. Two
  independent sequential retraining runs reproduced byte-identical KSLM,
  manifest and test-report files, and the strict evaluator re-run against the
  first replay passed all 30 gates with report SHA-256
  `5e77f44b857c9096cc306ce4de3232f81037df932d1d3b5c8ca01de8082404fc`.
- Add `tools/write_intent_rejection.py`, which writes `rejection-vN.json`
  from the manifest, registry and strict report so a rejected candidate's
  receipt cannot disagree with its evidence.
- Score rows on worker processes in the strict intent-model evaluator
  (`--workers`, default every available CPU). Model predictions, fallback
  comparisons and production-context profiles are pure per-row functions, so
  results are identical in any worker count; the context-invariant prediction
  cache is replayed in the parent from the recorded calls so its reported
  counters stay exactly those of a sequential run.
- Precompute the US/RU layout translation tables once and translate through
  `str.translate`. The mapping is unchanged character for character, but the
  runtime, trainer audits and evaluator no longer rebuild the dictionary on
  every call (about 20x faster per translation).
- Run the sequential FTRL-Proximal epochs of the layout-intent trainer in a
  compiled kernel. The C source is embedded in `tools/train_intent_model.py`,
  compiled with fused multiply-add disabled and called through `ctypes`; it
  mirrors `FTRLProximal.update` expression by expression, including the
  compensated float `sum()` of CPython 3.12+, and must reproduce the Python
  reference bit for bit on the first 4,096 real rows before the first epoch.
  `--ftrl-kernel auto|native|python` selects the loop; the bytes are identical
  and an epoch takes seconds instead of a minute.
- Let `packaging/build-deb.sh` reuse a strict intent-model report produced
  earlier in the same release contour instead of repeating the half-hour
  evaluation. `tools/verify_intent_strict_report.py` accepts the report only
  when every gate passed and every recorded hash of the artifact, config,
  frozen sources, model toolchain and preseal receipt still equals the current
  file; CI, the release pipeline and packaging share that single check.
- Run the Windows release job concurrently with the Linux job instead of
  after it, and make the release pipeline's `build-deb` phase depend on
  `model-strict` so one strict evaluation serves both.

## 0.7.0 — 2026-09-02

- Accept the v15 layout-intent candidate `intent-v1-bec1f1d3dceb` after the
  multiprocess trainer changed the toolchain identity behind v14. Rotate the
  split, registry, hard-negative source and holdout namespaces to v15, freeze
  the model-blind `unknown-typo-development-v15.json` (SHA-256
  `a0585bdbd21526434fc77effc64200075269d884321a702fa44bd8a9dc7f963c`) and
  preseal `holdout-v15-preseal.json` with zero overlap against 288,869 sealed
  and 10,000 development physical signatures. The trainer ran all 64 permitted
  epochs and selected epoch 64 with 765,205 nonzero weights and 1,031,416
  membership fingerprints. The independent strict report, SHA-256
  `82ff2b6f332369eea2e71eb2df4960a554a1aa9de9c31025c35bf15d4485c303`,
  passed all 30 gates: on the 60,000-negative model-blind holdout the
  production ensemble produced 12 false positives (2 per trigger slice, Wilson
  upper endpoint 0.000728996 against the 0.001 limit; 6 introduced by the model
  while 60 fallback false positives were prevented) with recall 0.96935, and
  every sealed per-trigger slice shows 2 false positives among 21,574
  negatives with recall 0.973 (Pause 0.968). The published artifact has SHA-256
  `7631b821bafc958364353a8a13de3abc23e922e51b589bd181075db55fa9e9dc`; the
  manifest and test-report SHA-256 values are
  `e0070e8e6813da4a8dde1a09eb2c1713f033d002a64216299cba3764032d82f7` and
  `05caf3828ff2724fc5f1d22ff2e28d9b31cd2d1bcfcceb64f934bb8bfe84480d`. Two
  independent sequential retraining runs reproduced byte-identical KSLM,
  manifest and test-report files, and the strict evaluator re-run against the
  first replay passed all 30 gates with report SHA-256
  `88f6704c845efd86b8c3ba924607bcdf972e915d0d7d6c75ff147e1d0099f23e`.
- Add `tools/release_pipeline.py`, an unattended Linux release pipeline that
  runs model provenance and reproducibility evidence, strict typing, coverage,
  detector gates, X11/tray E2E, the native Debian build, its verifier and the
  packaged E2E as one detached process. Phases form a dependency graph and run
  concurrently under a memory-aware scheduler; every run leaves a live
  `state.json` plus final `summary.json`/`SUMMARY.md` with the runbook
  checklist for later review.
- Pin the v15 registry, preseal receipt and frozen development source in
  `.gitattributes`, describe the v15 contract in the man page and pin
  `intent-v1-bec1f1d3dceb` in the Windows packaging contract test.
- Use every CPU available through process affinity by default while training
  the layout-intent model. Parallelize deterministic feature extraction,
  frozen-model epoch evaluation, calibration, threshold and sealed-test
  scoring; preserve canonical row order and provide `--workers N` for resource
  limits or single-process diagnostics.
- Preserve the maximized or snapped state of the active Windows application
  when the learning prompt closes. Return keyboard focus without issuing
  `SW_RESTORE`, and cover the maximized-window confirmation path in Win32 E2E.

## 0.6.1 — 2026-09-02

- Correct the reviewed Russian-layout `ша` to English `if` case independently
  of the configured minimum word length. Require an exact target lexicon hit
  and a 100x frequency advantage; keep other two-letter collisions behind the
  normal minimum-length guard to avoid new false positives.
- Give an explicit manual layout change absolute priority for the next complete
  word, including pause correction and previously learned automatic rules.
- Add opt-in structured technical decision logging to the Linux and Windows
  settings. Include model/session metadata, score and correction events, redact
  words from excluded applications, and rotate the 5 MiB log with three
  backups.

## 0.6.0 — 2026-09-01

- Accept the v14 layout-intent candidate after all 30 strict gates passed.
  Freeze training config schema 13, split/registry/source/holdout namespaces
  v14, a zero false-positive selection budget, and overall selection recall
  0.956. The independent model-blind holdout is disjoint from 288,891 base
  sealed and 10,000 development physical signatures. On 60,000 unknown-typo
  negatives the production ensemble introduced zero false positives; ordinary
  trigger recall is 0.95 and Pause recall is 0.9425. The complete strict report
  has SHA-256 `c4e72a3290801dab22236a2bc381f7ba97b99f0745a078dd410e973d55d8bf52`.
  The published `intent-v1-6bf96537c28f` artifact has SHA-256
  `b22706d95e6ac942e39cd16006f4ce9c4508d98566271f202d5855d528cf1b16`.
  Two independent full retraining runs reproduced byte-identical KSLM,
  manifest and test-report files; the manifest/report SHA-256 values are
  `33b55674f6454c0b45ad55fc704b9113b5e934404ea69577ce8d7949518d2822`
  and `84b01b68e9fa186f1791828520d1b4319b39d9e6ab261a5210844cd68ba4f01e`.
- Preserve two additional non-reusable audit decisions. V12 never evaluated
  external metrics because the evaluator constructed its base exclusion index
  after merging development and could not reproduce frozen development
  provenance; `rejection-v12.json` records the exact hashes. V13 fixed domain
  separation and evaluated the fresh holdout, but ordinary triggers produced
  4 false positives among 10,000 negatives and a 0.001028128 Wilson upper
  endpoint above the 0.001 limit; `rejection-v13.json` records the decision.
- Reject the internally passing v11 artifact after the signed strict evaluator
  failed before independent external-holdout scoring: its sealed-signature
  exclusion index did not support the frozen `hunspell-unknown-*` row family.
  Preserve exact candidate, artifact, registry, manifest, report and evaluator
  hashes in `rejection-v11.json`; validate those rows against their rendered
  physical signature instead of bypassing the proof. Rotate to training config
  schema 11, split/registry/holdout v12, and a new model-blind frozen
  `unknown-typo-development-v12.json`. The v12 preseal receipt fixes 288,875
  sealed plus 10,000 development exclusions with zero holdout overlap before
  any model is loaded or evaluated.
- Enforce `selection_maximum_false_positives_per_trigger` as one aggregate
  budget jointly allocated across EN-to-RU and RU-to-EN operating curves. Add a
  regression where each direction would individually spend one false positive
  and prove that the combined selection still spends at most one.
- Rotate the signed intent candidate to v11 after preserving the exact v10
  rejection. V10 deterministically selected a common calibrated-logit margin
  of 0.9938225471937638 and passed pre-seal, but independent sealed non-pause
  recall was 0.944410276 against the 0.95 minimum; its revealed sealed rows are
  not reused for tuning. Freeze the already model-blind unknown-typo
  development corpus as `unknown-typo-development-v11.json`, bind its bytes,
  expanded-corpus and physical-signature SHA-256 values plus Hunspell
  provenance, and partition each language's 5,000 signatures under a separate
  role namespace as 3,500/500/500/500 words across
  train/development/calibration/threshold with no test role. Re-expand and
  audit all 120,000 symmetric rows, rotate the split namespace and registry,
  and preseal a model-blind v11 holdout with zero overlap against 288,902
  sealed and 10,000 development signatures.
  Serialise
  KSLM decoding behind an exact cyclic-GC guard, restore the caller's GC state
  on every path, and retain the complete strict evaluator JSON beside native
  Debian build artifacts. Remove the post-model membership-coverage and
  target-language-score vetoes that reduced certified raw recall: after hard
  guards, the calibrated trigger/direction threshold is now the only
  statistical decision, while coverage and language scores remain diagnostics.

- Add a dynamic tray-menu language action on Linux and Windows: when `RU` is
  active it offers English, and when `EN` is active it offers Russian.
- Add an optional local linear intent model on top of the precision-first
  detector: context-invariant feature schema v5 with raw-token-only signed
  character 1–5-grams, direction, length and trigger, FTRL-Proximal offline training,
  independent EN→RU/RU→EN Platt calibration on a dedicated split, jointly
  selected exact calibrated-logit thresholds for every trigger and physical
  direction, and a checksum-validated bundled int16
  KSLM schema 4 artifact
  with sorted uint64 feature-membership fingerprints and deterministic
  fallback.
- Make KSLM the sole statistical decision after user, structural and
  source-known hard guards. Use the heuristic ensemble only for short tokens,
  a disabled model or a missing artifact, so a negative model decision cannot
  be bypassed by a second positive rule and a positive calibrated decision
  cannot be weakened by an unsigned secondary veto.
- Freeze the exact Onboard EN/RU training sources and original copyright
  evidence in the repository, verify their SHA-256 in training, CI and native
  packaging, and keep the complete KSLM container bounded to 14 MiB, its
  embedded manifest to 1 MiB, its payload to 12 MiB and exact membership to
  `2^20` fingerprints.
- Gate the bundled model in CI, native Debian and Windows packaging, expose its
  version/checksum or fallback state in diagnostics, and keep ordinary input
  offline and outside model training.
- Bundle the exact frozen EN/RU Onboard language models in the native Debian
  distribution as well as Windows, prefer them over mutable system models at
  runtime, and verify both installed files byte for byte.
- Harden Windows packaging with bounded artifact/config/manifest reads, exact
  config and `SHA256SUMS` binding for frozen or staged sources, a portable
  sealed-provenance replay before Nuitka, and exact model diagnosis both before
  archiving and after silent installation.
- Harden offline training with a 2,097,152-bucket vector, up to 64 deterministic
  FTRL epochs and exact train/serve feature parity. Feature schema v5 ignores
  context, every `WordScore` and every language-model field; short-lived
  context remains only in the deterministic fallback and is not counted twice
  by the classifier. Train-only character scorers are retained only as
  checked provenance and are never classifier inputs. Use the
  `keyswitch:intent-v12:physical-signature` split namespace and quarantine
  physical signatures across splits, languages and protected safety rows.
- Restrict the optional probabilistic layer to normalized interpretations of at
  least five characters, keep shorter words on deterministic/user-rule paths,
  and enforce the identical post-augmentation limit in training and external
  unknown-typo evaluation. Record the applicability contract in signed model
  policy metadata.
- Preserve real three- and four-character bilingual Onboard collisions as a
  safety-only corpus, proving valid-source pre-model guards for every trigger
  without letting those ambiguous short tokens train the classifier.
- Select the retained epoch only on a development high-precision operating
  curve, prioritizing the full precision/recall/specificity/family-wise
  Wilson-FPR policy
  and using mean log loss only as the final tie-breaker; keep threshold data
  independent from optimization.
- Allocate the immutable 40-bucket namespace as 65/10/10/7.5/7.5 percent for
  train/development/calibration/threshold/test. A rejected 70% training
  experiment both regressed development recall and exceeded the exact-membership
  payload budget, so it is not part of the release design.
- Exclude every pre-seal candidate-quarantine signature from the separately
  built sealed test and validate candidate/test quarantine ownership in its
  original phase. Retain the consumed v2, v3 and v4 seals as rejected-run audit
  evidence. The v3 candidate passed pre-seal and the ordinary sealed slice, but
  its non-pause typo slice produced 10 false positives in 17,392 negatives
  (95% Wilson upper FPR 0.001058171), so no model was published. Rotate the
  strengthened protocol to the v4 namespace instead of reusing that test. The
  v4 candidate then passed selection with 9/23,067 ordinary and 8/17,220 typo
  false positives, but the independent sealed slice produced 14/23,090 and
  13/17,223 respectively (ordinary 95% Wilson upper FPR 0.001017564 and
  0.001291083), so it was also rejected without publication. The v5 candidate
  passed internal sealed gates, but the serving ensemble suppressed too much
  model recall and the first external protocol incorrectly treated flagged
  Hunspell stems as necessarily source-known. Keep the inspected v5 external
  corpus as development-only evidence, fix the runtime policy and methodology,
  and rotate both the internal split/seal and unseen external holdout to v6.
  Candidate v6 passed the ordinary and Pause sealed slices, but its non-pause
  typo slice produced 9 false positives among 15,812 negatives (upper 95%
  Wilson FPR 0.001081498 versus the 0.001 limit), so it was rejected without
  publication. Preserve `rejection-v6.json`, rotate to v7, and never reuse the
  revealed v6 test.
- Make pre-sealed threshold selection non-vacuous and complete: overall and
  typo precision floors of 0.9995, recall floors of 0.95/0.90, specificity
  0.999, and a 0.001 maximum family-wise 95% Wilson FPR upper bound for both.
  Correct the 12 primary selection comparisons (six triggers times overall and
  typo slices) with Bonferroni: per-comparison confidence
  0.9958333333333333 and pinned z-score 2.8652602385321333. Signed gate evidence
  records the correction, comparison count, confidence, z-score and endpoint;
  the independent sealed test keeps the ordinary 95% Wilson endpoint. Keep the
  independent sealed precision floor at 0.999 to provide explicit transfer
  headroom without weakening the closed gate.
  Training config schema 11 additionally enforces a maximum-one false-positive
  budget on every trigger's overall and typo selection slices before the
  sealed test can be materialized; this is at most 0.0045% of 22,552 negatives
  and remains subject to the strengthened family-wise Wilson bound.
  Require 0.96/0.91 overall/typo selection recall and 0.91/0.86 for Pause,
  preserving a one-percentage-point transfer reserve above the sealed floors.
  Require pause recall/typo recall of 0.90/0.85, the same Wilson bound and a 0.5
  logit margin over the strictest non-pause threshold; stop before sealed-test
  evaluation if selection is infeasible. Gate the guarded production detector
  on safety cases while retaining direct model and membership results only as
  raw diagnostics.
- Extend the pre-sealed gate to safety and selection-veto evidence, reject
  global lexicon truncation, and serialize/reload a trial KSLM before claiming
  the test namespace so numeric, quantization and payload-limit failures cannot
  consume the seal.
- Exercise the real production detector under neutral context and six reachable
  context extrema across sealed, typo, external unknown-typo, safety and
  source-known slices. Require no more total false positives than fallback or
  neutral, absolute precision/specificity/Wilson-FPR bounds, guarded-row
  reachability, and the fixed asymmetric recall policy. Use an exact-zero
  invariant for the finite safety/source-known adversarial sets instead of an
  underpowered Wilson interval.
- Store and compare the exact calibrated-logit threshold at runtime; expose its
  sigmoid-derived confidence only as diagnostics, avoiding saturated-probability
  comparisons. Certify exact classifier context invariance with fixed,
  label-independent context-stress profiles and the same per-trigger quality
  gates on threshold and sealed-test splits.
- Freeze external evaluation schema 2 to exact Hunspell dictionary/affix sizes
  and SHA-256 digests, expected lexical, unknown-typo-development and unseen
  unknown-typo-holdout corpus digests, at least 5,000 words per language, and
  the canonical list of all six runtime triggers. Build the v7 holdout without
  loading a model, under separate rank/choice namespaces, and prove zero
  overlap with 288,862 sealed and 10,000 development physical signatures.
- Avoid resorting and revalidating nearly one million already-validated uint64
  fingerprints inside the immutable KSLM constructor; cold loading of the
  current 10.4 MB artifact drops from roughly 0.7-1.0 s to 0.27-0.33 s while
  retaining payload ordering, uniqueness, checksum and corruption checks.
- Keep training config schema 11, KSLM schema 4 and external publication
  manifest schema 1 explicitly independent.
- Bind build provenance to the complete toolchain and protected-token hashes,
  candidate/full datasets, both quarantines, excluded test signatures,
  train-only scorer and Python/platform identity. Publish
  report, artifact and the final manifest commit marker as a rollback-capable
  bundle, and document a two-run byte-identical retraining procedure.
- Build candidate and sealed-test datasets in separate phases, bind the seal to
  the exact KSLM payload/runtime parameters, publish the registry atomically
  without replacement, and stop the evaluator before test construction or
  metric disclosure when provenance is invalid.

## 0.5.0 — 2026-08-28

- Add stable GitHub Release checks at startup and every six hours on Windows
  and Ubuntu, with a complete updates settings/status page on both platforms.
- On Windows, download the exact versioned Setup EXE, require its GitHub asset
  size and SHA-256 digest to match, install silently, and relaunch KeySwitch.
- Keep Ubuntu package replacement under APT authorization: notify about a new
  release and open its verified repository page instead of silently invoking
  elevated package installation.
- Cover update metadata validation, redirect restrictions, corrupted and
  interrupted downloads, state transitions and installer arguments with the
  mandatory 100% line/branch test gate.

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
- Install the AT-SPI bus runtime on Linux and fall back safely when the desktop
  accessibility service is disabled or unavailable.
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

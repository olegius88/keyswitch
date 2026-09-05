# Automatic layout detection research

Sources were verified on 28 August 2026.

The external-product comparison below is a historical review as of that date,
not a fresh check of current releases. The KSLM description covers the baseline
detector. Since 0.15.0, `assist` applies a separate
[contextual policy](context-assistant.md); KSLM v20 gates do not measure its
effect. The [expanded 0.16.0 experiment](../model/context_v2/README.md) is a
rejected research candidate, not a replacement for shipping weights.

## What mature solutions use

| Solution | Verified mechanism | Adopted in KeySwitch |
| --- | --- | --- |
| EveryLang | Character-sequence checks; rules learned after a configurable number of manual last-word switches; explicit short-word, Enter and application-exclusion policies | Manual confirmations, configurable length/boundaries and exclusions |
| Caramba Switcher | Local self-learning; erasing a bad correction and retyping creates an exception; its author calls out code and abbreviations as difficult cases | Undo records a rejection; code and abbreviations are guarded before statistical scoring |
| XNeur | Dictionary, Enchant/Aspell morphology, typo suggestions, impossible 2/3-grams and training after repeated manual conversions | Frequency dictionary, Hunspell, character 2/3/4-grams, one-extra-letter resilience and confirmations |
| KeyboardSwitcher | Pair-specific neural score, adaptive thresholds, consecutive agreement, single-character deletion for typos, context, exceptions and calibration | A local hybrid with a first-party linear classifier, trigger-specific thresholds, context, deletion evidence, rules and rejections |
| RSwitcher | Bigrams/trigrams, impossible sequences, dictionaries, code guards, special short-word lists and adaptive counts | Smoothed n-grams, dictionaries, code guards and a conservative short-word policy |
| UASwitcher | Precision-first handling: skip short words, URLs, digits, internal punctuation, ALL-CAPS, camelCase, terminals/IDEs/password managers; always/never lists and Undo | The same error-cost policy, token guards, application exclusions, rules/rejections and Undo |
| Kanningem Switcher | Frequency tables, a large Hunspell dictionary, protected developer terms, user dictionary and application exclusions | Onboard frequencies, system Hunspell dictionaries, protected technical tokens and exclusions |

## The KeySwitch hybrid model

KeySwitch decodes every physical key sequence in each supported layout group.
The detector follows a precision-first policy: a false correction costs more
than a missed one, so the linear classifier cannot bypass hard guards.

The baseline decision, before any `ContextPolicy` intervention, is layered:

1. The engine checks the global pause state, the boundary-specific option,
   application exclusions and an explicit manual layout choice.
2. The detector applies minimum length, user exclusions and rejections.
3. A confirmed user rule remains an explicit local decision and is not passed
   to the statistical layer.
4. For all other candidates, the detector guards URLs, paths, numbers,
   identifiers, `camelCase`, ALL-CAPS, mixed scripts and protected technical
   terms. A valid source word is also left unchanged; if both decodings are
   valid (`here` and `руку` share one physical sequence), the case remains
   ambiguous and is left for manual conversion.
5. If KSLM is available for a remaining candidate of at least five characters,
   its calibrated trigger/direction threshold is the sole statistical decision.
   Exact membership coverage and target-language scores remain diagnostics and
   cannot veto that decision.
6. The fallback ensemble of Onboard frequencies, Hunspell morphology,
   character 2/3/4-grams, context and single-deletion evidence is used only for
   short tokens, a disabled model, or a missing artifact. It is not run after a
   negative KSLM decision: unioning two positive rules would add their false
   positives.

### Linear classifier

The model compares the source and alternative decodings of the same physical
sequence. Its input contains signed boundary-aware character n-grams of orders
1–5, length, direction and the word-completion trigger. Feature schema v5 is
derived only from the two raw tokens: context fields, every `WordScore` and
every language-model score field are ignored by the classifier. Real
short-lived context remains in the conservative detector heuristic and is not
counted again in the linear score. Features use a stable signed FNV-1a hash into a bounded
vector. The trainer invokes the same runtime extractor with the same seeds and
n-gram orders, giving exact train/serve feature parity on Linux and Windows
without a runtime ML library. For coverage, each full character-feature name
receives an independent unsigned FNV-1a uint64 fingerprint; membership is not
inferred from the collision-prone linear-weight bucket. The fixed vector size
is 2,097,152 hash buckets.

The probabilistic layer applies only when the maximum length of the two
normalized interpretations is at least five characters. Shorter tokens remain
with the deterministic policy and user rules. Dataset generation applies the
same limit to shortened deletion variants, and the external unknown-typo corpus
contains only rows that can actually reach the model at runtime.
Short bilingual collision pairs from frozen Onboard remain as separate safety
examples that exercise the valid-source guard across every trigger without
participating in training or threshold selection.

The offline trainer fits sparse logistic regression with FTRL-Proximal. Every
variant of one physical sequence is assigned to one split before augmentation;
train, development, calibration, threshold and sealed-test sets stay separate
under `keyswitch:intent-v20:physical-signature`. The candidate pre-pass enumerates
the actual identity, deletion, duplication and transposition signatures without
accessing test, then quarantines conflicts among pre-sealed splits/languages and
protected/safety tokens. Only after the exact candidate SHA is atomically
claimed is an independent test phase built; its asymmetric merge excludes test
signatures already exposed by candidate rows, quarantine or safety data without
changing candidate rows. A post-build audit verifies these invariants against
the rows actually produced.

Since v14 (and again for every later candidate, including v20), a model-blind unknown-typo development corpus is frozen as
a separate checksum-bound JSON source. Its 10,000 unique physical signatures are
assigned by an independent hash namespace with no test role: 3,500/500/500/500
words from each language enter train/development/calibration/threshold. Every
compact record is strictly verified and expanded into symmetric positive and
negative pairs for all six triggers, producing 120,000 rows. Provenance binds
the compact file, original expanded corpus, signature set and both Hunspell
sources by SHA-256; the complete post-merge audit forbids cross-role leakage.

A train-only EN/RU scorer built from non-quarantined identity words in the
`train` split remains only as separate, checked provenance. The feature
extractor never calls it, so its character scores, train frequencies and
Hunspell cannot affect the classifier. Training config schema 13 permits at most
64 epochs with deterministic high-precision operating-point selection only on
development; log loss is the final tie-breaker and threshold remains
independent. Weights are then quantized to
signed int16 and an independent calibration set fits a Platt transform.
Training config schema 13 requires zero false positives in each trigger's
overall and typo slices while
retaining the Bonferroni/Wilson FPR gate; the sealed test participates in
neither epoch selection nor threshold selection. Selection requires
overall/typo recall of 0.956/0.91 and Pause recall of 0.91/0.86, preserving
headroom over the corresponding sealed floors of 0.95/0.90 and 0.90/0.85.

Calibration, threshold and sealed-test splits use neutral context as their
primary slice. Fixed non-empty label-independent context-stress profiles
additionally prove feature-schema-v5 invariance: changing context must not alter
the vector, logit or decision, and every profile and trigger passes the same
precision/recall/specificity and Wilson-FPR gates as the neutral slice.

After the internal sealed check, the strict evaluator runs the real production
ensemble at neutral context and six reachable context extrema (deltas
-2.05…+2.30) across sealed, typo, external unknown-typo, safety, and source-known
slices. The gate limits total false positives to no more than contextual
fallback and neutral, requires absolute precision/specificity/Wilson-FPR
constraints, and checks the asymmetric recall policy. Finite safety/source-known
sets use an exact-zero invariant and must not reach the linear model; every
unknown-typo row must reach it.
The source-known slice is derived from the unchanged broad Hunspell corpus, but
each negative row must additionally receive `known=true` from the already-open
runtime Hunspell handle. Mere presence of a stem in `.dic` is insufficient:
affix flags can forbid the standalone form. Unconfirmed rows are therefore not
relabelled as known; they are fail-closed excluded only from this derived policy
slice, while the source external-corpus hash remains unchanged.

Before sealed-test evaluation, separate EN→RU/RU→EN thresholds are selected
jointly for every trigger on neutral context. Each directional operating curve
must contain both labels in its overall and typo slices. The aggregate trigger
slice must simultaneously reach
precision >= 0.9995, recall >= 0.956 and specificity >= 0.999 with a family-wise
95% Wilson FPR upper endpoint <= 0.001; the typo slice must reach selection
precision >= 0.9995, recall >= 0.91 and specificity >= 0.999 with the same
bound. The Bonferroni correction for 12 primary comparisons (six triggers times
overall/typo) fixes per-comparison confidence at 0.9958333333333333 and
z=2.8652602385321333; those parameters and the endpoint are hash-bound gate
evidence. The independent sealed test uses the ordinary 95% Wilson endpoint
with z=1.959963984540054. After directional selection, config schema 13
deterministically chooses on the threshold split the greatest common
calibrated-logit margin that preserves the full selection gate, then adds it to
every threshold. Its config-bound 2.0 cap was fixed before v11 from the model-blind
unknown-typo development corpus; the effective value is selected only on its
frozen threshold role and recorded for every trigger. The sealed test does not
select the margin. For `pause`, recall/typo recall become 0.91/0.86, the
FPR bound remains 0.001,
and its logit threshold in each direction must be at least 0.5 above the
strictest non-pause threshold in that same direction. If no candidate satisfies
the complete selection policy, the sealed test is not evaluated. The independent
sealed test keeps a 0.999 minimum precision for both slices; the stricter 0.9995
requirement applies only to pre-seal selection as a transfer reserve.
Safety and the selection veto must also pass before the claim, and trial KSLM
serialization must prove the runtime numeric, size-cap, and quantization
contract. Frozen lexicons are used without global pre-split truncation:
`maximum_words_per_language` accepts only zero.

The lexicon comes from byte-frozen `model/intent_v1/sources/en_US.lm` and
`ru_RU.lm`. Their source-package version, sizes, SHA-256 digests and original
`COPYRIGHT.onboard-data` are pinned alongside the config, so an operating-system
`onboard-data` update cannot change the train/test corpus or artifact. Build
provenance also binds hashes of the trainer, intent runtime, layouts,
`language_model.py`, detector and protected-token list, the Python/platform
identity, candidate/full datasets, both quarantines, excluded test signatures
and train-only scorer. The receipt also binds the exact KSLM payload/runtime
parameters; until it is valid the evaluator neither builds the test phase nor
reveals its metrics. A frozen external-evaluation
policy additionally pins the sizes and SHA-256 digests of the EN/RU Hunspell
`.dic`/`.aff` files; expected SHA-256 digests of the lexical-disjoint,
unknown-typo development, and independent unknown-typo holdout corpora; at
least 5,000 words per language; and the canonical list of all six triggers.
The holdout is built before a model is loaded under distinct rank/choice
namespaces and excludes both sealed and development signatures. The strict
evaluator recomputes these relationships and rejects any mismatch.

The bundled `layout_intent_v1.ksm` uses KSLM schema 4: a canonical JSON
manifest, little-endian int16 weights and a sorted array of unique uint64
membership fingerprints instead of a bitset of occupied weight buckets. The
loader uses binary search for exact fingerprint membership, bounds the
complete container at 14 MiB, the embedded manifest at 1 MiB, the payload at
12 MiB and membership at `2^20` fingerprints, and strictly validates
schema/feature versions, payload shape, ordering, uniqueness, CRC32 and SHA-256. If the file is absent, corrupt
or incompatible, KeySwitch keeps running with its conservative deterministic
ensemble. The `layout_intent_v1.ksm` name identifies the classifier generation,
not the container schema.
Training config schema 13, KSLM schema 4 and external publication
`manifest.json` schema 1 are three independent schemas.

KSLM schema 4 stores separate EN→RU/RU→EN Platt coefficients and exact
calibrated-logit thresholds for every trigger/direction pair. The coefficients
are fit only on the calibration split, and runtime selects `threshold_logit` by
trigger and physical direction, then compares the direction-calibrated logit
directly with it; the sigmoid-derived
threshold confidence is computed only for diagnostics and does not participate
in the decision, so sigmoid saturation cannot change the selected boundary.

The trainer leaves outputs untouched when any gate fails. After success it
pre-writes and `fsync`s every payload, then publishes report, artifact and
manifest in that order. The manifest is the final commit marker; a process
error rolls back destinations already replaced. The two-independent-run
procedure for byte-comparing all three files in one Python/platform environment
is in the [model card](../model/intent_v1/MODEL_CARD.en.md#reproducibility).

The Local linear model switch disables only this extra layer. Diagnostics show
the model version and abbreviated SHA-256 checksum, or an explicit safe-fallback
status.

The complete data, feature, split-policy and limitations card is available in
[model/intent_v1/MODEL_CARD.en.md](../model/intent_v1/MODEL_CARD.en.md).

## Learning and privacy

Local rules learn only from explicit actions. Enter in a manual-conversion
prompt activates the rule immediately without submitting a message. Without
that confirmation, two repeated conversions activate it by default; the UI
allows 1–5 on Linux and 1–10 on Windows.
Undoing an automatic correction records a rejection for that source token and
direction. `learning.json` contains only those tokens, directions and counters,
not the ordinary input stream. Baseline previous-word context is RAM-only,
expires after 45 seconds and is keyed by application name. The contextual
assistant additionally uses a bounded observed phrase in the active window and
an optional field snapshot. Technical logging can contain evaluated words,
including unchanged words; this is a separate setting, not model-weight training.

The global linear model is built by a separate offline tool and is not updated
from the ordinary keystroke stream while the application runs. Inference is
fully local: neither words nor their features are sent over the network.
Disabling the model does not remove explicitly confirmed user rules or
rejections.

## Reproducible evaluation

Run:

```bash
PYTHONPATH=src tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
PYTHONPATH=src tools/evaluate_intent_model.py --strict
```

The second command validates the bundled KSLM structure, reproduces its fixed
evaluation slices and applies non-vacuous strict Wilson gates. The safety corpus
is evaluated through the production `LanguageDetector` with real runtime
scorers and pre-model guards; direct raw-model predictions and membership
coverage are published only as diagnostics, not gates. The linear model's
training set is built from SHA-256-verified frozen Onboard lexical data and
synthetic correct/wrong-layout pairs with symmetric typo augmentation. Grouping
and quarantining by physical key sequence prevent direct variant leakage across
splits, but do not turn this corpus into a study of real user input.

On the verified system with `onboard-data`, `libhunspell-1.7-0`,
`hunspell-en-us` and `hunspell-ru`, the following baseline result was recorded
(a historical slice without the new `ContextPolicy`):

| Intended word language | Precision | Recall | Specificity |
| --- | ---: | ---: | ---: |
| EN | 100.00% | 99.67% | 100.00% |
| RU | 100.00% | 99.86% | 100.00% |

An additional broad slice of 10,000 evenly selected stems from each Hunspell
dictionary produces:

| Intended word language | Precision | Recall | Specificity |
| --- | ---: | ---: | ---: |
| EN | 99.96% | 94.29% | 99.96% |
| RU | 100.00% | 99.22% | 100.00% |

The run evaluates 10,000 frequent and 10,000 broad-dictionary words per
language both correctly and under the wrong layout, plus 196 curated
inflections, typos, ambiguous cases and technical tokens. This is a regression
corpus derived from the project's own lexical sources, not an independent study
of real user text. Its figures therefore must not be compared directly with
marketing figures for Punto Switcher, EveryLang or any other product.

In particular, KSLM calibration describes only this lexical-synthetic corpus.
It must not be interpreted as a production-measured probability of a correct
switch. That requires a separately collected explicitly labelled feedback set;
the absence of an Undo action alone is not a confirmed label.

CI requires at least 99.9% precision and specificity and 98.5% recall on the
frequency corpus; at least 99.9% precision and specificity and 90% recall on
the broad dictionary corpus; and zero failures in the defensive corpus.

## Primary sources

- [EveryLang official help](https://everylang.net/help)
- [Caramba Switcher official site and self-learning description](https://caramba-switcher.com/)
- [XNeur detector source](https://github.com/linuxbuh/xneur/blob/main/lib/ai/detection.c)
- [XNeur configuration manual](https://github.com/linuxbuh/xneur/blob/main/xneurrc.5)
- [KeyboardSwitcher source](https://github.com/kertser/KeyboardSwitcher)
- [RSwitcher algorithm description by its author](https://github.com/andrewchuev/rswitcher)
- [UASwitcher precision-first heuristics](https://github.com/deimoc/UASwitcher)
- [Kanningem Switcher data and guard description](https://github.com/kanningemai-sudo/kanningem-switcher)
- [ACL Anthology: Language Identification of Short Text Segments with N-gram Models](https://aclanthology.org/L10-1193/)
- [Google Research: Ad Click Prediction — FTRL-Proximal](https://research.google/pubs/ad-click-prediction-a-view-from-the-trenches/)
- [AISTATS/PMLR: Beta calibration](https://proceedings.mlr.press/v54/kull17a.html)
- [arXiv: Language Detection Engine for Multilingual Texting on Mobile Devices](https://arxiv.org/abs/2101.03963)
- [Hunspell official repository](https://github.com/hunspell/hunspell)

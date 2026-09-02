# KeySwitch Layout Intent v1

[Русский](MODEL_CARD.md) · [**English**](MODEL_CARD.en.md)

## Purpose

`Layout Intent v1` is KeySwitch's own local linear classifier for the intent to
switch between the EN and RU layouts. It evaluates the pair “typed word → the
same physical key sequence in the other layout” and is used only inside the
conservative KeySwitch policy. Hard safeguards for short words, code,
application exclusions, accepted and rejected user rules, and a valid source
word remain above the model.

The model sends no text over the network, requires no NumPy, scikit-learn, ONNX
or separate runtime, and executes one scalar pass over sparse features.

## Certified v15 artifact

The current artifact is `intent-v1-bec1f1d3dceb`, 12,962,713 bytes, SHA-256
`7631b821bafc958364353a8a13de3abc23e922e51b589bd181075db55fa9e9dc`.
Its build provenance is
`bec1f1d3dceb05ccf6edd10f5b2cf5f18bdf184a19af3d24b73e1cf418c2e7e2`,
config digest is
`06fa899534c8e6e0d3984d2ff7e22b46fe9721efde7df4cf02c53b6967d55127`,
and dataset digest is
`624dc0e77601fa1d1157c85c43ed9f379b33a60a924e00d477692ae7582cad33`.
The trainer ran all 64 permitted epochs and selected epoch 64; the container
holds 765,205 nonzero weights and 1,031,416 exact membership fingerprints.

The complete independent strict report, SHA-256
`82ff2b6f332369eea2e71eb2df4960a554a1aa9de9c31025c35bf15d4485c303`,
passed all 30 gates. On the model-blind unknown-typo holdout, the ensemble had
12 false positives among 60,000 negatives (2 per 10,000-negative trigger
slice), precision 0.999793719, specificity 0.9998 and recall 0.96935; ordinary
triggers have 0.9702 recall and Pause has 0.9651. Of those 12 false positives,
6 were introduced by the model, while it prevented 60 false positives of the
deterministic fallback. The upper 95% Wilson endpoint for every 2/10,000
negative trigger slice is 0.000728996 against the 0.001 limit. The internal
sealed test shows 2 false positives among 21,574 negatives for every trigger
with recall 0.973160896 (Pause 0.967598387). All seven production-context
profiles passed. Across 5,000 measurements inference median is 0.608413 ms and
p95 is 0.936677 ms; load median across 11 measurements is 186.378296 ms and
p95 is 236.702506 ms. These synthetic results are not an estimate of real user
traffic. The v15 holdout was built under a new namespace and differs from the
v14 holdout, so 12/60,000 here and 0/60,000 for v14 are results on different
samples rather than a direct comparison of one metric.

Two independent full retraining runs were executed after the official train in
the same Python/platform environment and wrote to distinct output paths. The
comparison proved three-way byte identity across official/replay-a/replay-b for
the KSLM, manifest and test report; their SHA-256 digests are respectively
`7631b821bafc958364353a8a13de3abc23e922e51b589bd181075db55fa9e9dc`,
`e0070e8e6813da4a8dde1a09eb2c1713f033d002a64216299cba3764032d82f7`, and
`05caf3828ff2724fc5f1d22ff2e28d9b31cd2d1bcfcceb64f934bb8bfe84480d`. The
independent strict evaluator, re-run against replay-a, also passed all 30
gates; that report has SHA-256
`88f6704c845efd86b8c3ba924607bcdf972e915d0d7d6c75ff147e1d0099f23e`. Both
replays ran sequentially: one trainer with its worker pool claims nearly all of
the reference host's memory.

## Training data and licensing

Training uses only the `1-grams` sections of the byte-frozen Onboard models
stored in the repository:

- `model/intent_v1/sources/en_US.lm`;
- `model/intent_v1/sources/ru_RU.lm`.

The snapshot comes from Ubuntu 26.04 package `onboard-data`
`1.4.3+git20260213+ds-2`. The file
`model/intent_v1/sources/COPYRIGHT.onboard-data` is a byte-for-byte copy from
that package; its `Files: models/*` stanza contains the exact `GPL-3+`
declaration and attribution. `SHA256SUMS` and the `sources` section of
`config.json` pin the SHA-256 digest and size of all three files. The trainer
records the same provenance fields in the manifest. We neither replace that
declaration with a normalized SPDX identifier nor make independent legal
conclusions in this model card. No third-party corpora or network APIs are
used.

The byte-level provenance and checksums are documented in
[sources/README.md](sources/README.md) and `sources/SHA256SUMS`.

## Leakage protection

The split unit is not a row or a language but a physical key sequence. A
Russian word is first mapped to US-keyboard coordinates. Before augmentation,
the SHA-256 digest of that sequence in the
`keyswitch:intent-v15:physical-signature` namespace assigns it to one of 40
stable buckets:

- 26/40 (65%) — training;
- 4/40 (10%) — epoch selection;
- 4/40 (10%) — sigmoid calibration;
- 3/40 (7.5%) — operating-threshold selection;
- 3/40 (7.5%) — sealed final test.

One physical sequence can never appear in two splits. EN/RU words with the
same sequence are excluded from training and placed in a separate safety set
whose target decision is “do not switch.”

The dataset is constructed in two phases. Before the seal is claimed, a
separate pre-pass enumerates the physical signatures produced by identity,
deletion, duplication and transposition variants only for train, development,
calibration and threshold. A signature enters the candidate quarantine if it
has owners from different pre-sealed splits or languages, or intersects a
protected hard-negative or safety token. At this phase the sealed test is not
built and cannot affect candidate rows, the quarantine, scorer or fingerprint.

Only after a successful claim is the test partition built independently with
its own quarantine. An asymmetric merge removes test signatures that overlap
already fixed candidate rows, quarantine or safety data; pre-sealed rows must
remain byte-for-byte equivalent. The generated sides of the merged pairs are
then audited again. Canonical SHA-256 values for both quarantines, the excluded test
signatures and occurrence counts are part of model provenance.

V15 uses an additional frozen source,
`unknown-typo-development-v15.json`, created model-blind before training from
the unknown-typo development corpus. It contains 10,000 unique physical
signatures, 5,000 per language, and has no test role. The independent
`keyswitch:intent-v15:unknown-typo-development-role` namespace assigns 3,500
words per language to train and 500 each to development, calibration and
threshold. The loader verifies source size and SHA-256, Hunspell `.dic`/`.aff`
provenance, EN/RU physical equivalence, uniqueness, and the exact SHA-256 of
120,000 re-expanded rows (two labels times six triggers). A complete row-level
audit after merging again forbids cross-split, cross-language, safety and
quarantine overlap. The compact source and freezer are part of toolchain
provenance; the external v15 holdout uses distinct rank/choice namespaces.

`config.json` schema 13 also contains the `sealed_evaluation` schema 1 policy.
Its repository-relative
`registry_path: model/intent_v1/seal-registry-v15.json` is resolved from the
canonical project root, not from the location of a supplied config copy, and
binds one candidate SHA to one `split_namespace`. After the complete pre-sealed
gate passes — threshold/context, safety, selection veto, and trial runtime KSLM
serialization — but before the test phase is built or the first sealed-test row
is scored, the trainer calculates the canonical candidate hash. Trial
serialization proves numeric bounds, quantization parity, and
payload/fingerprint limits in advance. The hash binds the candidate dataset,
toolchain, scorer, selection evidence and the exact KSLM
payload and runtime parameters. The registry is first written completely and
`fsync`ed in a same-directory temporary file, then published with an atomic
no-replace hard link. A byte-identical record permits an identical-candidate
rerun for reproducibility. Any difference is rejected before sealed-test
access; a changed candidate requires an explicit joint rotation of
`split_namespace` and `registry_path`.

`seal-registry-v2.json` is retained only as immutable evidence of a rejected
attempt: the v2 candidate was claimed and the phase audit then stopped the run
before any sealed-test evaluation or model publication. That namespace is not
reused.

`seal-registry-v3.json` is likewise retained as immutable audit evidence. The
v3 candidate passed pre-seal and the ordinary sealed slice, but its non-pause
typo slice produced 10 false positives among 17,392 negative examples: the
0.001058171 upper 95% Wilson bound exceeded the 0.001 policy. No model was
published. That test is not reused for tuning; the next run used a new v4
namespace and a separate v4 registry.

`seal-registry-v4.json` is retained for the same reason. On selection, the v4
candidate produced 9 false positives among 23,067 overall negatives and 8 among
17,220 typo negatives. The independent sealed test produced 14/23,090 and
13/17,223: the ordinary upper 95% Wilson endpoints of 0.001017564 and
0.001291083 exceeded the 0.001 policy. Again, no model was published. The next
v5 run did not reuse that test: its pre-seal policy applied a Bonferroni-corrected
family-wise 95% endpoint to the 12 primary FP checks (six triggers times
overall/typo), fixing per-comparison confidence at 0.9958333333333333 and
z=2.8652602385321333. The sealed gate remains an ordinary independent 95%
Wilson endpoint.

`seal-registry-v5.json` fixes the next non-reusable candidate. It passed the
internal sealed gates, but external production evaluation exposed two issues:
secondary heuristic conditions reduced unknown-typo ensemble recall to 0.5965
while raw-model recall was about 0.98, and 822 of 10,000 parsed `.dic` stems
were not accepted by the open runtime Hunspell handle because of affix-flag
semantics. Once inspected, that external v5 corpus became development-only.

`seal-registry-v6.json` and `rejection-v6.json` record the next rejected
attempt. The ordinary non-pause slice passed with 9/21,288 false positives and
an upper 95% Wilson endpoint of 0.000803369; Pause also passed. The non-pause
typo slice, however, produced 9/15,812 and an upper endpoint of 0.001081498
against the 0.001 limit. No v6 artifact, manifest, or report was published. V7
passed 29 of 30 strict external gates but was rejected for a 655.432857 ms load
latency p95 against a 500 ms limit; `rejection-v7.json` preserves the exact
decision. V8 passed the other 29 gates, including the corrected 268.771704 ms
load-latency p95, but the production-context check found that secondary
membership/target-score vetoes reduced raw sealed non-pause recall from
0.950488303 to 0.941585283; `rejection-v8.json` preserves the exact decision.
The revealed v6/v7/v8 tests are not reused. V9 removed those post-model vetoes
and passed its internal gates, but the independent external unknown-typo
holdout produced 4 false positives among 10,000 negatives for every trigger:
precision 0.999591378, specificity 0.9996, and an upper 95% Wilson endpoint of
0.001028128 against the 0.001 limit. The production-context gate therefore
rejected it; `rejection-v9.json` preserves the exact decision, and revealed v9
sealed/holdout data is not used for tuning. V10 applied the signed 2.0 cap
selected only on development and deterministically chose a common margin of
0.9938225471937638. It passed pre-seal, but independent sealed non-pause recall
was 0.944410276 against the 0.95 minimum; `rejection-v10.json` records the exact
decision and the revealed v10 rows are not reused. V11 passed its internal
selection and sealed gates, but the signed strict evaluator then stopped before
the independent external holdout because its exclusion index did not support
the new frozen `hunspell-unknown-*` row family. The v11 artifact is rejected and
`rejection-v11.json` preserves the exact cause and hashes. V12 fixed that index,
but the evaluator built its base exclusion index after merging the development
corpus and could not reproduce the frozen provenance; `rejection-v12.json`
records the rejection before external metrics. V13 fixed domain separation and
reached the independent holdout, where ordinary triggers produced 4 false
positives among 10,000 negatives and a 0.001028128 Wilson upper endpoint against
the 0.001 limit; `rejection-v13.json` records the decision. Before opening a new
holdout, v14 fixed a zero-FP selection budget and 0.956 minimum recall, rotated
the split/registry/source/holdout namespaces, and passed strict evaluation with
0 false positives among 60,000 unknown-typo negatives. After the trainer
became multiprocess, v15 rotated every namespace again, froze a new model-blind
holdout without loading a model in `holdout-v15-preseal.json`, and passed
strict evaluation with 12 false positives among 60,000 negatives of that new
holdout and recall 0.96935.

`--dry-run` does not waive this guarantee: after the complete pre-sealed gate,
the registry is claimed before sealed scoring and the seal is consumed even though
the artifact, manifest and report are not published.

## Examples and features

A symmetric pair is created for every word:

- the word in the correct layout is a negative example;
- the same sequence in the wrong layout is a positive example.

Deletion, duplication and transposition of adjacent physical keys are applied
symmetrically to both classes. This prevents the model from learning the unsafe
rule “every typo means the wrong layout.” Technical tokens, addresses,
versions, paths and identifiers are added as hard negatives; a separate set of
such rows remains a safety check.

Feature schema v5 uses no dense lexical or context features. Classifier input
is built only from the source and alternative raw tokens: signed character 1–5-grams,
direction, length and trigger. `context_delta`, `context_group` and every
`WordScore` field — lexical/frequency/ngram/invalid-ratio, exact and spell-known
— are ignored even when the caller populated them. Real short-lived context
remains in the conservative detector heuristic and is not counted a second time
in the linear score. The trainer passes an intentionally neutral `WordScore`
and invokes the same runtime extractor with the same feature and membership
seeds and n-gram orders. This provides exact train/serve feature parity and
removes feature dependence on the language-model corpus.

Two train-only scorers, built from non-quarantined identity words in the
`train` split, are stored as a separate verifiable provenance object. They use
character 2/3/4-grams without Hunspell, but the feature extractor never invokes
them, so their scores and training frequencies cannot affect the classifier.
The canonical hash of the input training lexicons, their sizes and the number
of excluded quarantined identity rows are recorded in the external manifest.
The frozen EN/RU lexicons are used in full after filtering;
`maximum_words_per_language` must remain zero, so held-out identities cannot
influence global truncation before split assignment.

Calibration, threshold selection and sealed test use neutral context
(`context_delta=0`, `context_group=None`) as the primary slice. Threshold and
sealed test separately certify fixed, non-empty, label-independent
context-stress profiles. For feature schema v5 this is an invariance check:
changing only `context_delta`/`context_group` must not alter the feature vector,
logit, probability or decision, and every profile passes the same per-trigger
precision/recall/specificity/Wilson-FPR gates. One training trigger is selected
uniformly by a hash function from all runtime triggers; threshold/test evaluate
every trigger separately.

Eighteen stress profiles cover both context relations (`source` and `target`),
the ±6 adversarial-domain limits, representative outer, inner and near-zero
points at ±1.25, ±0.75 and ±0.125, and the zero point:

| `context_delta` | `source` | `target` |
| ---: | --- | --- |
| -6.0 | `source_minimum` | `target_minimum` |
| -1.25 | `source_outer_negative` | `target_outer_negative` |
| -0.75 | `source_inner_negative` | `target_inner_negative` |
| -0.125 | `source_near_zero_negative` | `target_near_zero_negative` |
| 0.0 | `source_zero` | `target_zero` |
| +0.125 | `source_near_zero_positive` | `target_near_zero_positive` |
| +0.75 | `source_inner_positive` | `target_inner_positive` |
| +1.25 | `source_outer_positive` | `target_outer_positive` |
| +6.0 | `source_maximum` | `target_maximum` |

`positive`/`negative` in these names denotes only the sign of the observed
`context_delta`, not the example label; `minimum`/`maximum` denotes the
adversarial-domain limits. In the report, `context` equals
`neutral_primary_plus_fixed_label_independent_stress`, and the exact ordered
list is pinned in `context_stress_profiles`. Pre-sealed evidence is stored in
`threshold_selection_gate_breakdown.neutral` and
`threshold_selection_gate_breakdown.context_stress.profiles.<name>.per_trigger`,
with separate `{overall,typos}` results for each trigger. Sealed gates use the
same structure in `quality_gate_breakdown.sealed_test_context_stress`; raw
sealed metrics are in
`sealed_test_context_stress.<name>.{overall,typos}`.

A separate strict production-context gate executes the real
`LanguageDetector` at neutral context and six reachable extrema:
`none_min/-1.75`, `none_max/+1.75`, `source_min/-2.05`, `source_max/+1.45`,
`target_min/-1.20`, and `target_max/+2.30`. It covers five slices: sealed test,
sealed typos, external unknown typos, safety, and source-known. Every profile
has absolute precision/specificity/Wilson-FPR constraints, may have no more
total false positives than either contextual fallback or neutral, and follows
an asymmetric recall policy: an absolute floor for neutral/target-supporting
context and no worse than fallback minus 0.005 for source-supporting context.
Finite safety/source-known sets use an exact-zero invariant and must stop before
model inference, while every unknown-typo row must reach the model. The
evaluator records the complete proof
under `production_context_ensemble`.
Source-known rows are a runtime-verified subset of the frozen lexical-disjoint
corpus: each stem must actually be accepted by the open Hunspell handle,
because a `.dic` entry carrying affix flags does not guarantee a valid
standalone form. This derived filter leaves the pinned source-corpus hash
unchanged and fail-closed requires examples from both directions.

The extractor shared with runtime uses signed feature hashing (FNV-1a 64),
character n-grams of orders 1–5 for both interpretations, layout direction,
length and boundary type. The vector has 2,097,152 hash buckets. Its dimension,
seed, separate membership
seed and n-gram orders are pinned in `config.json`; feature schema v5 is pinned
by shared trainer/runtime constants and the embedded KSLM manifest.

Runtime invokes the classifier only when the maximum length of the two
normalized interpretations is at least five characters. The dataset builder
applies the same minimum after typo augmentation, so a shortened deletion typo
cannot be present in training or the external unknown-typo evaluation and then
be skipped at runtime. The limit is pinned under
`gate_policy.model_applicability` in both the embedded and external manifests.
After the hard guards, an applicable KSLM result is the sole statistical
decision: only its calibrated direction/trigger threshold is applied.
Membership coverage and language scores remain diagnostics and cannot veto a
positive threshold result. A negative result does not give the heuristic a
second chance; that ensemble remains a fallback only for short tokens, a
disabled model, or a missing artifact.
Short real collision pairs from the same frozen sources are retained only in
the safety corpus. They contribute no model features or gradients, but provide
non-zero valid-source hard-guard coverage for all six triggers.

## Training algorithm

Training uses sparse FTRL-Proximal logistic regression. For coordinate `i`:

```
w_i = 0,                                      if |z_i| <= L1
w_i = -(z_i - sign(z_i) * L1) /
      ((beta + sqrt(n_i)) / alpha + L2),      otherwise

g_i = (sigmoid(w*x) - y) * x_i * sample_weight
sigma_i = (sqrt(n_i + g_i^2) - sqrt(n_i)) / alpha
z_i <- z_i + g_i - sigma_i * w_i
n_i <- n_i + g_i^2
```

The intercept is trained with the same update but without L1/L2. After each
epoch, only the development split supplies log loss and an aggregate
high-precision operating point. Epoch ranking first prefers complete passage of
the same precision/recall/specificity/family-wise-Wilson-FPR policy, then the
number of
passing checks, recall, typo recall and guard metrics; log loss is the final
tie-breaker. The threshold split never participates in epoch selection. Example
order is controlled by a separate `random.Random(seed + epoch)`. Configuration
allows at most 64 epochs, at least 6, and early stopping after 4 epochs without
an ordering improvement. The manifest records the selected epoch and the full
metric history.

The formulas follow FTRL-Proximal as described by [McMahan,
2011](https://proceedings.mlr.press/v15/mcmahan11b.html) and [McMahan et al.,
2013](https://research.google.com/pubs/archive/41159.pdf).

## Quantization, calibration and thresholds

Weights are quantized symmetrically to signed int16. Calibration and every
subsequent metric are calculated from quantized logits, matching runtime
execution. KSLM schema 4 stores sorted, unique uint64 fingerprints of the full
names of character features actually observed in the training split.
Membership uses a separate unsigned FNV-1a namespace, not the linear-weight
bucket. Runtime looks up the exact fingerprint by binary search; therefore, a
collision in the bounded weight vector does not falsely claim coverage as an
occupancy bitset would.

The container consists of a canonical JSON manifest, little-endian int16
weights and little-endian uint64 fingerprints. The loader validates the schema
and feature versions, exact payload shape, membership ordering and uniqueness,
CRC32, SHA-256 and a 14 MiB complete-file limit. The embedded manifest is
capped separately at 1 MiB, the payload at 12 MiB and membership at `2^20`
fingerprints. `layout_intent_v1.ksm` remains the
stable name of the first classifier generation; the container format has
independently advanced to KSLM schema 4. The three independent version numbers
must not be conflated: the training config uses `schema_version: 13`, the
container and its embedded manifest use KSLM schema 4, and the external
publication `manifest.json` uses `schema_version: 1`.

Independent two-parameter sigmoid calibration (Platt scaling) is trained for
each physical EN→RU and RU→EN direction only on the calibration split. Each
mapping is monotonic, preserving within-direction ranking while correcting the
inter-direction score shift. Its result is **technical confidence on the synthetic
lexical distribution**, not the probability of a real user's error. Neither
the UI nor documentation should call it a real-world probability.

For every runtime trigger (`boundary_probe`, `space`, `enter`, `tab`,
`punctuation`, `pause`), separate EN→RU/RU→EN logit thresholds are selected
jointly only on the threshold split. Context is neutral, and each directional
operating curve must contain both labels in the overall and typo slices. Before
the sealed test is evaluated, a candidate must pass the
complete selection policy. For the overall slice this is precision >= 0.9995,
recall >= 0.956, specificity >= 0.999, and a family-wise upper 95% Wilson FPR
endpoint <= 0.001. For the typo slice it is selection precision >= 0.9995,
recall >= 0.91, specificity >= 0.999, and the same endpoint. The Bonferroni
correction covers 12 primary comparisons (six triggers times overall/typo):
per-comparison confidence 0.9958333333333333 and z=2.8652602385321333. Signed
gate evidence records the method, correction, comparison count, confidence, z
and endpoint; the sealed gate independently uses ordinary 95% Wilson with
z=1.959963984540054. Config schema 13 additionally requires zero false positives
in every trigger's overall and typo selection slices; this absolute budget is
part of the signed evidence and is checked before test materialization. After directional selection, the
trainer deterministically
finds on the threshold split the greatest common margin that preserves the
complete selection gate and adds it to every calibrated-logit threshold. The
signed 2.0 cap was fixed before v11 from the external model-blind unknown-typo
development corpus; schema 13 selects the effective margin only on its frozen
threshold role. It is recorded for every trigger, and the sealed test does not
participate in its selection. Selection requires overall/typo recall of
0.956/0.91 and Pause recall of 0.91/0.86, preserving headroom over the sealed
floors of 0.95/0.90 and 0.90/0.85. The FPR limit remains <= 0.001, and its
logit threshold in each direction is at least 0.5 above the strictest non-pause
threshold in that same direction;
metrics are recalculated after that tightening. If the full requirement set is
infeasible, the trainer exits with `sealed_test_evaluated=false` without
reading sealed-test results. The independent sealed test keeps a 0.999 minimum
precision for both slices; strengthening selection precision and recall
provides transfer headroom without changing the closed gate. An observed zero
FPR without a sufficiently large
slice is not accepted as safety evidence. KSLM schema 4 stores both calibration
parameter pairs, an exact calibrated-logit threshold for each trigger/direction
pair, and a raw-logit veto threshold for the conservative policy layer. Runtime
selects the exact `threshold_logit` by trigger and physical direction, then
compares the calibrated logit with it. The sigmoid-derived
confidence at that threshold is diagnostic only and is not part of the
comparison, so sigmoid saturation cannot collapse different logit boundaries
into the same runtime decision.

The veto threshold is not borrowed from auto-switch. It is placed a fixed 0.25
margin below the lower 0th percentile — the minimum logit of positive
calibration examples. The manifest and sealed test separately record the share
of positive examples such a veto could block; the allowed false-negative rate
is 0.001.

The strict safety gate passes the protected corpus through the actual
production `LanguageDetector`: runtime lexical scorers, pre-model guards and
the policy layer. Layout collisions and protected tokens must stop before the
model, and zero guard failures are allowed. Direct linear-model responses and
membership coverage for those rows are retained only as raw diagnostics with
`is_a_gate=false`; they do not replace validation of the actual production
decision.

## Reproducibility

The complete candidate acceptance/rejection procedure is documented in the
Russian [runbook](../../docs/intent-model-runbook.md), with copy-paste training
recipes in the [cookbook](../../docs/intent-model-cookbook.md).

```bash
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
PYTHONPATH=src:tools python3 tools/preseal_intent_holdout.py | \
  diff -u model/intent_v1/holdout-v15-preseal.json -
PYTHONPATH=src python3 tools/train_intent_model_release.py
PYTHONPATH=src python3 tools/evaluate_intent_model.py --strict
```

The v15 trainer is part of the candidate identity and remains unchanged after
its receipt is issued. At the KSLM write boundary it converts dataclass tuple
containers to JSON-native arrays and proves that canonical JSON bytes remain
unchanged. `train_intent_model_release.py` remains the stable command entry
point and only delegates to the trainer, without monkey-patching process-global
state.

`config.json` schema 13 contains every parameter and exact frozen-source
provenance without hidden defaults. Its root `external_evaluation` section pins
`schema_version`, `minimum_words_per_group: 5000`, the canonical
`trigger_expansion` (`boundary_probe`, `pause`, `space`, `enter`, `tab`,
`punctuation`), and the exact `dictionary_sha256`, `dictionary_bytes`,
`affix_sha256` and `affix_bytes` for `hunspell.en_US` and `hunspell.ru_RU`.
The nested `external_evaluation.schema_version: 2` versions only this external
policy; it does not change the training config's root `schema_version: 13`.
The resulting samples are pinned by `lexical_disjoint_corpus_sha256`,
`unknown_typo_development_corpus_sha256`, and
`unknown_typo_holdout_corpus_sha256`. Before v11, the development corpus was
used to select serving policy, including the signed 2.0 cap. Since v14 a fresh
model-blind development source is assigned to independent pre-sealed roles; the effective global
calibrated-logit margin is selected only on the threshold role.
The v15 holdout was built under distinct rank/choice namespaces before loading
a model, excludes all 288,869 sealed and 10,000 development physical
signatures, and is first evaluated only after the candidate receipt is fixed.
Its model-blind provenance is pre-recorded in `holdout-v15-preseal.json` with
`model_loaded=false`, `metrics_evaluated=false`, and both overlap counts equal
to zero. The external manifest schema 1 stores SHA-256
digests for config, frozen sources, trainer, the external evaluator, the preseal
generator/receipt, development freezer, runtime intent extractor, layouts,
`language_model.py`, detector, frozen hard-negative source and the
protected-token list, as well as Python
implementation/version/build, platform, architecture, libc and byte order. A
build-provenance hash additionally binds the candidate/full datasets, both
quarantines, excluded test signatures and train-only scorer; the first 12
characters of that hash are included in the model version.
The strict evaluator recomputes these relationships and validates the current
toolchain and protected-token hashes.

The manifest and report contain the same immutable `sealed_evaluation` receipt:
schema, namespace, candidate/config/candidate-dataset SHA-256 values, the
repository-relative registry path and the SHA-256 of its canonical bytes. The
evaluator independently recomputes the candidate SHA from the loaded KSLM,
candidate dataset, toolchain, scorer and selection evidence, and requires the
receipt to match the local registry exactly. A missing, unavailable, symlinked,
oversized, modified or inconsistent registry causes a fail-closed stop before
the test phase is built or sealed metrics are printed, even without `--strict`.
The full dataset SHA is checked separately after the merge and before metrics
are calculated.

Strict test and release CI jobs pin the GitHub-hosted `ubuntu-26.04` runner so
the Ubuntu generation used for external Hunspell validation is explicit; the
label is published in the official [GitHub Actions image
list](https://github.com/actions/runner-images/blob/main/README.md#available-images).
The exact `.dic`/`.aff` hashes and sizes in the external-evaluation policy
remain the authoritative gate: a changed runner snapshot fails validation
instead of silently changing the corpus.

The trainer publishes nothing when a gate fails. After successful validation,
it first writes and `fsync`s every temporary payload, then replaces destinations
in report -> artifact -> manifest order. The manifest is published last as the
commit marker; a process error restores the previous bytes or previous absence
of already-replaced files. The three output paths must differ from each other
and from immutable inputs.

The following procedure is not a sealed-test preview: its first run claims the
configured seal, and its second run is allowed only because the candidate is
byte-identical. Run it only for a candidate that is authorized to consume the
namespace. Do not delete or edit the registry to test a changed candidate; that
requires an explicit policy rotation.

Byte-identical retraining requires the same source and config bytes, the same
toolchain hashes, and the same pinned Python/platform identity:

```bash
set -euo pipefail
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
retrain_root="$(mktemp -d /tmp/keyswitch-intent-retrain.XXXXXX)"
trap 'rm -r -- "$retrain_root"' EXIT
for run in a b; do
  mkdir "$retrain_root/$run"
  PYTHONPATH=src python3 tools/train_intent_model_release.py \
    --artifact "$retrain_root/$run/layout_intent_v1.ksm" \
    --manifest "$retrain_root/$run/manifest.json" \
    --test-report "$retrain_root/$run/test-report.json"
done
cmp "$retrain_root/a/layout_intent_v1.ksm" \
    "$retrain_root/b/layout_intent_v1.ksm"
cmp "$retrain_root/a/manifest.json" "$retrain_root/b/manifest.json"
cmp "$retrain_root/a/test-report.json" "$retrain_root/b/test-report.json"
PYTHONPATH=src python3 tools/evaluate_intent_model.py \
  --artifact "$retrain_root/a/layout_intent_v1.ksm" \
  --manifest "$retrain_root/a/manifest.json" --strict
```

Successful `cmp` commands prove that all three files are byte-identical. This
guarantee does not extend to another Python/libc version or platform, because
their identity is deliberately included in provenance. The installed
`onboard-data` version does not affect the result because the trainer reads only
the frozen copies.

## Limitations and updates

- The model covers only the fixed US/RU pair.
- The data is lexical and synthetic; real user streams, applications, style
  and error frequency differ.
- The independent sealed test prevents tuning against the report, but does not
  replace voluntary anonymized/local feedback and a long shadow-mode period.
- The local registry protects normal and CI retraining, concurrent claims and
  config-path mistakes, but is not an indestructible ledger: a filesystem owner
  can delete all local evidence. The committed registry record in protected
  remote Git history, with mandatory review, must be the operational append-only
  authority; deletion or rotation requires a separate deliberate policy change.
- A new feature schema, split namespace or data format requires a new
  feature/config/container version; the existing `intent_v1` must not be
  silently retrained with incompatible semantics. Current values are feature
  schema v5, split namespace `keyswitch:intent-v15:physical-signature`, training
  config schema 13, KSLM schema 4 and external manifest schema 1.
- Recall must not be increased by violating precision, specificity or the
  safety gates in the fixed config.
- For false positives, the report contains the observed count, negative-slice
  size, ordinary upper 95% Wilson bound, and the exact statistical endpoint
  used by the gate. Even zero observations do not mean zero real-world risk.
- An additional Hunspell slice lexically excludes every Onboard unigram and
  checks the end-to-end detector with and without the model on at least 5,000
  EN and 5,000 RU stems. It is not a fully independent source because Hunspell
  is also part of the runtime language scorer, which is explicitly identified
  in the report. Frozen `.dic`/`.aff` hashes and sizes, hashes of the resulting
  lexical-disjoint and unknown-typo corpora, and the complete trigger set
  prevent this external regression slice from being silently substituted.

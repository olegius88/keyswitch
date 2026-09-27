# Context policy, public-phrase experiment 1

Status: **rejected for runtime promotion**. The comparison preserves the exact
v1 weights that shipped at the time of this experiment in
`baseline-context-v1.json`; that frozen baseline is not a declaration of the
currently installed model. `candidate.json` is a research artifact, not a
package resource or an automatic upgrade. This experiment did not change the
default application mode or existing user settings.

## Data and separation

- 64,537 public CC0 source sentences: 41,503 EN and 23,034 RU.
- 63,513 retained variants in 59,721 normalized/near-duplicate groups.
- 224,693 action situations: 147,835 train; 24,746 development; 23,851
  calibration; 25,653 phrase test; 2,608 focus-lexical test.
- 6,272 source sentences (5,891 groups) were an **unused reserve for this
  experiment**. The later [boundary experiment](../boundary_v1/README.md)
  consumes words from that reserve; it is no longer globally untouched.
- A group holds at most four source variants. Grouping joins normalized
  lexical text and bounded single-word substitutions/insertions/deletions;
  it is not a semantic paraphrase detector.
- The phrase test has unseen source groups, but may share common words with
  training. The additional lexical test excludes its supervised focus-token
  families from training/development/calibration; surrounding text and the
  pre-existing external lexicons can still contain those words.
- Texts are real contributed sentences; layout/spelling errors, application
  IDs, trigger choices and action labels are **synthetic interventions**.
  Technical keep cases are project-authored. Neither represents recorded
  application traces or independently annotated human intent.

Words with diacritics, combining marks, mixed identifiers, addresses or
digits are excluded from natural-word interventions rather than partially
tokenized into misleading examples. Exact punctuation/spacing are preserved.
The independent technical curriculum covers commands, variables, paths,
addresses, versions and abbreviations, including inside Russian sentences.
Deletion spelling errors are keep examples: a typo is not by itself proof
that the layout should change. Label ambiguity remains a limitation.

## Training and review policy

The same runtime feature extractor (format version 2) is used without
modification. A sparse four-action AdaGrad classifier is trained for 24
epochs; development loss selects the epoch. Train-only feature counts select
at most 45,000 features. A small training-only C kernel has a Python reference
and numerical parity tests; there is no new native/runtime dependency.

Training uses a deterministic mix of portable and reference-Hunspell evidence.
Development, calibration and test each check both profiles. Conversion
threshold candidates and quality conditions are fixed in `config.json`
before test scoring. Zero false conversions on calibration selected 0.9999.
That score is **not** a calibrated real-world correctness probability.

`candidate-seal.json` binds the selected weights, feature count, data split,
configuration, source code and frozen inputs before test evaluation. The
test does not select weights, features, epochs or thresholds. A failed test
is retained as evidence, not followed by lowering a threshold on the same
test. Subsequent experimentation needs a new candidate and independent test.

## Observed results

The following are effective contextual-policy decisions, including its
unsupported-context fallback to the detector, not only raw softmax classes.

| Independent test | Required changes | Frozen v1 baseline correct / false | Candidate correct / false |
| --- | ---: | ---: | ---: |
| New phrase groups, portable | 10,115 | 8,024 / 4 | 7,006 / 1 |
| New phrase groups, reference Hunspell | 10,115 | 7,678 / 24 | 6,987 / 1 |
| New focus families, portable | 1,156 | 853 / 0 | 835 / 0 |
| New focus families, reference Hunspell | 1,156 | 857 / 0 | 845 / 0 |

The candidate reduces false conversions on this sample, but misses too many
desired changes and fails the declared recall/non-regression conditions.
Simply increasing the number of examples did not produce a better general
replacement policy. The old small synthetic benchmark was insufficient to
characterize this broader domain; its published numbers remain historical,
not a claim about these new phrases or real chat quality.

## Engine replay

`engine-report.json` replays 128 distinct test-source phrases in both
initially correct and wrong layouts (256 cases per policy). Physical key
choices stay fixed; glyphs follow layout changes made by the engine.

- Whole wrong-layout phrases restored in the current stored report: detector
  80/128; frozen context v1 baseline 102/128; candidate 53/128.
- Initially correct phrases changed: 0/128 for all three policies.
- Length mismatches: zero for all three policies in this sample.

The [historical report](engine-history/71d982a8278f3325ce758cd1324e9b2246272371772ba48f69cd529fa6c5046a.json)
recorded 77/90/43 restorations and one changed correct phrase for the detector.
Those numbers remain evidence about the earlier runtime. Refreshing the same
observed cases is a runtime regression, not a new independent evaluation or
proof that the current working tree has passed verification.

This is an in-process visible-editor test using a portable dictionary,
boundary-only corrections and no automatic learning. It does not validate
native key delivery, IME, actual applications, idle timing or Enter submission.
The separate Windows/X11/AT-SPI and action-boundary E2E/regression tests remain
required. No universal absence of lost or extra symbols is claimed.

## Reproduction

```sh
PYTHONPATH=src python3 tools/context_corpus.py
PYTHONPATH=src python3 tools/context_evidence.py
PYTHONPATH=src python3 tools/train_context_v2.py verify
PYTHONPATH=src python3 tools/verify_context_v2.py --verify-frozen
```

`context_corpus.py` rebuilds the phrase split and compares its receipt;
`context_evidence.py` checks the frozen lexical cache; `train_context_v2.py
verify` refits the candidate in a temporary directory and compares the
candidate, the seal and the test report byte for byte; `verify_context_v2.py
--verify-frozen` checks the historical anchors and recomputes the four test
tracks. Training replay requires a C compiler on Linux, not a GPU. The
checked-in source snapshot and lexical cache make it independent of network
access and installed Hunspell dictionaries.

The seal, the receipts and the engine report pin the source files that
produced them. Those files have changed since (the runtime moved its numbers
into named constants, the intent model was retrained), so each pin outside
this directory is checked against an exact archived copy in
[`compatibility/generation-sources/`](compatibility/README.md), never against
the live file. The replays above run the live tools; the seal and the receipt
they reproduce keep the recorded pins, so a replay shows that the current
tools still produce the same bytes, and the pins show which bytes produced the
evidence. The lexical cache was computed once with the intent model of that
time: its archived copy is kept as evidence and is not claimed to equal the
installed one.

Fast metadata/provenance validation runs on both packaging platforms. The
historical verifier prohibits installing this rejected candidate and preserves
its comparison evidence. Acceptance of the active model belongs to the separate
feature-version-aware [shipping gate](../../tools/verify_context_model.py); it
does not require every later accepted model to equal this experiment's frozen
v1 baseline.

The next research step is richer, carefully annotated intent/context evidence
and error analysis on development data, followed by a fresh held-out test.
An "exhaustive dictionary" alone cannot determine whether a valid short word,
variable, English insertion or typo was intentional.

## Runtime regression history

The [engine report](engine-report.json) describes the runtime of its time: its
provenance names the engine, context policy and editor harness bytes that ran,
and those bytes are archived with the other sources; the reports it replaced
stay in `engine-history/`. The report is anchored by the historical
verifier, so it is frozen evidence rather than a regression check of the
current engine: `tools/evaluate_context_engine.py` replays the current engine,
which is a different program, and neither its `--verify` nor its
`--refresh-runtime` mode is part of the verification of this candidate.
Replaying an already observed test would not create new independent model
evidence or reverse the candidate's rejection.

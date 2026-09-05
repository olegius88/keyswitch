# Context policy, public-phrase experiment 1

Status: **rejected for runtime promotion**. The shipping 0.15.0 weights are
retained byte-for-byte. `candidate.json` is a research artifact, not a package
resource and not an automatic upgrade. The default application mode and
existing user settings are unchanged by this experiment.

## Data and separation

- 64,537 public CC0 source sentences: 41,503 EN and 23,034 RU.
- 63,513 retained variants in 59,721 normalized/near-duplicate groups.
- 224,693 action situations: 147,835 train; 24,746 development; 23,851
  calibration; 25,653 phrase test; 2,608 focus-lexical test.
- 6,272 source sentences (5,891 groups) remain an **unused reserve**.
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

| Independent test | Required changes | Current v1 correct / false | Candidate correct / false |
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

- Whole wrong-layout phrases restored: detector 77/128; current context v1
  90/128; candidate 43/128.
- Initially correct phrases changed: detector 1/128; v1 0/128; candidate 0/128.
- Length mismatches: zero for all three policies in this sample.

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
PYTHONPATH=src python3 tools/evaluate_context_engine.py --verify
PYTHONPATH=src python3 tools/verify_context_v2.py
```

Training replay requires a C compiler on Linux, not a GPU. The checked-in
source snapshot and lexical cache make it independent of network access and
installed Hunspell dictionaries. Fast metadata/provenance validation runs on
both packaging platforms. It requires the shipping model to equal the frozen
v1 comparison artifact, preventing accidental deployment of rejected weights.

The next research step is richer, carefully annotated intent/context evidence
and error analysis on development data, followed by a fresh held-out test.
An "exhaustive dictionary" alone cannot determine whether a valid short word,
variable, English insertion or typo was intentional.

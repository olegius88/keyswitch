# Learned completed-token boundaries, v2

Status: accepted by the predeclared calibration/test gates and selected as
the default completed-token boundary model for KeySwitch 0.18.0.
Runtime version: `boundary-v2-3b2b1af6693e`, calibrated threshold `0.95`.
The intent/KSLM, context-v1 and prefix-v1 weights are unchanged. Rejected v1
weights and its original sealed test are preserved and remain inactive.

## Task and behavior

The observer retains layout-ambiguous punctuation while a token is unfinished.
After a hard boundary or enabled idle timeout, the ranker chooses the whole
word or a word with up to eight literal trailing signs. It can abstain.
Internal comma/semicolon keys are not prematurely committed as punctuation;
selected literal suffixes retain their exact characters through replacement
and undo. A separate intent decision still determines whether to convert.
An all-symbol token does not qualify for automatic word conversion. Manual
Pause, exclusions, explicit rules and execution guards keep their priority.

This is a small linear ranking model, not an LLM. It uses lexical scores,
lengths, punctuation types and learned interactions for competing valid
spans. There are no full-word identifiers or word-specific replacement rules
in the weights. It does not read surrounding text. Its softmax score is not
a calibrated real-user success probability. It is not a prefix-action model.

## Data and separation

Examples come from the frozen public lexical resources in
[`intent_v1/sources`](../intent_v1/sources/), with source checksums and license
notice retained there. No private logs, group messages or adjacent user text
are training inputs. Author-written engine examples are regression only.

The split uses four leading physical-key characters before punctuation/typo
variants, with case/yo normalization. All test families exclude the entire
previously used public phrase vocabulary, including the boundary-v1 reserve
and prefix corpus source. This is string-family separation, not lemmatization.
The reference lexicons predate the split and may already know test words.
Fresh supervision families therefore do not mean unseen lexical knowledge.

| Partition | Rows | Physical-prefix families |
| --- | ---: | ---: |
| Train | 49,298 | 3,397 |
| Development | 8,210 | 547 |
| Calibration | 7,616 | 526 |
| Sealed test | 2,122 | 203 |

Training balances Russian endings, literal English/Russian punctuation, typos
and ambiguous cases. Every span is checked for alternative valid lexicon words,
including words outside the sampled training list. Several possible intended
outputs are labeled ambiguous, not arbitrarily forced into one spelling.
The complete [receipt](corpus.json) records counts, provenance and checksums.
Rows are synthetic variants: their count is not a measure of diverse human intent.

## Selection and results

Development loss selected epoch 50. Calibration selected threshold 0.95;
threshold 0.90 had a segmentation error and was rejected. Gates, frozen
features, weights and threshold were sealed before opening the test.
Earlier pre-test candidates are retained in `development-history/`.
No threshold or weights were adjusted after observing this test.

The predeclared gates require zero errors, at least 80% correct decisions
within each of `word_ending`, `literal_en`, `literal_ru`, and at least 1,000
test rows with 50 whole-word endings. Ambiguous inputs require abstention.

| Sealed-test stratum | Correct | Abstained | Errors |
| --- | ---: | ---: | ---: |
| Literal English punctuation | 328 | 0 | 0 |
| Literal Russian punctuation | 1,567 | 0 | 0 |
| Whole-word endings | 77 | 0 | 0 |
| Typo interventions | 24 | 0 | 0 |
| Ambiguous interpretations | 0 | 126 | 0 |
| Total | 1,996 | 126 | 0 |

[Calibration](seal.json): 7,101 correct, 515 abstained, zero errors.
[Sealed test](report.json): 1,996 correct, 126 abstained, zero errors.
Development retains **10 known errors** (nine ambiguous interpretations and
one typo), 7,582 correct and 618 abstentions. Zero test errors are not a
promise of zero errors in other words or programs; these failures remain
visible in the seal and must inform a future separately evaluated experiment.

The shared [engine regression](../boundary_v1/engine-regression.json) reuses
18 authored sequences: exact outputs improve from legacy 13/18 to active v2
18/18, with no changes to correct input, length mismatches or premature
injections in this suite. It is not independent model-quality evidence or
native Windows proof. Additional tests cover exact suffix/Unicode spaces,
manual Pause, undo, deferred Enter/Tab and real GTK/XRecord/XTEST behavior.

## Reproduce and package

Run sequentially in the Linux reference environment, with a 16 GiB memory
budget for training and without downloads:

```bash
PYTHONPATH=src python3 tools/boundary_v2_corpus.py
PYTHONPATH=src python3 tools/train_boundary_v2.py --verify
PYTHONPATH=src python3 tools/evaluate_boundary_engine.py --verify
PYTHONPATH=src python3 tools/verify_boundary_v2.py
```

`--verify` retrains and compares candidate, seal and test bytes, without
resealing or reopening a fresh test. The package verifier checks frozen
partitions, provenance, sealed metrics and exact engine evidence; both native
builds verify the bundled model checksum. It does not need a compiler.
Engine-only evidence refresh archives the previous report and keeps the
same authored cases; it cannot make observed cases independent again.

Limits: supervision is primarily EN-physical input and short synthetic
suffixes, not arbitrary multi-layout punctuation. Contextual punctuation,
multiple intended words without spaces, unseen misspellings, morphological
families, editor auto-pairing and IME behavior are not comprehensively covered.
An isolated `z` at the start of a message can still wait for context; this
ranker does not solve `z` versus `я` intention. Missing or invalid weights
fall back to the legacy boundary path with its known limitations; package
gates prohibit shipping without the exact accepted artifact.

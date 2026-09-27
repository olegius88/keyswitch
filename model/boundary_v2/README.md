# Learned completed-token boundaries, v2

Status: accepted by the predeclared calibration/test gates and installed as the
default completed-token boundary model.
Runtime version: `boundary-v2-db59e2332c9c`, feature version 3, calibrated
threshold `0.9`. It replaces `boundary-v2-3b2b1af6693e` (feature version 2),
whose sealed evidence is kept in [`generation-history/candidate1`](generation-history/candidate1/).
The policy still reads a version-2 artifact with version-2 features, so those
weights keep their earlier decisions. Rejected v1 weights and their original
sealed test are preserved and remain inactive.

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

Feature version 3 adds, for each candidate span, whether its English and its
Russian reading is a misspelling of a known word: no word itself, but a known
word with one letter missing (not its last). When no reading of the typed keys
is a known word, several such misspellings compete the way several known words
do, and the model learns those interactions separately. Lexical evidence comes
from the dictionary the engine serves: the onboard lexicons plus the packaged
Russian supplement (`src/keyswitch/resources/lexicon-supplement-ru_RU.json`,
filtered by `tools/filter_lexicon_supplement.py`, receipt beside it).

## Data and separation

Examples come from the frozen public lexical resources in
[`intent_v1/sources`](../intent_v1/sources/), with source checksums and license
notice retained there. Intended words are drawn from the onboard lexicons; their
features and the alternative valid readings are computed with the served
dictionary. No private logs, group messages or adjacent user text are training
inputs. Author-written engine examples are regression only.

The split uses four leading physical-key characters before punctuation/typo
variants, with case/yo normalization. All test families exclude the entire
previously used public phrase vocabulary, including the boundary-v1 reserve
and prefix corpus source. This is string-family separation, not lemmatization.
The reference lexicons predate the split and may already know test words.
Fresh supervision families therefore do not mean unseen lexical knowledge.
Earlier boundary-v2 splits partitioned the same lexicon words differently: the
first generation's test was exposed, two later candidates were rejected on
calibration and one was superseded before their tests were read (records in
`development-history/` and `generation-history/`), so these families overlap
their partitions.

| Partition | Rows | Physical-prefix families |
| --- | ---: | ---: |
| Train | 48,981 | 3,391 |
| Development | 8,144 | 529 |
| Calibration | 7,929 | 549 |
| Sealed test | 2,093 | 201 |

Training balances Russian endings, literal English/Russian punctuation,
misspelled words and ambiguous cases. A misspelling drops one key other than
the last of a sampled word, is categorised like a correctly spelled word
(ending, Russian literal, English literal) and is followed by one literal suffix
drawn from the same set as for correctly spelled words. It is generated only
where no reading of the typed keys is a known word. Every span is checked for
alternative valid readings: known words, including words outside the sampled
list, and, where no reading is a known word, misspellings of known words.
Several possible intended outputs are labeled ambiguous, not arbitrarily forced
into one spelling. The complete [receipt](corpus.json) records counts,
provenance and checksums. Rows are synthetic variants: their count is not a
measure of diverse human intent.

## Selection and results

Development loss selected epoch 50. Calibration selected threshold 0.9, the
first threshold of the declared list, with zero calibration errors. Gates,
frozen features, weights and threshold were sealed before opening the test.
Earlier pre-test candidates are retained in `development-history/`. No
threshold or weights were adjusted after observing this test.

The predeclared gates require zero errors, at least 80% correct decisions
within each of `word_ending`, `literal_en`, `literal_ru`, and at least 1,000
test rows with 50 whole-word endings. Ambiguous inputs require abstention.

| Sealed-test stratum | Correct | Abstained | Errors |
| --- | ---: | ---: | ---: |
| Literal English punctuation | 472 | 0 | 0 |
| Literal Russian punctuation | 1,352 | 0 | 0 |
| Whole-word endings | 64 | 0 | 0 |
| Misspelled words (ending, Russian and English literal) | 19 | 0 | 0 |
| Ambiguous interpretations | 0 | 186 | 0 |
| Total | 1,907 | 186 | 0 |

[Calibration](seal.json): 7,178 correct, 751 abstained, zero errors.
Development at the selected threshold: 7,350 correct, 794 abstained, zero errors.
[Sealed test](report.json): 1,907 correct, 186 abstained, zero errors.
Zero test errors are not a promise of zero errors in other words or programs.

The shared [engine regression](../boundary_v1/engine-regression.json) reuses
18 authored sequences: exact outputs improve from legacy 13/18 to active v2
18/18, with no changes to correct input, length mismatches or premature
injections in this suite. It is not independent model-quality evidence or
native Windows proof. Additional tests cover exact suffix/Unicode spaces,
manual Pause, undo, deferred Enter/Tab and real GTK/XRecord/XTEST behavior.

## Reproduce and package

Run sequentially in the Linux reference environment, without downloads:

```bash
PYTHONPATH=src python3 tools/filter_lexicon_supplement.py --verify
PYTHONPATH=src python3 tools/boundary_v2_corpus.py
PYTHONPATH=src python3 tools/train_boundary_v2.py --verify
PYTHONPATH=src python3 tools/evaluate_boundary_engine.py --verify
PYTHONPATH=src python3 tools/verify_boundary_v2.py
```

The corpus receipt pins the lexical contract of the intent configuration (the
word lists and dictionaries it names), the served supplement and the code and
constant values the rows are computed from; another intent generation with the
same lexical contract leaves it valid. `--verify` retrains and compares
candidate, seal and test bytes, without resealing or reopening a fresh test.
The package verifier checks frozen partitions, provenance, sealed metrics and
exact engine evidence; both native builds verify the bundled model checksum.
It does not need a compiler; `--verify` does. Engine-only evidence refresh
archives the previous report and keeps the same authored cases; it cannot make
observed cases independent again.

Limits: supervision is primarily EN-physical input and short synthetic
suffixes, not arbitrary multi-layout punctuation. A misspelling here is one
missing key; substituted, doubled or swapped keys and several typos in one word
are not modelled. Capital letters on punctuation keys (`<`, `>`, `:`, `"`, `{`,
`}`, `~`) and hyphenated words are not covered by the features. Contextual
punctuation, multiple intended words without spaces, editor auto-pairing and
IME behavior are not comprehensively covered. An isolated `z` at the start of a
message can still wait for context; this ranker does not solve `z` versus `я`
intention. Missing or invalid weights fall back to the legacy boundary path
with its known limitations; package gates prohibit shipping without the exact
accepted artifact.

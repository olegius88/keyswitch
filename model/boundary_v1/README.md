# Completed-token boundary experiment 1

Status: **rejected, inactive**. `candidate.json` is research evidence, not an
installed application model. No `resources/models/boundary-v1.json` is shipped.
The default punctuation policy and context-v1 weights remain unchanged.

## What was implemented

In the experimental engine path, a key such as `,`, `.`, `;`, `[` or `]` can
remain part of the tracked token until a hard boundary or idle evaluation.
This avoids asking a completed-word classifier to cut an unfinished prefix.
At that point, the boundary ranker compares up to nine spans: the whole token
or a word followed by one to eight literal punctuation characters. Uncertain
segmentation forbids an automatic rewrite, not the user's manual Pause.
The existing intent/context policy still decides whether to convert the
selected word. This ranker does not replace the contextual action model.

The executor preserves the selected punctuation in its original keyboard
layout, then the real boundary, then queued input. Deferred Enter is not
counted as a character and is delivered after the correction. Undo retains
the same literal tail. This contract has in-process editor and Windows/X11
API-mock tests; it is not evidence of a native Windows GUI run.

## Corpus, features and evidence

The source is the CC0 snapshot already stored under
[`context_v2/sources`](../context_v2/sources/). This experiment uses words from
the **6,272-sentence reserve** that context-v2 itself did not train/test on.
Consequently that reserve is no longer globally unused. The old context-v2
reports remain historical records of that experiment, not of this one.

Words and punctuation interventions are split before training by hashed
four-character physical-key families, with case/yo normalization. This is a
string-family split, not morphological lemmatization. Labels are synthetic;
no private logs, application text, Telegram contents or user-intent labels
are included. Lexicons predate the split and may already contain test words.

The ranker uses candidate length, preserved suffix length, lexical frequency,
exact dictionary membership, invalid-character ratio and character-language
scores. It does not memorize full word IDs or read surrounding text. It is a
small linear softmax ranker, not an LLM. A score is not a measured probability
of correctness for an arbitrary user.

| Partition | Examples | Physical-prefix families |
| --- | ---: | ---: |
| Train | 78,960 | 5,139 |
| Development | 12,017 | 775 |
| Calibration | 11,822 | 751 |
| Sealed test | 11,992 | 763 |

Epoch selection uses development loss; threshold selection uses calibration
only. The test was opened after the candidate/threshold seal. Prior pre-test
development seals are retained in `development-history/`. After the test is
observed, changing data, weights or thresholds requires a new experiment.

The [sealed test](report.json) has **1,993 correct decisions, 9,998 abstentions
and one wrong punctuation interpretation**. It fails both predeclared gates:
zero segmentation errors and at least 80% decided correctly. In particular,
only 43 test rows are whole-word endings and 11,949 are literal punctuation;
the large row count is not broad or balanced linguistic coverage.

The [authored engine regression](engine-regression.json) separately compares
18 physical-input sequences. Experimental buffering eliminates premature
injections in this small suite, but exact outputs fall from 13/18 to 12/18
because of missed corrections. Both variants retain correct inputs and text
lengths here. This is a regression comparison, not another independent test,
not a real-user error rate and not permission to activate the candidate.

## Reproduction

Run sequentially in the Linux reference environment, without downloads:

```bash
PYTHONPATH=src python3 tools/train_boundary_model.py --verify
PYTHONPATH=src python3 tools/verify_boundary_model.py
PYTHONPATH=src python3 tools/evaluate_boundary_engine.py --verify
```

`--verify` retrains and compares the candidate, seal and report byte-for-byte.
The fast verifier rejects altered provenance/counts and installation of
unapproved weights; CI and package builds invoke it. Training and test entry
points refuse to overwrite an observed test. The explicit promotion command
also refuses this rejected candidate.

Known gaps: unannotated real intention, uncommon/misspelled words, contextual
punctuation meaning, morphological family separation, IME/dead-key composition,
and multiple intended words with no spaces. The experimental path has no
end-user setting yet; it is injected explicitly by the integration tests.

# Automatic layout detection research

Sources were verified on 26 August 2026.

## What mature solutions use

| Solution | Verified mechanism | Adopted in KeySwitch |
|---|---|---|
| EveryLang | Character-sequence checks; rules learned after a configurable number of manual last-word switches; explicit short-word, Enter and application-exclusion policies | Manual confirmations, configurable length/boundaries and exclusions |
| Caramba Switcher | Local self-learning; erasing a bad correction and retyping creates an exception; its author calls out code and abbreviations as difficult cases | Undo records a rejection; code and abbreviations are guarded before statistical scoring |
| XNeur | Dictionary, Enchant/Aspell morphology, typo suggestions, impossible 2/3-grams and training after repeated manual conversions | Frequency dictionary, Hunspell, character 2/3/4-grams, one-extra-letter resilience and confirmations |
| KeyboardSwitcher | Pair-specific neural score, adaptive thresholds, consecutive agreement, single-character deletion for typos, context, exceptions and calibration | A lightweight deterministic ensemble, context, deletion evidence, rules and rejections |
| RSwitcher | Bigrams/trigrams, impossible sequences, dictionaries, code guards, special short-word lists and adaptive counts | Smoothed n-grams, dictionaries, code guards and a conservative short-word policy |
| UASwitcher | Precision-first handling: skip short words, URLs, digits, internal punctuation, ALL-CAPS, camelCase, terminals/IDEs/password managers; always/never lists and Undo | The same error-cost policy, token guards, application exclusions, rules/rejections and Undo |
| Kanningem Switcher | Frequency tables, a large Hunspell dictionary, protected developer terms, user dictionary and application exclusions | Onboard frequencies, system Hunspell dictionaries, protected technical tokens and exclusions |

## The KeySwitch model

KeySwitch decodes every physical key sequence in each supported XKB group and
then applies these signals in order:

1. An explicit user rejection or a confirmed rule.
2. Structural guards for URLs, paths, numbers, identifiers, `camelCase`,
   ALL-CAPS, mixed scripts and protected technical terms.
3. Source and target lookup in the Onboard frequency lexicons.
4. System Hunspell morphology for valid inflections and derived forms.
5. Smoothed character 2-, 3- and 4-gram scores. Unknown words are switched
   automatically only with a large margin.
6. A modest adjustment from the previous word pair and the recent language in
   the same application.
7. For tokens of at least five characters, a test for strong dictionary
   evidence after removing one accidentally duplicated character.

A valid source word is never changed automatically. If both decodings are
valid (`here` and `руку` share one physical sequence), the case is inherently
ambiguous and is left unchanged; the manual conversion hotkey remains
available.

## Learning and privacy

KeySwitch learns only from explicit actions. By default, two identical manual
conversions create a rule; the threshold is configurable from one to five.
Undoing an automatic correction records a rejection for that source token and
direction. `learning.json` contains only those tokens, directions and counters,
never the ordinary input stream. Previous-word context is memory-only, expires
after 45 seconds and is isolated by application `WM_CLASS`.

## Reproducible evaluation

Run:

```bash
PYTHONPATH=src tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
```

On the verified system with `onboard-data`, `libhunspell-1.7-0`,
`hunspell-en-us` and `hunspell-ru`, the current tree produces:

| Intended word language | Precision | Recall | Specificity |
|---|---:|---:|---:|
| EN | 100.00% | 99.67% | 100.00% |
| RU | 100.00% | 99.86% | 100.00% |

An additional broad slice of 10,000 evenly selected stems from each Hunspell
dictionary produces:

| Intended word language | Precision | Recall | Specificity |
|---|---:|---:|---:|
| EN | 99.96% | 94.29% | 99.96% |
| RU | 100.00% | 99.22% | 100.00% |

The run evaluates 10,000 frequent and 10,000 broad-dictionary words per
language both correctly and under the wrong layout, plus 196 curated
inflections, typos, ambiguous cases and technical tokens. This is a regression
corpus derived from the project's own lexical sources, not an independent study
of real user text. Its figures therefore must not be compared directly with
marketing figures for Punto Switcher, EveryLang or any other product.

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
- [arXiv: Language Detection Engine for Multilingual Texting on Mobile Devices](https://arxiv.org/abs/2101.03963)
- [Hunspell official repository](https://github.com/hunspell/hunspell)

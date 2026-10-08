"""Thresholds and limits of the runtime decisions: engine, context policy, early switch, learning."""

from __future__ import annotations

from typing import Final

# The shortest token word_shaped() can judge: with one character its first- and last-letter checks
# look at the same position.
MINIMUM_SHAPED_TOKEN_CHARACTERS: Final[int] = 2
# Confidence an early (prefix) switch reports.
EARLY_SWITCH_CONFIDENCE: Final = 15.0
# Most strokes the engine keeps for one word.
MAX_WORD_STROKES: Final = 256
# Words left as typed at a space that a later converted word may still take along: the first
# words of a message have nothing before them to tell their layout by (`hey here` typed in the
# Russian layout is `рун руку`, two Russian words, until `ерун` shows it was `they`).
KEPT_WORDS_TAKEN_ALONG: Final = 2
# Most Latin words an English term inside a Russian phrase may span before a lone Russian letter
# typed in the English layout is read as that letter (`в git diff d` is `в git diff в`).
STRANDED_TERM_MAX_WORDS: Final = 3
# What the Russian layout may print right after a one-letter Russian word that opens a message, for the
# message-start rule (word_decision.opening_letter_decision) to read it as that word: a space, nothing,
# or a sign that ends a clause (`а, понятно`, `а? что`, `о! круто`, `а) пункт`, `а-а-а`). A key that
# prints a letter or `№` there makes the letter part of a token, an initial or a name: `J. Smith` read
# in the Russian layout is `Ою Smith`, `C# rocks` is `С№ rocks`, and English opens with those.
OPENING_LETTER_FOLLOWING_SIGNS: Final[frozenset[str]] = frozenset(",.?!:;)-")
# Fewest letters left of a word once the letters at its end that are signs in the other layout are
# set aside (`руддщб` is `hello,`): `чё` without `ё` is a lone `ч`, and one letter is no word to judge.
REPLAYED_SIGNS_MIN_STEM_LETTERS: Final = 2
# A fragment of this many letters that a pause leaves is the start of a word being typed, not a word,
# when at least PAUSE_WORD_START_MIN_WORDS words of its own language's lexicon begin with it and its other
# reading is no word of the other language's prose: `иг` in `первую иг` (226 Russian words) and `фл` of
# `флагом` (344) became `bu` and `ak` at a pause and came back once the word was finished (owner's logs,
# 03.10 and 08.10.2026). `ша` typed for `if` begins 663 Russian words, and English prose has `if`. The
# Russian words typed in the English layout that a pause converts (`gj` is `по`, `lf` is `да`) begin at
# most 39 English words. Such a fragment is decided at its boundary, as every word is.
PAUSE_WORD_START_LETTERS: Final = 2
PAUSE_WORD_START_MIN_WORDS: Final = 100
# Input events the engine queues before the producer blocks.
ENGINE_EVENT_QUEUE_MAX_SIZE: Final = 4096
# Per-application remembered context words; oldest is dropped past this cap.
MAX_REMEMBERED_APPLICATION_CONTEXTS: Final = 32
# A boundary is natural (not just a configured minimum length) once the word is this long and its
# own-language score is at least this uncertain.
NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS: Final = 4
NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR: Final = -0.25
# A correction no model scored - one the user triggered (manual toggle, undo) or one an
# application convention asks for (Telegram's quote shown as "@") - is certain; this stands in
# for the confidence a model would have reported.
UNSCORED_CORRECTION_CONFIDENCE: Final = 99.0
# Context policy shared by the serving detector and the intent trainer and evaluator, which replay
# its arithmetic offline: a LanguageModel context_score lies in [minimum, maximum]; the difference of
# the target and source context scores is multiplied, then the recent context language adds the
# bonus when it is the target's and takes the penalty when it is the source's.
CONTEXT_SCORE_MINIMUM: Final[float] = 0.0
CONTEXT_SCORE_MAXIMUM: Final[float] = 1.0
CONTEXT_DELTA_MULTIPLIER: Final[float] = 1.75
CONTEXT_TARGET_GROUP_BONUS: Final[float] = 0.55
CONTEXT_SOURCE_GROUP_PENALTY: Final[float] = 0.3
# Confidence of a conversion a confirmed user rule forces, unless the score margin is larger.
CONFIRMED_RULE_MIN_CONFIDENCE: Final = 20.0
# Heuristic fallback, used when the intent model does not decide. A target word a lexicon knows
# needs the confidence threshold as its margin, relieved by the relief per character for each
# character past the relief start (by at most one), but never below the minimum margin; a target only
# Hunspell knows needs the extra margin on top.
HEURISTIC_KNOWN_TARGET_RELIEF_START_CHARACTERS: Final = 3
HEURISTIC_KNOWN_TARGET_RELIEF_PER_CHARACTER: Final = 0.18
HEURISTIC_KNOWN_TARGET_MIN_MARGIN: Final = 0.65
HEURISTIC_SPELL_ONLY_TARGET_EXTRA_MARGIN: Final = 0.15
# One accidental extra character: a word at least this long whose target reading is known after
# dropping one character converts when that margin beats the confidence threshold by this much.
HEURISTIC_TYPO_DELETION_MIN_CHARACTERS: Final = 5
HEURISTIC_TYPO_DELETION_EXTRA_MARGIN: Final = 0.5
# An unknown word judged by character n-grams needs the confidence threshold plus the extra margin,
# relieved by the relief per character past the relief start (by at most the cap) and reduced once
# more in aggressive mode, but at least the threshold plus the minimum extra margin. The source must
# score at most, and the target at least, the given n-gram bounds, and the word must be at least as
# long as the given length; aggressive mode has its own bounds. The short-word policy
# (keyswitch.short_words) holds a short, dictionary-only source to the same default source bound.
HEURISTIC_NGRAM_EXTRA_MARGIN: Final = 2.4
HEURISTIC_NGRAM_RELIEF_START_CHARACTERS: Final = 4
HEURISTIC_NGRAM_RELIEF_PER_CHARACTER: Final = 0.32
HEURISTIC_NGRAM_RELIEF_CAP: Final = 2.0
HEURISTIC_NGRAM_AGGRESSIVE_MARGIN_REDUCTION: Final = 0.75
HEURISTIC_NGRAM_MIN_EXTRA_MARGIN: Final = 0.25
HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX: Final = -1.1
HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX_AGGRESSIVE: Final = -0.65
HEURISTIC_PLAUSIBLE_TARGET_NGRAM_MIN: Final = -1.25
HEURISTIC_PLAUSIBLE_TARGET_NGRAM_MIN_AGGRESSIVE: Final = -2.0
HEURISTIC_NGRAM_MIN_CHARACTERS: Final = 5
HEURISTIC_NGRAM_MIN_CHARACTERS_AGGRESSIVE: Final = 4
# The structural guard protects a token longer than the convertible length, an all-capital token of
# at least the acronym length, and a token with a run of this many repeated characters.
MAX_CONVERTIBLE_TOKEN_CHARACTERS: Final = 64
ACRONYM_MIN_LETTERS: Final = 2
REPEATED_CHARACTER_RUN: Final = 4
# The runtime short-word policy (keyswitch.short_words, which exports these names). The curated
# trusted list decides normalised words of at most this many letters.
TRUSTED_SHORT_WORD_MAX_LENGTH: Final[int] = 2
# A dictionary-only conversion of a word up to this long (the longer normalised reading) is withheld
# while its source reads naturally, unless the recent context favours the target language.
NATURAL_SOURCE_MAX_LENGTH: Final[int] = 4
# A two-letter trusted entry converts on its own when its exact target reading is at least this
# frequent and, counting one more of each, at least the minimum ratio more frequent than the source;
# after a word in the target language the context ratio is enough.
TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY: Final[int] = 10_000
TRUSTED_SHORT_WORD_MINIMUM_RATIO: Final[float] = 100.0
TRUSTED_SHORT_WORD_CONTEXT_RATIO: Final[float] = 1.0
# Confidence of a one-letter trusted word converted on its context alone: the frequency lists hold no
# single letters, so there is no ratio to report.
SINGLE_LETTER_CONFIDENCE: Final[float] = 1.0

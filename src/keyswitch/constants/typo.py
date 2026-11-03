"""Typo correction at a word's boundary: its thresholds, its channel probabilities and its resource format."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE

# Fewest letters a word needs to be corrected: a short word lies one slip away from many others (`кот`, `rat`, `рот`),
# and no one of them is clearly the word meant.
TYPO_MIN_LETTERS: Final = 5
# Fewest occurrences in the counted list (OpenSubtitles 2018 via FrequencyWords) a corrected word needs: a rarer form
# is too uncertain a guess to type over the user's word. tools/evaluate_typo_correction.py measures other floors too.
TYPO_CANDIDATE_MIN_COUNT: Final = 20
# How many times the posterior of the best candidate must exceed the posteriors of all the other candidates together:
# a word with two plausible readings is left as typed.
TYPO_POSTERIOR_RATIO: Final = 3.0
# Natural logarithms of the probability of one particular slip (the noisy channel), by the operation that undoes it.
# Only their differences change a decision. Tuned on the tuning half of the synthetic development typos of
# tools/evaluate_typo_correction.py (equal shares of the four operations: no source we could open gives the shares
# for Russian typing): a swap of two neighbouring letters is the likeliest single slip, a letter left out next, a
# letter typed with a neighbouring key and a letter typed in excess (a key pressed twice or one brushed beside the
# key meant) after them.
TYPO_TRANSPOSITION_LOG_PROBABILITY: Final = -1.4
TYPO_INSERTION_LOG_PROBABILITY: Final = -2.5
TYPO_DELETION_LOG_PROBABILITY: Final = -5.0
TYPO_SUBSTITUTION_LOG_PROBABILITY: Final = -4.0
# Letters a transposition swaps.
TYPO_TRANSPOSED_LETTERS: Final = 2
# The physical key rows of layouts.US_KEYS and layouts.RU_KEYS, in order: the key left of `1`, then the three letter
# rows. Each lower row sits half a key to the right of the one above it, so a key touches the key above it and the one
# after that; the key of the top row stands above Tab and touches no letter.
KEYBOARD_ROW_LENGTHS: Final = (1, 13, 11, 9)

# Forms the training split of the ru.stackoverflow dump uses at least this many times are known forms too
# (tools/build_typo_frequency.py): programmers' words subtitles lack (`деплой`, `закомитить`, `unselected`). Chosen on
# the development splits (tools/evaluate_typo_correction.py vocabulary): the least count at which no operation loses
# more than TYPO_VOCABULARY_MAX_RECALL_LOSS of its synthetic typos, which the dump holds too when its writers make
# the same slips. At it the forum's English false corrections fell by a tenth and those of code by a quarter.
TYPO_VOCABULARY_MIN_COUNT: Final = 20
# Resource format (tools/build_typo_frequency.py writes it, typo_correction.TypoLexicon reads it).
TYPO_FREQUENCY_SCHEMA_VERSION: Final = 1
# Largest typo frequency file loaded; the shipped ones are about two mebibytes each.
TYPO_FREQUENCY_MAX_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Forms with at least this many occurrences are stored with their counts: the candidates, and the readings they compete
# with. A rarer form is only known to occur.
TYPO_STORED_MIN_COUNT: Final = 5
# Forms with at least this many occurrences count as words that occur: a typed word that occurs is never corrected,
# however rare, so a rare inflected form survives. One is every form the list holds.
TYPO_OCCURRENCE_MIN_COUNT: Final = 1
# Stored counts are natural logarithms times this scale, rounded, one byte each: the decision compares ratios, and an
# eighth of a natural logarithm (thirteen per cent) is finer than any threshold it uses.
TYPO_COUNT_LOG_SCALE: Final = 8
# Largest stored count value: one byte.
TYPO_COUNT_MAX_STORED: Final = 255
# The share of the forms that merely occur which the membership filter wrongly reports for a form it does not hold.
# Such a typed word is left as typed, the safe direction.
TYPO_OCCURRENCE_FALSE_POSITIVE_RATE: Final = 0.01
# Bytes of the SHA-256 digest of a form read as each of the two hashes the membership filter positions are made from.
TYPO_OCCURRENCE_HASH_BYTES: Final = 8
# The sections after the header line, in order: the stored forms and their counts (each compressed with zlib), then
# the membership filter's bits.
TYPO_SECTIONS: Final = ("forms", "weights", "seen")
# zlib level the build tool compresses the sections with: the smallest file; the receipt pins the decompressed bytes, so
# another zlib build that compresses them otherwise still verifies.
TYPO_ZLIB_LEVEL: Final = 9
# The byte that ends each stored form; the letters are coded from one up, in the order of their alphabet.
TYPO_FORM_SEPARATOR: Final = 0
# Measurement (tools/evaluate_typo_correction.py). One eligible word in this many of the development texts gets a
# synthetic typo, chosen by a hash of its place.
TYPO_EVALUATION_SAMPLE_MODULUS: Final = 32
# The decision parameters the tuning tries on the tuning half of the synthetic typos: the least count of a corrected
# word, the posterior ratio, and the slip probabilities of deletion, insertion and substitution (transposition stays
# at TYPO_TRANSPOSITION_LOG_PROBABILITY: only differences between operations change a decision). Every least count
# is measured and reported; the tuning chooses the others at TYPO_CANDIDATE_MIN_COUNT, the floor of the design.
TYPO_EVALUATION_MIN_COUNTS: Final = (10, 20, 50, 100)
TYPO_EVALUATION_RATIOS: Final = (1.0, 1.5, 2.0, 3.0, 5.0, 10.0, 20.0)
TYPO_EVALUATION_DELETION_LOG_PROBABILITIES: Final = (-3.7, -5.0, -6.5, -8.0)
TYPO_EVALUATION_INSERTION_LOG_PROBABILITIES: Final = (-1.4, -2.5, -3.7)
TYPO_EVALUATION_SUBSTITUTION_LOG_PROBABILITIES: Final = (-3.0, -4.0, -5.0)
# The most corrections to a wrong word a tuned policy may make, as a share of its corrections: the owner's acceptance.
# The tuning holds the upper bound of that share's confidence interval to it (Wilson, this many standard deviations,
# about 95 per cent), so that a policy at the edge on the tuning half does not pass it on the other.
TYPO_EVALUATION_MAX_WRONG_SHARE: Final = 0.02
TYPO_EVALUATION_WILSON_Z: Final = 1.96
# Words of clean text per false correction the owner's acceptance allows, on Russian and English prose and on the
# Russian Stack Overflow texts; code allows none.
TYPO_ACCEPTED_PROSE_WORDS_PER_FALSE_CORRECTION: Final = 10_000
TYPO_ACCEPTED_FORUM_WORDS_PER_FALSE_CORRECTION: Final = 5_000
# The counts the vocabulary floor is chosen among, and the most of an operation's synthetic typos (those no list
# holds) the vocabulary may make known at the chosen one.
TYPO_EVALUATION_VOCABULARY_MIN_COUNTS: Final = (2, 3, 5, 10, 20, 50)
TYPO_VOCABULARY_MAX_RECALL_LOSS: Final = 0.02
# Seconds within which a stream that follows an erased one in the same application holds the words typed again.
TYPO_EVALUATION_RETYPE_WINDOW_SECONDS: Final = 30
# The language whose typos are corrected only with detection.correct_typos_english on: in Russian technical text and
# code an English word nothing knows is far more often an identifier or a term than a slip (`runat`, `unselected`):
# the corrector changed 11.5 English words in 10 000 of ru.stackoverflow development text and 9.6 of code, against
# 0.65 Russian words in 10 000 of Tatoeba.
TYPO_ENGLISH_LOCALE: Final = "en_US"
# The letters of each language's forms, in their coding order.
TYPO_ALPHABETS: Final = {
    "en_US": "abcdefghijklmnopqrstuvwxyz",
    "ru_RU": "абвгдеёжзийклмнопрстуфхцчшщъыьэюя",
}

# The personal dictionary (personal_words.py): a word left as typed this many times is the user's own and never
# corrected again; one whose correction the user took back is theirs at once.
PERSONAL_WORD_MIN_COUNT: Final = 2
# Most of the user's own words kept, and most words counted on their way there; past either, the entry used longest
# ago goes. The file stays well under a mebibyte.
PERSONAL_WORDS_MAX: Final = 20_000
PERSONAL_WORD_COUNTS_MAX: Final = 5_000
# A word is stored as the first hexadecimal digits of the SHA-256 of a per-installation salt and the word: 64 bits,
# so two words of a full dictionary share a hash with odds below one in ten billion.
PERSONAL_WORD_HASH_HEX_CHARACTERS: Final = 16
# Random bytes of the salt, drawn once per installation and kept in the same file.
PERSONAL_WORD_SALT_BYTES: Final = 16
PERSONAL_WORDS_SCHEMA_VERSION: Final = 1

"""Facts about the runtime model artifacts: feature versions, weight caps, thresholds."""

from __future__ import annotations

from typing import Final

# The context model's features. Feature schema 3 (context_action_features) and the frozen feature
# schema 2 (context_model.extract_context_features, read by the installed context-v1 model and
# compared with its archived source by tools/verify_context_v2_history.py) share the values marked
# "schemas 2 and 3": changing one of them changes the shipped model's features too.
# Character n-gram orders extracted around each text span.
ACTION_FEATURE_NGRAM_ORDERS: Final = (1, 2, 3, 4)
# Cap on one accumulated character n-gram feature's weight.
ACTION_FEATURE_CHARACTER_WEIGHT_CAP: Final = 8.0
# A character feature name is `label:char:direction:order:text`: its n-gram text follows this many
# separators, so a split at most this many times leaves it whole at this index.
ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX: Final = 4
# A WordScore's value, ngram_score and the value delta between two readings share the same
# log-probability-like scale; clamped to this range and divided by it so the feature stays in [-1,
# 1]. Schemas 2 and 3 (schema 2 bounds only the score delta).
ACTION_FEATURE_WORD_SCORE_BOUND: Final = 10.0
# Magnitude a raw n-gram score is clamped to and divided by.
ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND: Final = 32.0
# Frequencies are log-scaled against this cap so one very common word cannot dominate the feature.
ACTION_FEATURE_FREQUENCY_CAP: Final = 10**12
# `after_origin == "planned_next_conversion"` is only reachable for a short waiting word with a
# short, single-line planned right context; it follows ACTION_DEFERRED_WORD_MAX_CHARACTERS, the
# longest word the action model defers and the engine re-decides with its planned next word. With
# two, a three-letter `tot` the model asked to wait for `привет` could never be asked again.
PLANNED_CONTEXT_WORD_MAX_CHARACTERS: Final = 3
# Longest single-line planned right context a planned_next_conversion origin allows.
PLANNED_CONTEXT_AFTER_MAX_CHARACTERS: Final = 64
# `after_origin == "kept_next_word"`: a waiting word of at most this many characters asked once more
# with its next word, which stayed as typed in the same layout (KeySwitchEngine._decide_with_kept_neighbour).
# Five covers the terms the owner's typing left behind that way (`шзм6` for `ipv6`, `тзь` for `npm`).
KEPT_CONTEXT_WORD_MAX_CHARACTERS: Final = 5
# Every feature of a kept-neighbour question carries this prefix: the question has weights of its own,
# trained on frames of its own, and never moves the weights of the word decided at its boundary.
KEPT_FEATURE_PREFIX: Final = "kept|"
# A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS, after
# text with more Latin letters than Cyrillic ones, has its features once more under this prefix: the
# capitals head, fitted onto a frozen model with its every other weight fixed, answers that class
# alone; a model without such weights decides as it did. The class is the shape of a cited
# abbreviation (`BP`, `NASA`) in Latin-script text; a longer word in capitals is a word typed with
# Caps Lock (`PFGHJC` for `ЗАПРОС`), which the first head turned into a keep.
CAPITALS_FEATURE_PREFIX: Final = "caps|"
CAPITALS_HEAD_MIN_LETTERS: Final = 2
CAPITALS_HEAD_MAX_LETTERS: Final = 5
# A token of ALONE_HEAD_LETTERS letters with no letter on either side in the field, sent or left at a
# pause - a one-word message - has its features once more under this prefix: the lone-word head,
# fitted onto a frozen model, answers that class alone. Two letters are where the
# owner's typing left most lone words in the wrong layout (`гш` for `ui`, `зщ` for `po`), because
# every such frame of the corpus was deferred (context_deferral).
ALONE_FEATURE_PREFIX: Final = "alone|"
ALONE_HEAD_LETTERS: Final = 2
# The lone-word head reads the ratio of a word's two counts as the difference of their binary orders of
# magnitude, clamped to this bound either way.
ALONE_COUNT_RATIO_BOUND: Final = 8
# A word of letters of at least START_HEAD_MIN_LETTERS, letters in both layouts, with no letter on either
# side in the field - the first word of a message, decided before anything follows it - has its features
# once more under this prefix: the message-start head, fitted onto a frozen model, answers that class
# alone. The shared features read how often each reading occurs in Russian and in English prose only for
# a word with text around it, so the first word was decided by its letters and the term tables, in which
# neither reading of a rare Russian word occurs: a correctly typed gerund opening a message turned into
# its Latin keys, whose ending reads like an English past tense. The head reads both prose counts too
# (the same ratio bound as the lone-word head). Two letters belong to the lone-word head; three letters
# alone are deferred where both readings are plausible (context_deferral), and a head fitted on them
# turned the owner's `чук`, `дес` and `дые` into Latin, so the class starts at four.
START_FEATURE_PREFIX: Final = "start|"
START_HEAD_MIN_LETTERS: Final = 4
# The message-start head also reads the detector's score delta in bands of this width, up to this many
# bands either way: the shared feature is clamped at ACTION_FEATURE_WORD_SCORE_BOUND, where nearly every
# correctly typed first word and every rare Russian word the model converted at once both lie, and a
# band of its own lets the head answer that end and the other one apart.
START_DELTA_BAND_WIDTH: Final = 2.5
START_DELTA_BANDS: Final = 6
# A word of one letter, a letter in both layouts, right after a word whose letters are all Latin, has a
# head of its own (context_action_features.letter_question): the class's features once more under this
# prefix, and how often the word before it and the word after it occur in English prose and in Russian
# technical text. `nats b redis` is `nats и redis` typed in the wrong layout, `plan b then` stays.
LETTER_FEATURE_PREFIX: Final = "letter|"
# A Cyrillic abbreviation in capitals that Russian technical text uses and the lexicon does not hold, whose Latin
# reading is letters too, after text with more Cyrillic letters than Latin ones has evidence of its own under this
# prefix: the abbreviation head (context_action_features.abbreviation_question), fitted onto a frozen model, answers
# that class alone. After Russian prose the frozen model turned such abbreviations into the Latin readings they spell
# (`страны ЕС приняли` became `страны TC приняли`, `поздравляю с НГ` became `поздравляю с YU`, 0.37-0.42): the corpus
# framed every Latin abbreviation typed in the Russian layout after Russian prose with the convert label, whatever
# its keys spelled. The capitals head answers the same shape after Latin text.
ABBREVIATION_FEATURE_PREFIX: Final = "abbr|"
# Letters no Russian word starts with, so no abbreviation of Russian words holds them: their keys are the Latin
# `M`, `S`, `]` and `Q` (`ДДЬ` is `LLM`, `ЫЫР` is `SSH`, `ЙФ` is `QA`).
ABBREVIATION_NON_INITIAL_LETTERS: Final = frozenset("ЪЬЫЙ")
# The same attested abbreviation after text with more Latin letters than Cyrillic ones has evidence of its own under
# this prefix: the Latin-context abbreviation head (context_action_features.latin_abbreviation_question). There the
# capitals head and the shared features answered alone and turned the abbreviation into the Latin reading its keys
# spell whatever its counts (`/designv2 ТЗ` became `/designv2 NP`, the owner's typing of 08.10.2026; `ЛС`, `ТГ`
# and `НГ` the same way).
LATIN_ABBREVIATION_FEATURE_PREFIX: Final = "labbr|"
# A token typed in the Russian layout whose Cyrillic reading the lexicon does not hold, of letters and digits, at
# least UNKNOWN_HEAD_MIN_CHARACTERS long and not all capitals, after text with more Cyrillic letters than Latin ones,
# has the shared features once more under this prefix, with evidence of its own: the unknown-token head
# (context_action_features.unknown_question). It is a technical term typed in the wrong layout (`цуиоы` for `webjs`,
# `мгуоы` for `vuejs`, `реьч` for `htmx`) or a Russian word the lexicon lacks (`ватсап`, a typo, a name). The frozen
# model kept the terms there: a token no table counts after Russian prose read to it as Russian slang (the owner's
# typing in a chat input of VS Code, 13.10.2026; 15 of 16 short terms stayed Cyrillic after `затрагивает только`).
UNKNOWN_FEATURE_PREFIX: Final = "unknown|"
# Four and more: a Russian token of three letters no table counts is slang or a typo as often as a term typed in the
# wrong layout (`зач`, `мты`, `нзч` stayed Russian in the owner's typing; `ufw` is a term), and its shape cannot tell.
UNKNOWN_HEAD_MIN_CHARACTERS: Final = 4
# Russian letters whose keys type signs in the English layout (comma, period, semicolon, quote, brackets, backtick): a
# token holding one has a Latin reading with a sign in it, no term, and the engine never types such a replacement
# (context_policy._spelling_a_word).
UNKNOWN_HEAD_SIGN_KEY_LETTERS: Final = frozenset("бюжэхъё")
# A token one of whose readings is words joined by hyphens (`ку-ку`, `pre-order`, `что-то`) has the shared features once
# more under this prefix, with evidence of its own: the joined-token head (context_action_features.joined_question). No
# term table counts such a token, so it had no frequency evidence at all, and the frozen model kept a Russian word of
# two short ones typed in the English layout, each an English word as typed (`Re-re!` after a Russian sentence, meant
# as `Ку-ку!`, stayed in sealed test v58 with both pairs). The head reads how often the token's words occur.
JOINED_FEATURE_PREFIX: Final = "joined|"
# A token typed in the Russian layout whose Latin reading, a trailing sign aside, is a term Russian technical text uses
# at least in this bucket of the term table (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS: fifty times), while its Cyrillic
# reading occurs in no Russian text the tables count, has evidence of its own under this prefix: the term head
# (context_action_features.term_question). The frozen model left such a term typed in the
# Russian layout the first time in a message at p 0.92-0.98 (`сымю` for `csv.`, `сш` for `ci`, `рвьш` for `hdmi`; the
# owner's typing and ru.stackoverflow comments of 2024-2026), below the conversion threshold.
TERM_FEATURE_PREFIX: Final = "term|"
TERM_HEAD_MIN_LATIN_BUCKET: Final = 2
TERM_HEAD_MIN_LETTERS: Final = 2
# The signs a term may end in, typed on a Russian letter's key (`ю` is `.`, `б` is `,`): a term ends a sentence or an
# item of a list. It may also open with the dot of a file extension (`юучу` is `.exe`). A sign anywhere else is no term
# typed in the wrong layout (`бещ` is `,to`, `эты` is `'ns`).
TERM_HEAD_TRAILING_SIGNS: Final = ".,"
TERM_HEAD_LEADING_SIGN: Final = "."
# A token of this many letters may be the tail of a Russian word left by an edit inside it (`ев`, `ась`); the term head
# reads how many words of Russian prose end with it, as the binary order of magnitude of their number, capped here. The
# keys of a term end hardly any word (`зк` 1, `гш` 0, `вум` 1); the tails the owner's typing left end hundreds (`ев` 8,
# `ась` 10). The first-term head reads, the same way, how many words begin with such a token (`уче` begins 77, bucket 7).
TERM_HEAD_ENDING_LETTERS: Final = (2, 3)
TERM_HEAD_ENDING_BUCKET_MAX: Final = 12
# A token of this many Latin letters alone in the field whose other reading is Russian letters has evidence of its own
# under this prefix: the short-message head (context_action_features.brief_question).
# A whole message typed in the English layout stayed (`ytn` for `нет` at p 0.94 kept; `jr` for `ок` and `tot` for `еще`
# waiting for a next word that never came). The head reads the binary orders of magnitude of both readings' counts and
# of their ratio, up to BRIEF_RATIO_BOUND either way.
BRIEF_FEATURE_PREFIX: Final = "brief|"
BRIEF_HEAD_LETTERS: Final = (2, 3)
BRIEF_RATIO_BOUND: Final = 8
# The head answers only a token whose counts settle it as a Russian word typed in the English layout, as the
# calibration's lone words are labelled (context_deferral.counted_reading): the Cyrillic reading counted at least
# BRIEF_SETTLED_MIN_COUNT times and BRIEF_SETTLED_DOMINANCE times as often as the Latin one in English prose or Russian
# technical text, which stays under BRIEF_SETTLED_OTHER_MAXIMUM and no lexicon of the profile knows. Answering every
# token of the class, the head turned pairs both readings count (`ij`, `Jq`) into Russian and kept `ds` (`вы`) at
# some boundaries, where the calibration takes the frozen model's answer (corpus v63, candidate v63e).
BRIEF_SETTLED_MIN_COUNT: Final = 30
BRIEF_SETTLED_OTHER_MAXIMUM: Final = 30
BRIEF_SETTLED_DOMINANCE: Final = 5
# A term of up to three letters alone in a field a field reader read empty has evidence of its own under this prefix: the
# first-term head (context_action_features.first_question). There it opens a message and converts at once (`йк ` for
# `qr`, the owner's requirement of 16.10.2026), unless it may still be the tail of a Russian word.
FIRST_FEATURE_PREFIX: Final = "first|"
# A letter of the Russian layout typed alone after a word of Cyrillic letters, that is no Russian word of one letter, has
# evidence of its own under this prefix: the variable head (context_action_features.variable_question). In technical
# Russian prose it is a Latin variable typed in the Russian layout (`т` for `n`, `ч` for `x`); in chat it is a letter
# meant as written (`т.` of `и т. д.`, a word broken off).
VARIABLE_FEATURE_PREFIX: Final = "variable|"
# The boundaries the variable head answers at: a space or a sign ends the letter. At a pause the letter may be the first
# of a word still being typed (`Ф` of `AmoCRM_v2`, the owner's typing of 14.10.2026), a question the head has no frame
# of; there the frozen model and the lone-letter refusal decide, as they did before the head.
VARIABLE_HEAD_TRIGGERS: Final = ("space", "punctuation")
# Characters of the text before a one-letter variable the variable head reads for signs of code, and the signs.
VARIABLE_CODE_CONTEXT_CHARACTERS: Final = 40
VARIABLE_CODE_SIGNS: Final = "=<>+*/()[]{}_^%"
# A token typed in the English layout whose Cyrillic reading is two or more short groups of letters joined by dots has
# evidence of its own under this prefix: the dotted-abbreviation head (context_action_features.dotted_question); `n/r/`
# is `т.к.`.
DOTTED_FEATURE_PREFIX: Final = "dotted|"
# Letters a part of a dotted abbreviation holds at most (`т.к.`, `д.б.`, `т.е.`): a longer part is a word, and the token
# one of a path (`src/main`).
DOTTED_PART_MAX_LETTERS: Final = 3
# A token of letters and digits, both readings of it, has evidence of its own under this prefix: the letters-and-digits
# head (context_action_features.alnum_question). In Russian technical text it is a Latin token typed in the Russian
# layout (`м67` for `v67`, `зь2` for `pm2`) or one meant as typed (`v2`, `x86`, `1с`, `3д`).
ALNUM_FEATURE_PREFIX: Final = "alnum|"
# Characters such a token holds at most: a longer one is a word with a number (`python3`, `sqlite3`), which the frozen
# model reads by its letters.
ALNUM_HEAD_MAX_CHARACTERS: Final = 8
# The head reads the token's shape as its runs of letters and digits, each run's length up to ALNUM_RUN_LENGTH_MAX (`м67`
# is one letter and two digits); a token of more than ALNUM_SHAPE_MAX_RUNS runs has one shape for all.
ALNUM_RUN_LENGTH_MAX: Final = 3
ALNUM_SHAPE_MAX_RUNS: Final = 3
# The binary order of magnitude by which how often the letters of one reading occur outnumbers how often those of the
# other do, read up to this many either way.
ALNUM_RATIO_BOUND: Final = 8
# The question about the next word of a waiting word asked with that word's other reading before it (engine
# _decide_after_waiting_word, ContextEvidence.before_origin) has evidence of its own under this prefix: the hypothesis
# head (context_action_features.hypothesis_question). `тыв` after `tot ` (`еще тыв`, a typo of `ты`) read as the keys of
# `nsd`, a command name, and the pair became `tot nsd`.
HYPOTHESIS_FEATURE_PREFIX: Final = "hypo|"
# How often the commonest word one edit from a reading occurs is read as its binary order of magnitude, up to this; their
# difference between the readings up to HYPOTHESIS_RATIO_BOUND either way.
HYPOTHESIS_MAGNITUDE_MAX: Final = 18
# The messages its frames are captured from open with a word of at most this many letters, a word the engine may wait on
# (tools/select_capture_messages.py opening-short).
OPENING_SHORT_MAX_LETTERS: Final = 3
HYPOTHESIS_RATIO_BOUND: Final = 8
# A token of two or three Russian letters after Russian text, whose Latin reading is letters too and which no other head
# answers, has evidence of its own under this prefix: the short-token head (context_action_features.short_question).
# `кз` after `говорит он ` became `rp`, a word the English lexicon knows and no Russian table counted.
SHORT_FEATURE_PREFIX: Final = "short|"
SHORT_HEAD_LETTERS: Final = (2, 3)
# Its class is the token Russian text uses at least in this bucket of the counts (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS:
# fifty times, in web and news text, prose or technical text) while its Latin reading stays under the same bucket in
# English prose and Russian technical text: fitted on every such token, the head kept `ьк` (`mr`), `ин` (`by`) and `сс`
# (`cc`), which the frozen model converted as the owner meant (candidate v74 e2, the field gate).
SHORT_HEAD_MIN_RUSSIAN_BUCKET: Final = 2
SHORT_HEAD_MAX_LATIN_BUCKET: Final = 1
# The binary order of magnitude of a count is read up to this, and the difference of two up to SHORT_RATIO_BOUND.
SHORT_MAGNITUDE_MAX: Final = 16
SHORT_RATIO_BOUND: Final = 8
# A token of Latin letters after a line of Russian text, whose Latin reading English prose and Russian technical text use
# at most in this bucket of the counts (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS) and whose Cyrillic reading Russian prose
# uses, has evidence of its own under this prefix: the slip head (context_action_features.slip_question). A word of
# Russian prose typed with the English layout on stayed a suggestion near the threshold: `cgtie` for `спешу` after `я `
# at p=0.9885, `htity` for `решен`, and `pfdsk` for `завыл` was kept (Tatoeba development sentences, 23.10.2026).
SLIP_FEATURE_PREFIX: Final = "slip|"
# Shorter tokens are left out: the Latin tokens of two or three letters typed as meant after Russian text are
# abbreviations whose keys happen to spell Russian prose (`dbg`, `ck`, `pf`, the captured typing of ru.stackoverflow), and
# among the class's captured questions they were the only ones kept.
SLIP_HEAD_MIN_LETTERS: Final = 4
SLIP_HEAD_MAX_LATIN_BUCKET: Final = 1
# The detector's probability is read in this many bands, the orthotactic margin in bands this wide, up to
# SLIP_ORTHO_BANDS either way, and how often Russian prose uses the word before the token in steps of this many binary
# orders of magnitude.
SLIP_PROBABILITY_BANDS: Final = 10
SLIP_ORTHO_BAND_WIDTH: Final = 5.0
SLIP_ORTHO_BANDS: Final = 4
SLIP_BEFORE_MAGNITUDE_STEP: Final = 4
# The "length" feature buckets every word length from here up together (schemas 2 and 3).
ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS: Final = 6
# Short categorical field labels (role, trigger) truncated before they reach a feature name.
ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS: Final = 32
# Right-context ("after") text considered for scripting/lexical features (schemas 2 and 3).
ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS: Final = 128
# Left-context ("before") text considered for scripting/lexical features (schemas 2 and 3).
ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS: Final = 512
# Source/target token text truncated before character and script features (schemas 2 and 3).
ACTION_FEATURE_WORD_MAX_CHARACTERS: Final = 64
# Neighbouring words taken from the left or right context.
ACTION_FEATURE_NEIGHBOUR_WORD_COUNT: Final = 2
# Characters of a neighbouring word used for its character features.
ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS: Final = 24
# Whitespace run between the word and its neighbour, truncated and used to normalise the
# whitespace-shape features.
ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS: Final = 16
# The literal tail and the boundary character(s) are both short trailing punctuation buffers
# normalised the same way.
ACTION_FEATURE_SHORT_TEXT_CHARACTERS: Final = 8
# Application identifier considered for the "app:" features (schemas 2 and 3).
ACTION_FEATURE_APPLICATION_NAME_CHARACTERS: Final = 128
# Alphanumeric tokens of the application identifier turned into features (schemas 2 and 3).
ACTION_FEATURE_APPLICATION_TOKEN_COUNT: Final = 4
# The orthotactic model's score and threshold share this magnitude range.
ACTION_FEATURE_ORTHO_SCORE_BOUND: Final = 128.0
# Letters of a token in capitals that the capitals evidence describes: an abbreviation is three or
# more (`WBC`, `ГАК`); two-letter tokens stay with the deferral rules and the owner's `ГА`.
ACTION_FEATURE_CAPITALS_MIN_LETTERS: Final = 3
# ContextModel feature_version of the context-action (orthotactic-aware) scheme, feature schema 3.
CONTEXT_ACTION_FEATURE_VERSION: Final[int] = 3
# Shortest prefix the engine offers the prefix model; prefix training mirrors this support.
PREFIX_MIN_CHARACTERS: Final = 4
# Longest prefix the engine offers the prefix model, the characters prefix features (schema two)
# encode, and the longest prefix a prefix corpus may generate.
PREFIX_MAX_CHARACTERS: Final = 12
IDENTIFIER_LEXICON_CACHE_SIZE: Final = 4
LEXICON_SUPPLEMENT_CACHE_SIZE: Final = 8
# Newest prefix feature schema; schema one stays frozen in prefix_model.py.
CURRENT_PREFIX_FEATURE_VERSION: Final = 2
# Longest character n-gram of an observed prefix (schema two).
MAX_PREFIX_CHARACTER_NGRAM_ORDER: Final = 4
PREFIX_CHARACTER_FEATURE_WEIGHT_CAP: Final = 2.0
# Lowest conversion threshold a prefix model artifact may carry.
MIN_PREFIX_CONVERSION_THRESHOLD: Final = 0.985
MAX_PREFIX_WEIGHTS: Final = 10000
MAX_PREFIX_FEATURE_NAME_CHARACTERS: Final = 160
# A built-in fallback or extra word gets the lexicon's top frequency divided by the divisor, but at
# least the floor (the floor alone for a lexicon without frequency data).
SYNTHETIC_FREQUENCY_FLOOR: Final = 1000
SYNTHETIC_FREQUENCY_DIVISOR: Final = 20
# Conversion threshold of the context-v1 model: ContextModel's default, and the threshold the
# context-v1 trainer selects its epoch with and writes into the artifact.
CONTEXT_V1_CONVERSION_THRESHOLD: Final = 0.985
# An action model's keep with the other layout at least this likely is a suggestion instead (ContextModel.predict):
# the model is unsure, and at a space the engine asks once more beside the next word, as after a conversion below
# the threshold. `зк` after `принимай ` was kept at p=0.51 against 0.49 for `pr` (the owner's typing, 08.10.2026);
# asked beside its next word the same model converts it at p=1.00.
CONTEXT_DOUBTFUL_KEEP_PROBABILITY: Final = 0.3
# ContextModel feature_version of the context-v1/v2 scheme, feature schema 2; context_model.py
# exports it as FEATURE_VERSION.
CONTEXT_MODEL_FEATURE_VERSION: Final = 2
# ContextModel feature_version of the context-v1 scheme with typo evidence, feature schema 5: schema 2
# plus whether each reading is one typo away from a word the lexicon knows, and in which layout a
# token with a digit was typed. Schema 4 was an experiment that never shipped.
CONTEXT_TYPO_FEATURE_VERSION: Final = 5
# Schema 5: the shortest reading checked for being one typo away from a word; nearly every shorter
# string has a neighbour in a lexicon of this size.
CONTEXT_TYPO_MIN_CHARACTERS: Final = 5
# ContextModel feature_version of the context-v1 scheme with opening evidence, feature schema 6:
# schema 5 plus whether each reading is a word that opens sentences of its language
# (short_words.OPENING_WORDS). The frequency lexicons are encyclopedic and rank the conversational
# `ты` next to the English token `ns`, so without it a model could tell them apart only by letters.
CONTEXT_OPENING_FEATURE_VERSION: Final = 6
# ContextModel feature_version of the context-v1 scheme with line, case and term evidence, feature
# schema 7: schema 6 plus where on its line the word stands (the first word of a message or of a new
# line against a word with words before it), the case pattern of the token (an inner capital, all
# capitals), a dot or underscore inside its Latin reading, how often each reading occurs inside
# Russian text (a Latin term like `id`, Cyrillic slang like `пдф`) and in text of its own language
# (`uh` is an English word, `гр` a rare Russian one), and whether the token has letters at all - the
# questions the engine asks while real mixed Russian and English text is typed made these the
# evidence that decides.
CONTEXT_LINE_FEATURE_VERSION: Final = 7
# Schemas of the context-v1 trainer: feature 2, 5, 6 and 7 artifacts share its report, provenance and
# replay.
CONTEXT_V1_FEATURE_VERSIONS: Final = (
    CONTEXT_MODEL_FEATURE_VERSION, CONTEXT_TYPO_FEATURE_VERSION, CONTEXT_OPENING_FEATURE_VERSION,
    CONTEXT_LINE_FEATURE_VERSION,
)
# Schemas that carry every schema 6 feature (schema 7 adds its own on top).
CONTEXT_OPENING_SCHEMAS: Final = (CONTEXT_OPENING_FEATURE_VERSION, CONTEXT_LINE_FEATURE_VERSION)
# Schema 7: the occurrence counts that open the term frequency buckets above the first, in every
# table; a reading seen fewer times than the first bound is in bucket 0, as one never seen.
CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS: Final = (5, 50, 500, 5000)
# The lowest term frequency bucket of a Russian abbreviation or slang word the context policy keeps
# as typed (context_policy.russian_slang): fifty occurrences or more inside Russian technical text.
RUSSIAN_SLANG_MIN_TERM_BUCKET: Final = 2
# Feature schemas a context artifact may carry: context-v1's and the context action scheme's.
CONTEXT_SUPPORTED_FEATURE_VERSIONS: Final = (*CONTEXT_V1_FEATURE_VERSIONS, CONTEXT_ACTION_FEATURE_VERSION)
# Feature schema 2 only: words of the left and of the right context that become word features, and
# the character n-gram orders of the two readings.
CONTEXT_FEATURE_BEFORE_WORD_COUNT: Final = 6
CONTEXT_FEATURE_AFTER_WORD_COUNT: Final = 3
CONTEXT_FEATURE_NGRAM_ORDERS: Final = (1, 2, 3)
# What the context model loader accepts (schemas 2 and 3): at most this many features, feature names
# and version strings of at most these many characters, weights of at most this magnitude, and a
# conversion threshold from this floor up to one.
MAX_CONTEXT_MODEL_FEATURES: Final = 50000
# An action model (schema 3) holds up to sixteen feature spaces: the word decided at its boundary, the
# kept-neighbour question (KEPT_FEATURE_PREFIX), the capitals head (CAPITALS_FEATURE_PREFIX), the
# lone-word head (ALONE_FEATURE_PREFIX), the message-start head (START_FEATURE_PREFIX), the
# single-letter head (LETTER_FEATURE_PREFIX), the abbreviation heads (ABBREVIATION_FEATURE_PREFIX,
# LATIN_ABBREVIATION_FEATURE_PREFIX), the unknown-token head (UNKNOWN_FEATURE_PREFIX), the joined-token head
# (JOINED_FEATURE_PREFIX), the term heads (TERM_FEATURE_PREFIX, FIRST_FEATURE_PREFIX), the short-message head
# (BRIEF_FEATURE_PREFIX), the variable head (VARIABLE_FEATURE_PREFIX), the dotted-abbreviation head
# (DOTTED_FEATURE_PREFIX), the letters-and-digits head (ALNUM_FEATURE_PREFIX), the hypothesis head
# (HYPOTHESIS_FEATURE_PREFIX) and the short-token head (SHORT_FEATURE_PREFIX), each with its own budget in the recipe.
# Raised from 100 000 with the unknown-token head (the 0.54.0 pair holds 98 498), and from 110 000 with the hypothesis
# and short-token heads: the 0.61.0 pair holds 107 808, about 8.6 MB written one feature to a line, and
# MAX_CONTEXT_ACTION_MODEL_BYTES bounds the file.
MAX_CONTEXT_ACTION_MODEL_FEATURES: Final = 150000
MAX_CONTEXT_FEATURE_NAME_CHARACTERS: Final = 512
MAX_CONTEXT_MODEL_VERSION_CHARACTERS: Final = 80
MAX_CONTEXT_WEIGHT_MAGNITUDE: Final = 1000
MIN_CONTEXT_CONVERSION_THRESHOLD: Final = 0.95
# Feature schema of the intent model (the KSLM manifest's feature_version).
INTENT_FEATURE_VERSION: Final[int] = 5
# The last intent feature schema verified to ignore the context grid; the trainer's fast context
# stress relies on it and refuses any other schema.
INTENT_CONTEXT_INVARIANT_FEATURE_VERSION: Final[int] = 5
# FNV-1a, 64-bit: the offset basis (also the intent model's default feature-hash seed) and the prime.
FNV1A64_OFFSET_BASIS: Final[int] = 0xCBF29CE484222325
FNV1A64_PRIME: Final[int] = 0x100000001B3
# Default seed of the intent model's exact-name membership hash; it must differ from the feature
# seed.
INTENT_MEMBERSHIP_FNV_SEED: Final[int] = 0x9E3779B97F4A7C15
# The top bit of a 64-bit feature hash picks the feature's collision sign.
FEATURE_HASH_SIGN_BIT: Final[int] = 1 << 63
# Character n-gram orders of the intent model's features; KSLM schema 4 requires exactly these.
INTENT_NGRAM_ORDERS: Final[tuple[int, ...]] = (1, 2, 3, 4, 5)
# A token is cut to this many characters before normalisation, bounding CPU and memory.
INTENT_RAW_TOKEN_MAX_CHARACTERS: Final[int] = 64
# The intent model decides only when the longer normalised reading has at least this many
# characters.
INTENT_MIN_RUNTIME_TOKEN_CHARACTERS: Final[int] = 5
# Largest magnitude of a number a KSLM manifest carries (weight scale, calibration, thresholds).
INTENT_MAX_MODEL_FLOAT_MAGNITUDE: Final[float] = 1_000_000.0
INTENT_MODEL_VERSION_MAX_CHARACTERS: Final[int] = 128
# Deepest nesting of KSLM manifest metadata.
INTENT_METADATA_MAX_DEPTH: Final[int] = 16
# Layout groups are clamped into -1 through this before they reach a feature name.
INTENT_MAX_FEATURE_GROUP: Final[int] = 63
# The intent model's word-length feature: every length up to the first bound is its own bucket, then
# one bucket ends at each further bound, and longer words share the last one. The trainer's
# diagnostic slices use the same bounds.
INTENT_LENGTH_EXACT_MAX_CHARACTERS: Final[int] = 4
INTENT_LENGTH_SHORT_MAX_CHARACTERS: Final[int] = 7
INTENT_LENGTH_MEDIUM_MAX_CHARACTERS: Final[int] = 11
INTENT_LENGTH_LONG_MAX_CHARACTERS: Final[int] = 19
# Character n-gram orders of the language model's smoothed naturalness score.
LANGUAGE_MODEL_NGRAM_ORDERS: Final[tuple[int, ...]] = (2, 3, 4)
# The n-gram order whose coverage gives a WordScore's gram_ratio and invalid_ratio.
LANGUAGE_MODEL_TRIGRAM_ORDER: Final[int] = 3
# Shortest word a language model reads from its lexicon or counts into its n-grams.
LANGUAGE_MODEL_MIN_WORD_CHARACTERS: Final[int] = 2
# Naturalness is normalised against the most frequent words: at most this many, each at least this
# long. Without any, this mean is assumed; a deviation below the floor counts as one.
LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT: Final[int] = 12_000
LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS: Final[int] = 3
LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN: Final[float] = -10.0
LANGUAGE_MODEL_MIN_NGRAM_DEVIATION: Final[float] = 0.05
# Loaded language models kept, one per locale and extra-word set.
LANGUAGE_MODEL_CACHE_SIZE: Final[int] = 16
# Word scores a language model caches; the intent trainer's train-only scorer caches as many.
LANGUAGE_MODEL_SCORE_CACHE_MAXSIZE: Final[int] = 65_536
# Cap on the log-scaled weight one word adds to its character n-gram counts.
LANGUAGE_MODEL_MAX_GRAM_WEIGHT: Final[int] = 32
# Additive smoothing of n-gram probabilities: the pseudo-count of a gram and the unseen grams added
# to the vocabulary.
LANGUAGE_MODEL_GRAM_SMOOTHING: Final[float] = 0.2
LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY: Final[int] = 2048
# Score of a token with no letters: its WordScore value and its raw n-gram score.
LANGUAGE_MODEL_EMPTY_TOKEN_SCORE: Final[float] = -30.0
# Naturalness (the normalised n-gram score) is clamped to this range. A known word's naturalness is
# raised to at least the floor, and the detector accepts a target at the floor as a rare valid form.
LANGUAGE_MODEL_NATURALNESS_MIN: Final[float] = -15.0
LANGUAGE_MODEL_NATURALNESS_MAX: Final[float] = 4.0
LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR: Final[float] = -4.0
# A WordScore's value: a lexical score, plus the naturalness weight times naturalness, minus the
# invalid-ratio weight times the invalid ratio. The lexical score is the base plus the popularity
# weight times popularity for a frequency-lexicon word, or the Hunspell-only score.
LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE: Final[float] = 7.0
LANGUAGE_MODEL_POPULARITY_WEIGHT: Final[float] = 3.0
LANGUAGE_MODEL_SPELL_KNOWN_SCORE: Final[float] = 6.5
LANGUAGE_MODEL_NATURALNESS_WEIGHT: Final[float] = 1.15
LANGUAGE_MODEL_INVALID_RATIO_WEIGHT: Final[float] = 0.75
# best_single_deletion() leaves words shorter than this alone and tries at most this many evenly
# spaced positions.
LANGUAGE_MODEL_DELETION_MIN_CHARACTERS: Final[int] = 4
LANGUAGE_MODEL_DELETION_POSITIONS: Final[int] = 12

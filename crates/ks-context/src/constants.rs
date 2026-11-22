//! Values the context layer uses, each once, as the Python keeps them in its `constants` folder.

// ---- constants/boundary.py

// Letters a misspelled reading may be missing, by locale of the word model that reads it.
pub const BOUNDARY_MISSING_LETTERS: &[(&str, &str)] = &[("en_US", "abcdefghijklmnopqrstuvwxyz"), ("ru_RU", "абвгдеёжзийклмнопрстуфхцчшщъыьэюя")];

// ---- constants/detection.py

// The shortest token word_shaped() can judge: with one character its first- and last-letter checks
// look at the same position.
pub const MINIMUM_SHAPED_TOKEN_CHARACTERS: usize = 2;

// The quotes a Latin token typed in the Russian layout may open with (the `Э` and `э` keys), and the signs one may stand
// after its closing quote (`"legacy",`), for the word-shape refusal (context_policy.quoted_or_scoped).
pub const WORD_QUOTES: &str = "\"'";

pub const QUOTED_WORD_CLOSING_SIGNS: &str = ".,:;!?";

// ---- constants/file_formats.py

// Hex characters of a SHA-256 kept in a readable model or resource version string
// (context-v1-<hash>, context-v3-<hash>, prefix-v2-<hash>, intent-v1-<hash>, identifiers-<hash>)
// and in the intent model's status summary (sha256:<hash>).
pub const VERSION_HASH_CHARACTERS: usize = 12;

// Largest prefix model artifact loaded.
pub const MAX_PREFIX_MODEL_BYTES: usize = 2_097_152;

// Largest context model artifact loaded (context_model.py exports it as MAX_ARTIFACT_BYTES); the feature-2 history
// (tools/verify_context_v2_history.py) holds this value.
pub const MAX_CONTEXT_MODEL_BYTES: usize = 8_388_608;

// Largest action model (schema 3) artifact loaded: written one feature to a line (train_context_action_model.artifact_bytes),
// the 0.61.0 pair takes about 8.6 MB.
pub const MAX_CONTEXT_ACTION_MODEL_BYTES: usize = 16_777_216;

// Largest term frequency table the context model reads with feature schema 7.
pub const MAX_CONTEXT_TERM_FREQUENCY_BYTES: usize = 8_388_608;

// Largest table of tokens of letters and digits the letters-and-digits head reads.
pub const MAX_CONTEXT_ALNUM_FREQUENCY_BYTES: usize = 2_097_152;

// Largest table of short Russian tokens counted in web and news text that the short-token head reads.
pub const MAX_CONTEXT_SHORT_FREQUENCY_BYTES: usize = 2_097_152;

// Largest table of tokens spelled in capitals that the upper-case head reads.
pub const MAX_CONTEXT_CAPITALS_FREQUENCY_BYTES: usize = 4_194_304;

// ---- constants/models.py

// The context model's features. Feature schema 3 (context_action_features) and the frozen feature
// schema 2 (context_model.extract_context_features, read by the installed context-v1 model and
// compared with its archived source by tools/verify_context_v2_history.py) share the values marked
// "schemas 2 and 3": changing one of them changes the shipped model's features too.
// Character n-gram orders extracted around each text span.
pub const ACTION_FEATURE_NGRAM_ORDERS: &[usize] = &[1, 2, 3, 4];

// Cap on one accumulated character n-gram feature's weight.
pub const ACTION_FEATURE_CHARACTER_WEIGHT_CAP: f64 = 8.0;

// A character feature name is `label:char:direction:order:text`: its n-gram text follows this many
// separators, so a split at most this many times leaves it whole at this index.
pub const ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX: usize = 4;

// A WordScore's value, ngram_score and the value delta between two readings share the same
// log-probability-like scale; clamped to this range and divided by it so the feature stays in [-1,
// 1]. Schemas 2 and 3 (schema 2 bounds only the score delta).
pub const ACTION_FEATURE_WORD_SCORE_BOUND: f64 = 10.0;

// Magnitude a raw n-gram score is clamped to and divided by.
pub const ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND: f64 = 32.0;

// Frequencies are log-scaled against this cap so one very common word cannot dominate the feature.
pub const ACTION_FEATURE_FREQUENCY_CAP: i64 = 1_000_000_000_000;

// `after_origin == "planned_next_conversion"` is only reachable for a short waiting word with a
// short, single-line planned right context; it follows ACTION_DEFERRED_WORD_MAX_CHARACTERS, the
// longest word the action model defers and the engine re-decides with its planned next word. With
// two, a three-letter `tot` the model asked to wait for `привет` could never be asked again.
pub const PLANNED_CONTEXT_WORD_MAX_CHARACTERS: usize = 3;

// Longest single-line planned right context a planned_next_conversion origin allows.
pub const PLANNED_CONTEXT_AFTER_MAX_CHARACTERS: usize = 64;

// `after_origin == "kept_next_word"`: a waiting word of at most this many characters asked once more
// with its next word, which stayed as typed in the same layout (KeySwitchEngine.decide_with_kept_neighbour).
// Five covers the terms the owner's typing left behind that way (`шзм6` for `ipv6`, `тзь` for `npm`).
pub const KEPT_CONTEXT_WORD_MAX_CHARACTERS: usize = 5;

// Every feature of a kept-neighbour question carries this prefix: the question has weights of its own,
// trained on frames of its own, and never moves the weights of the word decided at its boundary.
pub const KEPT_FEATURE_PREFIX: &str = "kept|";

// A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS, after
// text with more Latin letters than Cyrillic ones, has its features once more under this prefix: the
// capitals head, fitted onto a frozen model with its every other weight fixed, answers that class
// alone; a model without such weights decides as it did. The class is the shape of a cited
// abbreviation (`BP`, `NASA`) in Latin-script text; a longer word in capitals is a word typed with
// Caps Lock (`PFGHJC` for `ЗАПРОС`), which the first head turned into a keep.
pub const CAPITALS_FEATURE_PREFIX: &str = "caps|";

pub const CAPITALS_HEAD_MIN_LETTERS: usize = 2;

pub const CAPITALS_HEAD_MAX_LETTERS: usize = 5;

// A token of ALONE_HEAD_LETTERS letters with no letter on either side in the field, sent or left at a
// pause - a one-word message - has its features once more under this prefix: the lone-word head,
// fitted onto a frozen model, answers that class alone. Two letters are where the
// owner's typing left most lone words in the wrong layout (`гш` for `ui`, `зщ` for `po`), because
// every such frame of the corpus was deferred (context_deferral).
pub const ALONE_FEATURE_PREFIX: &str = "alone|";

pub const ALONE_HEAD_LETTERS: usize = 2;

// The lone-word head reads the ratio of a word's two counts as the difference of their binary orders of
// magnitude, clamped to this bound either way.
pub const ALONE_COUNT_RATIO_BOUND: i64 = 8;

// A word of letters of at least START_HEAD_MIN_LETTERS, letters in both layouts, with no letter on either
// side in the field - the first word of a message, decided before anything follows it - has its features
// once more under this prefix: the message-start head, fitted onto a frozen model, answers that class
// alone. The shared features read how often each reading occurs in Russian and in English prose only for
// a word with text around it, so the first word was decided by its letters and the term tables, in which
// neither reading of a rare Russian word occurs: a correctly typed gerund opening a message turned into
// its Latin keys, whose ending reads like an English past tense. The head reads both prose counts too
// (the same ratio bound as the lone-word head). Two letters belong to the lone-word head; three letters
// alone are deferred where both readings are plausible (context_deferral), and a head fitted on them
// turned the owner's `чук`, `дес` and `дые` into Latin, so the class starts at four.
pub const START_FEATURE_PREFIX: &str = "start|";

pub const START_HEAD_MIN_LETTERS: usize = 4;

// The message-start head also reads the detector's score delta in bands of this width, up to this many
// bands either way: the shared feature is clamped at ACTION_FEATURE_WORD_SCORE_BOUND, where nearly every
// correctly typed first word and every rare Russian word the model converted at once both lie, and a
// band of its own lets the head answer that end and the other one apart.
pub const START_DELTA_BAND_WIDTH: f64 = 2.5;

pub const START_DELTA_BANDS: i64 = 6;

// A word of the message-start head's class (START_HEAD_MIN_LETTERS letters or more, letters in both layouts, no letter on
// either side) that no lexicon of the profile and no identifier index knows in either reading is asked of the feel head
// as well. The frozen model decides such a word by the orthotactic total and its margin, which add up its keys: a long
// word and a short one of the same per-key fit read differently, and a made-up Russian word typed in the English layout
// at text start stayed Latin in 34 of 465 cases, 19 of them with the gap of its readings a key above 1.5 (the read half of
// the made-up words of cycle v89).
// The head reads, a key at a time (ContextEvidence::ortho_typed_mean, ortho_other_mean), the gap between the other
// reading and the reading as typed in bands of FEEL_GAP_BAND_WIDTH up to FEEL_GAP_BANDS either way, and the typed
// reading alone in bands of FEEL_TYPED_BAND_WIDTH up to FEEL_TYPED_BANDS; no letters and no copy of the shared features.
// On the class's words of the training documents typed both ways, a gap above 1.5 was a conversion in all 825 cases and
// one below -1.5 a word typed as meant in all 840; 6 lay between (of the 1 959 made-up words, 132). The head reads only a
// gap of FEEL_DECISIVE_GAP or more either way: fitted with a bias and the typed fit alone, read in the middle too, it
// kept 15 made-up English words typed in the Russian layout (`frupz`, gap 0.9) and `afrit` (gap 1.8) that the frozen model
// converted (development round 1, cycle v89).
pub const FEEL_FEATURE_PREFIX: &str = "feel|";

pub const FEEL_DECISIVE_GAP: f64 = 1.5;

pub const FEEL_GAP_BAND_WIDTH: f64 = 1.5;

pub const FEEL_GAP_BANDS: i64 = 4;

pub const FEEL_TYPED_BAND_WIDTH: f64 = 2.0;

pub const FEEL_TYPED_BANDS: i64 = 6;

// A word of one letter, a letter in both layouts, right after a word whose letters are all Latin, has a
// head of its own (context_action_features.letter_question): the class's features once more under this
// prefix, and how often the word before it and the word after it occur in English prose and in Russian
// technical text. `nats b redis` is `nats и redis` typed in the wrong layout, `plan b then` stays.
pub const LETTER_FEATURE_PREFIX: &str = "letter|";

// A Cyrillic abbreviation in capitals that Russian technical text uses and the lexicon does not hold, whose Latin
// reading is letters too, after text with more Cyrillic letters than Latin ones has evidence of its own under this
// prefix: the abbreviation head (context_action_features.abbreviation_question), fitted onto a frozen model, answers
// that class alone. After Russian prose the frozen model turned such abbreviations into the Latin readings they spell
// (`страны ЕС приняли` became `страны TC приняли`, `поздравляю с НГ` became `поздравляю с YU`, 0.37-0.42): the corpus
// framed every Latin abbreviation typed in the Russian layout after Russian prose with the convert label, whatever
// its keys spelled. The capitals head answers the same shape after Latin text.
pub const ABBREVIATION_FEATURE_PREFIX: &str = "abbr|";

// Letters no Russian word starts with, so no abbreviation of Russian words holds them: their keys are the Latin
// `M`, `S`, `]` and `Q` (`ДДЬ` is `LLM`, `ЫЫР` is `SSH`, `ЙФ` is `QA`).
pub const ABBREVIATION_NON_INITIAL_LETTERS: &[char] = &['Й', 'Ъ', 'Ы', 'Ь'];

// The same attested abbreviation after text with more Latin letters than Cyrillic ones has evidence of its own under
// this prefix: the Latin-context abbreviation head (context_action_features.latin_abbreviation_question). There the
// capitals head and the shared features answered alone and turned the abbreviation into the Latin reading its keys
// spell whatever its counts (`/designv2 ТЗ` became `/designv2 NP`, the owner's typing of 08.10.2026; `ЛС`, `ТГ`
// and `НГ` the same way).
pub const LATIN_ABBREVIATION_FEATURE_PREFIX: &str = "labbr|";

// A token typed in the Russian layout whose Cyrillic reading the lexicon does not hold, of letters and digits, at
// least UNKNOWN_HEAD_MIN_CHARACTERS long and not all capitals, after text with more Cyrillic letters than Latin ones,
// has the shared features once more under this prefix, with evidence of its own: the unknown-token head
// (context_action_features.unknown_question). It is a technical term typed in the wrong layout (`цуиоы` for `webjs`,
// `мгуоы` for `vuejs`, `реьч` for `htmx`) or a Russian word the lexicon lacks (`ватсап`, a typo, a name). The frozen
// model kept the terms there: a token no table counts after Russian prose read to it as Russian slang (the owner's
// typing in a chat input of VS Code, 13.10.2026; 15 of 16 short terms stayed Cyrillic after `затрагивает только`).
pub const UNKNOWN_FEATURE_PREFIX: &str = "unknown|";

// Four and more: a Russian token of three letters no table counts is slang or a typo as often as a term typed in the
// wrong layout (`зач`, `мты`, `нзч` stayed Russian in the owner's typing; `ufw` is a term), and its shape cannot tell.
pub const UNKNOWN_HEAD_MIN_CHARACTERS: usize = 4;

// Russian letters whose keys type signs in the English layout (comma, period, semicolon, quote, brackets, backtick): a
// token holding one has a Latin reading with a sign in it, no term, and the engine never types such a replacement
// (context_policy._spelling_a_word).
pub const UNKNOWN_HEAD_SIGN_KEY_LETTERS: &[char] = &['б', 'ж', 'х', 'ъ', 'э', 'ю', 'ё'];

// A token one of whose readings is words joined by hyphens (`ку-ку`, `pre-order`, `что-то`) has the shared features once
// more under this prefix, with evidence of its own: the joined-token head (context_action_features.joined_question). No
// term table counts such a token, so it had no frequency evidence at all, and the frozen model kept a Russian word of
// two short ones typed in the English layout, each an English word as typed (`Re-re!` after a Russian sentence, meant
// as `Ку-ку!`, stayed in sealed test v58 with both pairs). The head reads how often the token's words occur.
pub const JOINED_FEATURE_PREFIX: &str = "joined|";

// A token typed in the Russian layout whose Latin reading, a trailing sign aside, is a term Russian technical text uses
// at least in this bucket of the term table (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS: fifty times), while its Cyrillic
// reading occurs in no Russian text the tables count, has evidence of its own under this prefix: the term head
// (context_action_features.term_question). The frozen model left such a term typed in the
// Russian layout the first time in a message at p 0.92-0.98 (`сымю` for `csv.`, `сш` for `ci`, `рвьш` for `hdmi`; the
// owner's typing and ru.stackoverflow comments of 2024-2026), below the conversion threshold.
pub const TERM_FEATURE_PREFIX: &str = "term|";

pub const TERM_HEAD_MIN_LATIN_BUCKET: usize = 2;

pub const TERM_HEAD_MIN_LETTERS: usize = 2;

// The signs a term may end in, typed on a Russian letter's key (`ю` is `.`, `б` is `,`): a term ends a sentence or an
// item of a list. It may also open with the dot of a file extension (`юучу` is `.exe`). A sign anywhere else is no term
// typed in the wrong layout (`бещ` is `,to`, `эты` is `'ns`).
pub const TERM_HEAD_TRAILING_SIGNS: &str = ".,";

pub const TERM_HEAD_LEADING_SIGN: &str = ".";

// A token of this many letters may be the tail of a Russian word left by an edit inside it (`ев`, `ась`); the term head
// reads how many words of Russian prose end with it, as the binary order of magnitude of their number, capped here. The
// keys of a term end hardly any word (`зк` 1, `гш` 0, `вум` 1); the tails the owner's typing left end hundreds (`ев` 8,
// `ась` 10). The first-term head reads, the same way, how many words begin with such a token (`уче` begins 77, bucket 7).
pub const TERM_HEAD_ENDING_LETTERS: &[usize] = &[2, 3];

pub const TERM_HEAD_ENDING_BUCKET_MAX: usize = 12;

// A token of this many Latin letters alone in the field whose other reading is Russian letters has evidence of its own
// under this prefix: the short-message head (context_action_features.brief_question).
// A whole message typed in the English layout stayed (`ytn` for `нет` at p 0.94 kept; `jr` for `ок` and `tot` for `еще`
// waiting for a next word that never came). The head reads the binary orders of magnitude of both readings' counts and
// of their ratio, up to BRIEF_RATIO_BOUND either way.
pub const BRIEF_FEATURE_PREFIX: &str = "brief|";

pub const BRIEF_HEAD_LETTERS: &[usize] = &[2, 3];

pub const BRIEF_RATIO_BOUND: i64 = 8;

// The head answers only a token whose counts settle it as a Russian word typed in the English layout, as the
// calibration's lone words are labelled (context_deferral.counted_reading): the Cyrillic reading counted at least
// BRIEF_SETTLED_MIN_COUNT times and BRIEF_SETTLED_DOMINANCE times as often as the Latin one in English prose or Russian
// technical text, which stays under BRIEF_SETTLED_OTHER_MAXIMUM and no lexicon of the profile knows. Answering every
// token of the class, the head turned pairs both readings count (`ij`, `Jq`) into Russian and kept `ds` (`вы`) at
// some boundaries, where the calibration takes the frozen model's answer (corpus v63, candidate v63e).
pub const BRIEF_SETTLED_MIN_COUNT: i64 = 30;

pub const BRIEF_SETTLED_OTHER_MAXIMUM: i64 = 30;

pub const BRIEF_SETTLED_DOMINANCE: i64 = 5;

// A token a lexicon of the profile knows is answered by the head too where its Cyrillic reading is counted at least
// BRIEF_KNOWN_MIN_COUNT times and BRIEF_KNOWN_DOMINANCE times as often as the token, which stays under
// BRIEF_SETTLED_OTHER_MAXIMUM (context_action_features.brief_dominant): `jr` sent alone for `ок` (7 180 against 13)
// stayed, as the English lexicon knows `Jr`, and the frozen model only suggested a look at the layout (the laptop's
// Telegram, 02.11.2026). Six known tokens pass in the portable profile (`jr`, `nj`, `tot`, `nf`, `gr`, `eds`) and two more
// Hunspell knows (`ult`, `dbl`); `ha` (273 against 28), `ns` (`ты`, 195 in technical text), `tim` and `ltd` stay with the
// frozen model.
pub const BRIEF_KNOWN_MIN_COUNT: i64 = 1000;

pub const BRIEF_KNOWN_DOMINANCE: i64 = 100;

// A token of the short-message head's class whose counts outnumber it that far (brief_dominant) has evidence of its own
// under this prefix: the known-reply head (context_action_features.known_reply_question). Let the short-message head
// answer `jr` alone in Telegram, the frozen model still left it at p 0.51 for a suggestion against 0.49 for `ок`: the
// lone-word head reads the application, and it had learned to suggest every known token sent alone.
pub const KNOWN_REPLY_FEATURE_PREFIX: &str = "knownreply|";

// A term of up to three letters alone in a field a field reader read empty has evidence of its own under this prefix: the
// first-term head (context_action_features.first_question). There it opens a message and converts at once (`йк ` for
// `qr`, the owner's requirement of 16.10.2026), unless it may still be the tail of a Russian word.
pub const FIRST_FEATURE_PREFIX: &str = "first|";

// A letter of the Russian layout typed alone after a word of Cyrillic letters, that is no Russian word of one letter, has
// evidence of its own under this prefix: the variable head (context_action_features.variable_question). In technical
// Russian prose it is a Latin variable typed in the Russian layout (`т` for `n`, `ч` for `x`); in chat it is a letter
// meant as written (`т.` of `и т. д.`, a word broken off).
pub const VARIABLE_FEATURE_PREFIX: &str = "variable|";

// The boundaries the variable head answers at: a space or a sign ends the letter. At a pause the letter may be the first
// of a word still being typed (`Ф` of `AmoCRM_v2`, the owner's typing of 14.10.2026), a question the head has no frame
// of; there the frozen model and the lone-letter refusal decide, as they did before the head.
pub const VARIABLE_HEAD_TRIGGERS: &[&str] = &["space", "punctuation"];

// Characters of the text before a one-letter variable the variable head reads for signs of code, and the signs.
pub const VARIABLE_CODE_CONTEXT_CHARACTERS: usize = 40;

pub const VARIABLE_CODE_SIGNS: &str = "=<>+*/()[]{}_^%";

// A token typed in the English layout whose Cyrillic reading is two or more short groups of letters joined by dots has
// evidence of its own under this prefix: the dotted-abbreviation head (context_action_features.dotted_question); `n/r/`
// is `т.к.`.
pub const DOTTED_FEATURE_PREFIX: &str = "dotted|";

// Letters a part of a dotted abbreviation holds at most (`т.к.`, `д.б.`, `т.е.`): a longer part is a word, and the token
// one of a path (`src/main`).
pub const DOTTED_PART_MAX_LETTERS: usize = 3;

// A token of letters and digits, both readings of it, has evidence of its own under this prefix: the letters-and-digits
// head (context_action_features.alnum_question). In Russian technical text it is a Latin token typed in the Russian
// layout (`м67` for `v67`, `зь2` for `pm2`) or one meant as typed (`v2`, `x86`, `1с`, `3д`).
pub const ALNUM_FEATURE_PREFIX: &str = "alnum|";

// Characters such a token holds at most: a longer one is a word with a number (`python3`, `sqlite3`), which the frozen
// model reads by its letters.
pub const ALNUM_HEAD_MAX_CHARACTERS: usize = 8;

// The head reads the token's shape as its runs of letters and digits, each run's length up to ALNUM_RUN_LENGTH_MAX (`м67`
// is one letter and two digits); a token of more than ALNUM_SHAPE_MAX_RUNS runs has one shape for all.
pub const ALNUM_RUN_LENGTH_MAX: usize = 3;

pub const ALNUM_SHAPE_MAX_RUNS: usize = 3;

// The binary order of magnitude by which how often the letters of one reading occur outnumbers how often those of the
// other do, read up to this many either way.
pub const ALNUM_RATIO_BOUND: i64 = 8;

// The question about the next word of a waiting word asked with that word's other reading before it (engine
// decide_after_waiting_word, ContextEvidence.before_origin) has evidence of its own under this prefix: the hypothesis
// head (context_action_features.hypothesis_question). `тыв` after `tot ` (`еще тыв`, a typo of `ты`) read as the keys of
// `nsd`, a command name, and the pair became `tot nsd`.
pub const HYPOTHESIS_FEATURE_PREFIX: &str = "hypo|";

// How often the commonest word one edit from a reading occurs is read as its binary order of magnitude, up to this; their
// difference between the readings up to HYPOTHESIS_RATIO_BOUND either way.
pub const HYPOTHESIS_MAGNITUDE_MAX: i64 = 18;

// The messages its frames are captured from open with a word of at most this many letters, a word the engine may wait on
// (tools/select_capture_messages.py opening-short).
pub const OPENING_SHORT_MAX_LETTERS: usize = 3;

pub const HYPOTHESIS_RATIO_BOUND: i64 = 8;

// A token of two or three Russian letters after Russian text, whose Latin reading is letters too and which no other head
// answers, has evidence of its own under this prefix: the short-token head (context_action_features.short_question).
// `кз` after `говорит он ` became `rp`, a word the English lexicon knows and no Russian table counted.
pub const SHORT_FEATURE_PREFIX: &str = "short|";

pub const SHORT_HEAD_LETTERS: &[usize] = &[2, 3];

// Its class is the token Russian text uses at least in this bucket of the counts (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS:
// fifty times, in web and news text, prose or technical text) while its Latin reading stays under the same bucket in
// English prose and Russian technical text: fitted on every such token, the head kept `ьк` (`mr`), `ин` (`by`) and `сс`
// (`cc`), which the frozen model converted as the owner meant (candidate v74 e2, the field gate).
pub const SHORT_HEAD_MIN_RUSSIAN_BUCKET: usize = 2;

pub const SHORT_HEAD_MAX_LATIN_BUCKET: usize = 1;

// The binary order of magnitude of a count is read up to this, and the difference of two up to SHORT_RATIO_BOUND.
pub const SHORT_MAGNITUDE_MAX: i64 = 16;

pub const SHORT_RATIO_BOUND: i64 = 8;

// A token of two or three Russian letters typed in the Russian layout after text with letters, that Russian text uses at
// least in bucket SHORT_HEAD_MIN_RUSSIAN_BUCKET while English prose or Russian technical text use its Latin reading above
// bucket SHORT_HEAD_MAX_LATIN_BUCKET, has evidence of its own under this prefix: the common-reading head
// (context_action_features.common_question), the rest of the short-token head's shape. The installed pair of 0.71.2
// turned `ру` after a Russian word into `he` at p=1.00 (14 times in two days of a user's typing): the questions it was fitted
// on hold 1 543 conversions of `ру` first on its line under a Russian line (English sentences typed in the Russian layout,
// `He …`) against 12 frames that keep it, and nothing told the two places apart. The head reads where the token stands
// on its line, how Russian the word before it reads, how often each reading comes right after a word of Cyrillic letters
// in Russian technical text (the after_cyrillic section of SHORT_FREQUENCY_FILE: `ру` 276 times, `he` 15) and the counts
// the short-token head reads, each by the binary order of magnitude (SHORT_MAGNITUDE_MAX) and their differences
// (SHORT_RATIO_BOUND); no letters and no copy of the shared features.
pub const COMMON_FEATURE_PREFIX: &str = "common|";

// The boundaries the common-reading head answers at: those its frames are asked at. The typing bench types no pause and
// no Enter, and at a pause the token may be the first letters of a word (`ес` of `если`), which the frozen model, fitted
// on the corpus's frames at a pause, waits on.
pub const COMMON_HEAD_TRIGGERS: &[&str] = &["space", "punctuation"];

// The head answers a token whose Cyrillic reading comes right after a word of Cyrillic letters in Russian technical text at
// least this many binary orders of magnitude more often than its Latin reading (four times as often; `ру` 276 times, `he`
// 15: five orders): a token the counts settle as a Russian one after a Russian word, the error the head is for. Where the
// Latin reading follows a Russian word as often or more often (`сс` 232 and `cc` 151, `ва` 896 and `df` 1 414), the token
// after a Russian word is as often a Latin term typed in the Russian layout, and the frozen model, which reads its letters,
// decides it. On the head's development questions (candidate context-v3-1a3e7c017630 of corpus v86, fitted on every token
// of the shape) the head gave 17 right answers more and 23 false conversions fewer among the 542 after a Cyrillic word
// whose orders differ by two or more; among the 126 that differ by one or not at all it gave two right answers fewer and
// three false conversions more; among the 449 where the Latin reading leads it gave 17 right answers more, learned from
// the frames of a few terms of technical text (`гд` for `ul` is 96 of the 425 to convert).
pub const COMMON_HEAD_MIN_FOLLOWING_LEAN: i64 = 2;

// A token of Latin letters after a line of Russian text, whose Latin reading English prose and Russian technical text use
// at most in this bucket of the counts (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS) and whose Cyrillic reading Russian prose
// uses, has evidence of its own under this prefix: the slip head (context_action_features.slip_question). A word of
// Russian prose typed with the English layout on stayed a suggestion near the threshold: `cgtie` for `спешу` after `я `
// at p=0.9885, `htity` for `решен`, and `pfdsk` for `завыл` was kept (Tatoeba development sentences, 23.10.2026).
pub const SLIP_FEATURE_PREFIX: &str = "slip|";

// Shorter tokens are left out: the Latin tokens of two or three letters typed as meant after Russian text are
// abbreviations whose keys happen to spell Russian prose (`dbg`, `ck`, `pf`, the captured typing of ru.stackoverflow), and
// among the class's captured questions they were the only ones kept.
pub const SLIP_HEAD_MIN_LETTERS: usize = 4;

pub const SLIP_HEAD_MAX_LATIN_BUCKET: usize = 1;

// The detector's probability is read in this many bands, the orthotactic margin in bands this wide, up to
// SLIP_ORTHO_BANDS either way, and how often Russian prose uses the word before the token in steps of this many binary
// orders of magnitude.
pub const SLIP_PROBABILITY_BANDS: i64 = 10;

pub const SLIP_ORTHO_BAND_WIDTH: f64 = 5.0;

pub const SLIP_ORTHO_BANDS: i64 = 4;

pub const SLIP_BEFORE_MAGNITUDE_STEP: i64 = 4;

// A letter of the variable head's class kept at its space and asked once more with the token after it, when that token
// holds no letter, has evidence of its own under this prefix: the sign head (context_action_features.sign_question). A
// Latin variable typed in the Russian layout before a formula sign stayed: `м` in `прохождения м + ` was kept at p=0.94
// and never asked again, and `ч` waiting before `=` was kept at p=0.95 (the owner's typing, 19.10.2026): the variable head
// reads every sign, dash and digit after the letter as one script.
pub const SIGN_FEATURE_PREFIX: &str = "sign|";

// The head reads a token of signs after the letter as itself up to this many characters (`=`, `+`, `==`, `+=`), one of
// digits as digits, and anything longer as one kind.
pub const SIGN_TOKEN_MAX_CHARACTERS: usize = 2;

// A token of dashes alone is no question of the head: after a letter in Russian prose it is the dash of a sentence
// (`серия М - это`, `буква е - пятая`), in a formula a minus (`n - 1`), and only the token after it tells them apart. Fitted
// with it, candidate v76 e2 turned 3 of 10 such Russian letters of the Leipzig development sentences into Latin ones.
pub const SIGN_DASH: &str = "-";

// The signs of an operator (`=`, `+`, `==`, `+=`, `*`, `%`): read as one kind beside the token itself, so that a sign the
// captured messages hold rarely (`+`, 63 converted frames) takes the evidence of the common one (`=`, 781).
pub const SIGN_OPERATOR_SIGNS: &str = "=+*%-";

// A short word right after Latin text, asked again once its next word is planned for conversion, has evidence of its own
// under this prefix: the resume head (context_action_features.resume_question). No head answered the question, and with
// Hunspell, which knows `f` and `z` as English words, the frozen model kept `а` typed `f` in `seek() , f там` at p=0.92
// and `я` typed `z` in `TSet - z использовал` at 0.99 where the person forgot to switch back to Russian (the
// developer-chat development set, 14 such messages, all converted without Hunspell): the class behind the tie of sealed
// test v78 in that profile alone (29.10.2026).
pub const RESUME_FEATURE_PREFIX: &str = "resume|";

// The one sign a short word may open with in either reading: a quotation mark or a parenthesis (`(и наверное`).
pub const RESUME_LEADING_SIGNS: &str = "\"«(";

// A Latin letter whose Cyrillic reading is a Russian word of one letter, typed after Russian text, has evidence of its own
// under this prefix: the Cyrillic-letter head (context_action_features.cyrletter_question), the mirror of the variable
// head. Every one-letter row of the corpus sits in quarantine, so the frozen model had no opinion on it: `в` typed `d` in
// `разрабатывал и d документ` was converted at p=0.998 and refused as a lone letter at its space and again beside its
// converted next word, and on public text the same shape read keep at 0.93 (the owner's typing: 17 such letters of the
// field batteries stayed Latin, 31.10.2026).
pub const CYRLETTER_FEATURE_PREFIX: &str = "cyrletter|";

// The boundaries the Cyrillic-letter head answers at: a space or a sign ends the letter. At a pause the letter may be the
// first of a Latin word still being typed after Russian text (`d` of `docker`), a question the head has no frame of.
pub const CYRLETTER_HEAD_TRIGGERS: &[&str] = &["space", "punctuation"];

// A token of one or two leading dashes and letters of one script, digits after them allowed, has evidence of its own
// under this prefix: the dash head (context_action_features.dash_question), a command-line flag typed in either layout.
// No head answered it, and the frozen model read the leading sign: `-v` typed after `git` became `-м` at p=0.9995, while
// `-цфеср` typed with the Russian layout on stayed where `-watch` was meant (the owner's typing, 31.10.2026).
pub const DASH_FEATURE_PREFIX: &str = "dash|";

// The most leading dashes a flag of the dash head's class has (`--force`).
pub const DASH_HEAD_MAX_DASHES: usize = 2;

// The boundaries the dash head answers at. At a pause the flag may still be being typed (`--for` of `--force`).
pub const DASH_HEAD_TRIGGERS: &[&str] = &["space", "punctuation", "enter"];

// Digits with `ю` typed in the Russian layout, digits after it allowed, have evidence of their own under this prefix: the
// digit-dot head (context_action_features.digit_dot_question). A person used to the English layout presses its dot key in
// Russian text too, and the Russian layout prints `ю` there: `2ю` for `2.`, `5ю2` for `5.2` (the owner's typing, five such
// tokens of the field batteries stayed). The engine replaces the sign and leaves the Russian layout on
// (engine.plan_from_decision). The comma's key prints `б`, and digits with `б` are written as meant too (`класс 5б`): fitted
// with them, candidate v81 e3 left every token of the class a suggestion, so they stay out of the class.
pub const DIGIT_DOT_FEATURE_PREFIX: &str = "digitdot|";

// A token of `@` and Latin letters whose other reading is `"` and Cyrillic letters, or the other way round, has evidence of
// its own under this prefix: the quote head (context_action_features.quote_question). The key of the Russian quotation mark
// types `@` in the English layout, so a Russian message begun in the other layout with a quotation mark opens with
// `@Z` for `"Я`; the frozen model left it (`@Z сейчас не могу` for `"Я сейчас не могу`, the Tatoeba development sentences
// of corpus v81, where the baseline pair converts it), and an `@` that opens a mention, a decorator or a CSS rule
// (`@media`, `@property`) is meant as typed.
pub const QUOTE_FEATURE_PREFIX: &str = "quote|";

// The two signs on the same key: `"` in the Russian layout, `@` in the English one.
pub const QUOTE_HEAD_SIGNS: &str = "@\"";

// The boundaries the quote head answers at.
pub const QUOTE_HEAD_TRIGGERS: &[&str] = &["space", "punctuation"];

// The boundary the digit-dot head answers at: the space after the token, the one boundary the sign is typed back before
// in the layout it belongs to.
pub const DIGIT_DOT_HEAD_TRIGGERS: &[&str] = &["space"];

// The letter of the Russian layout on the key of the English dot, and that sign.
pub const DIGIT_DOT_SIGN_LETTERS: &str = "ю";

pub const DIGIT_DOT_SIGNS: &str = ".";

// A token of UPPER_HEAD_MIN_LETTERS to UPPER_HEAD_MAX_LETTERS letters, all capitals, of one script, whose other reading is
// letters too, with no letter before it in the field or with more Cyrillic letters than Latin ones before it, has evidence
// of its own under this prefix: the upper-case head (context_action_features.upper_question). Every count the other heads
// and the shared features read is caseless, and a word in capitals is another word: the installed pair of 0.71.2 kept 9 of
// 74 Latin abbreviations typed in the Russian layout alone (`ЛЯЕ` for `KZT` at p=0.992: `ляе` is no word, but nothing said
// `KZT` is one), converted `BYN` typed as meant into `ИНТ` at p=0.994 (the short-message head counted `инт` 217 times and
// `byn` 6, none of them in capitals), turned `ЦА` sent alone into `WF` (the lone-word head, +10.8 to convert from caseless
// counts) and left 40 of 56 Russian abbreviations typed in the English layout alone as typed (`CCCH` for `СССР` kept at
// p=0.994; an authored battery on the engine, 10.11.2026). The head reads how often Russian web and news text and Russian
// technical text spell each reading in capitals (CAPITALS_FREQUENCY_FILE), whether the Latin one is a currency code of
// ISO 4217 and the Cyrillic one a form the Russian lexicon writes in capitals, how often Russian prose uses the Cyrillic
// one in any case against how often English prose or Russian technical text use the Latin one (a word typed with Caps
// Lock is that word: `YTN` for `НЕТ`), and no letters and no copy of the shared features: a head reading the letters of
// abbreviations learned the corpus's kinds by heart (`ТЗ` became `NP`, the abbreviation head). The lone-word,
// short-message and known-reply heads, which read caseless counts, no longer answer such a token.
pub const UPPER_FEATURE_PREFIX: &str = "upper|";

pub const UPPER_HEAD_MIN_LETTERS: usize = 2;

pub const UPPER_HEAD_MAX_LETTERS: usize = 5;

// The binary order of magnitude of a count is read up to this (`РФ` is spelled 84 627 times in capitals, 17 binary orders),
// and the difference of two readings' orders up to UPPER_RATIO_BOUND either way.
pub const UPPER_MAGNITUDE_MAX: i64 = 16;

pub const UPPER_RATIO_BOUND: i64 = 8;

// The head answers only a pair its counts settle (context_action_features.upper_settled), as the abbreviation and
// short-message heads do. A pair both of whose readings are spelled in capitals at least UPPER_ALIVE_MIN_CAPITALS times
// in one source of the capitals table, the larger within UPPER_ALIVE_MAX_ORDERS binary orders of the smaller, is alive:
// `СШ` 214 times and `CI` 389, `ЗК` 121 and `PR` 393, `ФЗ` 278 and `AP` 235, `ТП` 118 and `NG` 48. Its counts cannot tell
// such a pair's readings apart. A pair whose Cyrillic reading Russian prose uses at least UPPER_WORD_MIN_PROSE times in any
// case is alive too, whatever the capitals say: a word that common typed with Caps Lock is that word, which the capitals
// table does not count (`мы` 66 830 times against `VS` 3 141 in capitals). An alive pair stays with the frozen model, and
// the lone-word, short-message and known-reply heads answer it as in 0.73.0. Fitted on every pair, development candidate
// a4d2e926e7d0 of corpus v86 left `VS`, `NS`, `TOT` and `JY` sent alone for `МЫ`, `ТЫ`, `ЕЩЕ` and `ОН` (10 of 56 cases of
// the held-out half of the authored battery of words typed with Caps Lock, which 0.73.0 typed right). The values were
// chosen on that battery and on the calibration and development frames of corpus v86 (15.11.2026, a development candidate
// fitted for each): with 3 orders instead of 2, `AG` (576 in capitals) and `ФП` (90), `ФК` and `AR` are alive, and the
// calibration frames lose the two false conversions of `AG` to `ФП` and the lost conversion of `ФК` to `AR` that the head
// made (net 13 833 against 13 829, false 19 against 21, the pair of 0.73.0 13 825 and 19); with 32 instead of 50 the held-out
// half of the battery is right in 1 195 cases instead of 1 190 (0.73.0: 1 100), the frames are the same. The prose count
// is the least power of two above every abbreviation of the battery (`ок` 7 180, `ос` 6 526 times) and below every word of
// it typed with Caps Lock (`мы` 66 830 is the least); 65 536 would separate them as well, and no frame or case tells the
// two apart.
pub const UPPER_ALIVE_MIN_CAPITALS: u64 = 32;

pub const UPPER_ALIVE_MAX_ORDERS: i64 = 3;

pub const UPPER_WORD_MIN_PROSE: u64 = 8192;

// The "length" feature buckets every word length from here up together (schemas 2 and 3).
pub const ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS: usize = 6;

// Short categorical field labels (role, trigger) truncated before they reach a feature name.
pub const ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS: usize = 32;

// Right-context ("after") text considered for scripting/lexical features (schemas 2 and 3).
pub const ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS: usize = 128;

// Left-context ("before") text considered for scripting/lexical features (schemas 2 and 3).
pub const ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS: usize = 512;

// Source/target token text truncated before character and script features (schemas 2 and 3).
pub const ACTION_FEATURE_WORD_MAX_CHARACTERS: usize = 64;

// Neighbouring words taken from the left or right context.
pub const ACTION_FEATURE_NEIGHBOUR_WORD_COUNT: usize = 2;

// Characters of a neighbouring word used for its character features.
pub const ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS: usize = 24;

// Whitespace run between the word and its neighbour, truncated and used to normalise the
// whitespace-shape features.
pub const ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS: usize = 16;

// The literal tail and the boundary character(s) are both short trailing punctuation buffers
// normalised the same way.
pub const ACTION_FEATURE_SHORT_TEXT_CHARACTERS: usize = 8;

// Application identifier considered for the "app:" features (schemas 2 and 3).
pub const ACTION_FEATURE_APPLICATION_NAME_CHARACTERS: usize = 128;

// Alphanumeric tokens of the application identifier turned into features (schemas 2 and 3).
pub const ACTION_FEATURE_APPLICATION_TOKEN_COUNT: usize = 4;

// The orthotactic model's score and threshold share this magnitude range.
pub const ACTION_FEATURE_ORTHO_SCORE_BOUND: f64 = 128.0;

// Letters of a token in capitals that the capitals evidence describes: an abbreviation is three or
// more (`WBC`, `ГАК`); two-letter tokens stay with the deferral rules and the owner's `ГА`.
pub const ACTION_FEATURE_CAPITALS_MIN_LETTERS: usize = 3;

// ContextModel feature_version of the context-action (orthotactic-aware) scheme, feature schema 3.
pub const CONTEXT_ACTION_FEATURE_VERSION: usize = 3;

// Longest prefix the engine offers the prefix model, the characters prefix features (schema two)
// encode, and the longest prefix a prefix corpus may generate.
pub const PREFIX_MAX_CHARACTERS: usize = 12;

// Newest prefix feature schema; schema one stays frozen in prefix_model.py.
pub const CURRENT_PREFIX_FEATURE_VERSION: usize = 2;

// Longest character n-gram of an observed prefix (schema two).
pub const MAX_PREFIX_CHARACTER_NGRAM_ORDER: usize = 4;

pub const PREFIX_CHARACTER_FEATURE_WEIGHT_CAP: f64 = 2.0;

// Schema three: the orthotactic models read the keys of the prefix (crates/ks-context/src/prefix_schema.rs).
pub const ORTHO_PREFIX_FEATURE_VERSION: usize = 3;

// A prefix's orthotactic evidence (other layout's language against the typed one's, in nats) scaled into [-1, 1]:
// the sum over its keys, the mean per key and the worst key of each reading.
pub const PREFIX_ORTHO_SUM_BOUND_NATS: f64 = 60.0;
pub const PREFIX_ORTHO_MEAN_BOUND_NATS: f64 = 15.0;
pub const PREFIX_ORTHO_WORST_BOUND_NATS: f64 = 30.0;

// The sum also goes in as a step of this many nats, at most this many steps either way, per length bucket.
pub const PREFIX_ORTHO_STEP_NATS: f64 = 5.0;
pub const PREFIX_ORTHO_MAX_STEPS: i64 = 8;

// Length buckets of the orthotactic evidence: a key count at or above the last one shares its bucket.
pub const PREFIX_ORTHO_LENGTH_BUCKETS: [usize; 4] = [3, 4, 5, 6];

// Schema four: schema three, the term lists the program loads, each reading's own orthotactic score and a whole word at
// the start of the typed keys (crates/ks-context/src/prefix_schema.rs, prefix_terms.rs).
pub const TERMS_PREFIX_FEATURE_VERSION: usize = 4;

// The fewest letters of a reading the term lists are asked about: shorter ones begin too many terms to say anything.
pub const PREFIX_TERM_MIN_CHARACTERS: usize = 3;

// A term table count goes in as ln(1 + count) over this: the largest counts, some tens of thousands, come near one.
pub const PREFIX_TERM_COUNT_LOG_DIVISOR: f64 = 10.0;

// Each reading's mean log-probability per key by its own language's channel, in steps of this many nats down from zero, at
// most this many steps; and the two readings together, in coarser steps.
pub const PREFIX_ORTHO_ABSOLUTE_STEP_NATS: f64 = 1.0;
pub const PREFIX_ORTHO_ABSOLUTE_MAX_STEPS: i64 = 8;
pub const PREFIX_ORTHO_BOTH_STEP_NATS: f64 = 2.0;
pub const PREFIX_ORTHO_BOTH_MAX_STEPS: i64 = 3;

// A whole word or term at the start of the typed keys: from this many letters, its length counted up to the next bound, and
// the length of the keys after it up to the last.
pub const PREFIX_HEAD_MIN_CHARACTERS: usize = 3;
pub const PREFIX_HEAD_MAX_LENGTH_BUCKET: usize = 6;
pub const PREFIX_TAIL_MAX_LENGTH_BUCKET: usize = 4;

// Lowest conversion threshold a prefix model artifact may carry.
pub const MIN_PREFIX_CONVERSION_THRESHOLD: f64 = 0.985;

pub const MAX_PREFIX_WEIGHTS: usize = 10000;

pub const MAX_PREFIX_FEATURE_NAME_CHARACTERS: usize = 160;

// Conversion threshold of the context-v1 model: ContextModel's default, and the threshold the
// context-v1 trainer selects its epoch with and writes into the artifact.
pub const CONTEXT_V1_CONVERSION_THRESHOLD: f64 = 0.985;

// An action model's keep with the other layout at least this likely is a suggestion instead (ContextModel.predict):
// the model is unsure, and at a space the engine asks once more beside the next word, as after a conversion below
// the threshold. `зк` after `принимай ` was kept at p=0.51 against 0.49 for `pr` (the owner's typing, 08.10.2026);
// asked beside its next word the same model converts it at p=1.00.
pub const CONTEXT_DOUBTFUL_KEEP_PROBABILITY: f64 = 0.3;

// ContextModel feature_version of the context-v1/v2 scheme, feature schema 2; context_model.py
// exports it as FEATURE_VERSION.
pub const CONTEXT_MODEL_FEATURE_VERSION: usize = 2;

// ContextModel feature_version of the context-v1 scheme with typo evidence, feature schema 5: schema 2
// plus whether each reading is one typo away from a word the lexicon knows, and in which layout a
// token with a digit was typed. Schema 4 was an experiment that never shipped.
pub const CONTEXT_TYPO_FEATURE_VERSION: usize = 5;

// Schema 5: the shortest reading checked for being one typo away from a word; nearly every shorter
// string has a neighbour in a lexicon of this size.
pub const CONTEXT_TYPO_MIN_CHARACTERS: usize = 5;

// ContextModel feature_version of the context-v1 scheme with opening evidence, feature schema 6:
// schema 5 plus whether each reading is a word that opens sentences of its language
// (short_words.OPENING_WORDS). The frequency lexicons are encyclopedic and rank the conversational
// `ты` next to the English token `ns`, so without it a model could tell them apart only by letters.
pub const CONTEXT_OPENING_FEATURE_VERSION: usize = 6;

// ContextModel feature_version of the context-v1 scheme with line, case and term evidence, feature
// schema 7: schema 6 plus where on its line the word stands (the first word of a message or of a new
// line against a word with words before it), the case pattern of the token (an inner capital, all
// capitals), a dot or underscore inside its Latin reading, how often each reading occurs inside
// Russian text (a Latin term like `id`, Cyrillic slang like `пдф`) and in text of its own language
// (`uh` is an English word, `гр` a rare Russian one), and whether the token has letters at all - the
// questions the engine asks while real mixed Russian and English text is typed made these the
// evidence that decides.
pub const CONTEXT_LINE_FEATURE_VERSION: usize = 7;

// Schemas that carry every schema 6 feature (schema 7 adds its own on top).
pub const CONTEXT_OPENING_SCHEMAS: &[usize] = &[6, 7];

// Schema 7: the occurrence counts that open the term frequency buckets above the first, in every
// table; a reading seen fewer times than the first bound is in bucket 0, as one never seen.
pub const CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS: &[usize] = &[5, 50, 500, 5000];

// The lowest term frequency bucket of a Russian abbreviation or slang word the context policy keeps
// as typed (context_policy.russian_slang): fifty occurrences or more inside Russian technical text.
pub const RUSSIAN_SLANG_MIN_TERM_BUCKET: usize = 2;

// Feature schemas a context artifact may carry: context-v1's and the context action scheme's.
pub const CONTEXT_SUPPORTED_FEATURE_VERSIONS: &[usize] = &[2, 5, 6, 7, 3];

// Feature schema 2 only: words of the left and of the right context that become word features, and
// the character n-gram orders of the two readings.
pub const CONTEXT_FEATURE_BEFORE_WORD_COUNT: usize = 6;

pub const CONTEXT_FEATURE_AFTER_WORD_COUNT: usize = 3;

pub const CONTEXT_FEATURE_NGRAM_ORDERS: &[usize] = &[1, 2, 3];

// What the context model loader accepts (schemas 2 and 3): at most this many features, feature names
// and version strings of at most these many characters, weights of at most this magnitude, and a
// conversion threshold from this floor up to one.
pub const MAX_CONTEXT_MODEL_FEATURES: usize = 50000;

// An action model (schema 3) holds up to sixteen feature spaces: the word decided at its boundary, the
// kept-neighbour question (KEPT_FEATURE_PREFIX), the capitals head (CAPITALS_FEATURE_PREFIX), the
// lone-word head (ALONE_FEATURE_PREFIX), the message-start head (START_FEATURE_PREFIX), the
// single-letter head (LETTER_FEATURE_PREFIX), the abbreviation heads (ABBREVIATION_FEATURE_PREFIX,
// LATIN_ABBREVIATION_FEATURE_PREFIX), the unknown-token head (UNKNOWN_FEATURE_PREFIX), the joined-token head
// (JOINED_FEATURE_PREFIX), the term heads (TERM_FEATURE_PREFIX, FIRST_FEATURE_PREFIX), the short-message head
// (BRIEF_FEATURE_PREFIX), the variable head (VARIABLE_FEATURE_PREFIX), the dotted-abbreviation head
// (DOTTED_FEATURE_PREFIX), the letters-and-digits head (ALNUM_FEATURE_PREFIX), the hypothesis head
// (HYPOTHESIS_FEATURE_PREFIX), the short-token head (SHORT_FEATURE_PREFIX), the common-reading head
// (COMMON_FEATURE_PREFIX), the slip head (SLIP_FEATURE_PREFIX), the
// sign head (SIGN_FEATURE_PREFIX), the resume head (RESUME_FEATURE_PREFIX), the Cyrillic-letter head
// (CYRLETTER_FEATURE_PREFIX), the dash head (DASH_FEATURE_PREFIX), the digit-dot head (DIGIT_DOT_FEATURE_PREFIX), the
// quote head (QUOTE_FEATURE_PREFIX), the known-reply head (KNOWN_REPLY_FEATURE_PREFIX), the upper-case head
// (UPPER_FEATURE_PREFIX) and the feel head (FEEL_FEATURE_PREFIX), each with its own budget in the recipe.
// Raised from 100 000 with the unknown-token head (the 0.54.0 pair holds 98 498), and from 110 000 with the hypothesis
// and short-token heads: the 0.61.0 pair holds 107 808, about 8.6 MB written one feature to a line, and
// MAX_CONTEXT_ACTION_MODEL_BYTES bounds the file.
pub const MAX_CONTEXT_ACTION_MODEL_FEATURES: usize = 150_000;

pub const MAX_CONTEXT_FEATURE_NAME_CHARACTERS: usize = 512;

pub const MAX_CONTEXT_MODEL_VERSION_CHARACTERS: usize = 80;

pub const MAX_CONTEXT_WEIGHT_MAGNITUDE: i64 = 1000;

pub const MIN_CONTEXT_CONVERSION_THRESHOLD: f64 = 0.95;

// ---- constants/prefix.py

// feature_version of the schema-one prefix features (prefix_model.features) and of their artifact.
pub const PREFIX_V1_FEATURE_VERSION: usize = 1;

// Schema-one features: the one-hot length feature buckets every length from here up together; the
// continuous length feature is the length capped at and divided by the scale.
pub const PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS: usize = 12;

pub const PREFIX_V1_LENGTH_SCALE_CHARACTERS: usize = 24;

// log1p of the completion count and of the best completion frequency is divided by these.
pub const PREFIX_V1_COMPLETIONS_LOG_DIVISOR: usize = 10;

pub const PREFIX_V1_FREQUENCY_LOG_DIVISOR: usize = 20;

// The word n-gram score is clamped to this magnitude and divided by it.
pub const PREFIX_V1_NGRAM_SCORE_BOUND: f64 = 3.0;

// Text before the prefix that the context features read.
pub const PREFIX_V1_BEFORE_CONTEXT_CHARACTERS: usize = 512;

// Quote marks pair up: an odd count on the line leaves one open.
pub const PREFIX_V1_QUOTE_MARKS_PER_PAIR: usize = 2;

// Characters of the application identifier, and its alphanumeric tokens, that become "app:" features.
pub const PREFIX_V1_APPLICATION_NAME_CHARACTERS: usize = 128;

pub const PREFIX_V1_APPLICATION_TOKEN_COUNT: usize = 4;

// Last words of the text before the prefix that become "before:word:" features.
pub const PREFIX_V1_BEFORE_WORD_COUNT: usize = 2;

// ---- constants/text.py

// The most characters a field reader may report its caret short of the end of what was typed and
// still be taken as the same text: VS Code Insiders (06.10.2026) reported the caret of its chat box
// one character early, the last typed letter after it, and every word there was refused.
pub const FIELD_CARET_LAG_MAX_CHARACTERS: usize = 2;

// The most of the last typed letters a field read as their key goes down may not show yet: a browser
// or an Electron editor reports the text a moment after the key (engine.typed_in_field).
pub const FIELD_PENDING_MAX_CHARACTERS: usize = 2;

// The most an editor's input buffer shows when it is what UI Automation reads instead of the text: VS Code with its
// screen-reader support off keeps the input it takes keys through empty, and a read shows at most the key in flight
// (FieldContext.input_buffer_only).
pub const FIELD_INPUT_BUFFER_MAX_CHARACTERS: usize = 1;

// Field text kept on each side of the caret, typed or read (input_context exports it as
// CONTEXT_LIMIT), and the longest application name, field identifier and source label a field
// context carries.
pub const FIELD_CONTEXT_MAX_CHARACTERS: usize = 512;

pub const FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS: usize = 128;

pub const FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS: usize = 128;

pub const FIELD_CONTEXT_SOURCE_MAX_CHARACTERS: usize = 32;

// ---- constants/timing.py

// Typed field context older than this is dropped, and the engine waits as long for the neighbour of a
// waiting word (input_context exports it as CONTEXT_TTL).
pub const FIELD_CONTEXT_TTL_SECONDS: f64 = 45.0;

// ---- constants/training.py

// Only a tail that ends many words is framed as kept: its ending bucket (context_action_features.russian_ending_bucket)
// at least this. The tail of a rare word (`ыр` of `сыр`) ends as few words as a term's keys, and kept frames of such
// tails taught the head to keep `зк` and `вум` (experiment E5). From an ending bucket of three up, the class's tokens and
// the tails of words are the same tokens (`ац`, `мь`, `дые`): `дык` reopened with Backspace and finished as `дые` went to
// `lst`, which Pause undid (a field log of 24.09.2026), and a head without those tails repeated it (experiment E8).
pub const TERM_FRAGMENT_MIN_ENDING_BUCKET: usize = 3;

// ---- constants/typo.py

// Fewest letters a word needs to be corrected: a short word lies one slip away from many others (`кот`, `rat`, `рот`),
// and no one of them is clearly the word meant.
pub const TYPO_MIN_LETTERS: usize = 5;

// Fewest occurrences in the counted list (OpenSubtitles 2018 via FrequencyWords) a corrected word needs: a rarer form
// is too uncertain a guess to type over the user's word. tools/evaluate_typo_correction.py measures other floors too.
pub const TYPO_CANDIDATE_MIN_COUNT: usize = 20;

// How many times the posterior of the best candidate must exceed the posteriors of all the other candidates together:
// a word with two plausible readings is left as typed. The written list is held to the same ratio: on the development
// slips that leaves 38,924 right corrections and 127 wrong ones, where asking written text only to prefer the choice
// (a ratio of one) leaves 39,297 and 188.
pub const TYPO_POSTERIOR_RATIO: f64 = 3.0;

// Fewest times the written list (the `russian` and `english` sections of context-term-frequency.json: words of the
// training parts of Stack Overflow, Tatoeba and Taiga, each counted five times or more) must count a corrected word.
// The subtitle list holds stumps of words cut at a line's end (`кажетс`, `сегодн`) and names films say (`форман`); of
// its Russian forms of five letters and more counted twenty times, the reference Hunspell dictionary (offline, never at
// run time) rejects 31 % of those written text never counts, 4.0 % of those it counts five to nine times, 3.0 % of ten
// to nineteen and 2.4 % of twenty to forty-nine. On the development texts (`ks-train typo-evaluation`, the reporting half of the
// synthetic slips) the subtitle list alone makes 40,121 right corrections and 337 wrong ones; held to written text at
// 5, 10 and 20 it makes 39,200, 38,924 and 38,495 right ones and 128, 127 and 127 wrong ones, and of correctly typed
// Russian Tatoeba sentences (627,393 words) it corrects 23, 17 and 14 words where it alone corrected 41.
pub const TYPO_WRITTEN_MIN_COUNT: usize = 10;

// The section of the written list that counts the words of each language's text.
pub const TYPO_WRITTEN_SECTIONS: &[(&str, &str)] = &[("en_US", "english"), ("ru_RU", "russian")];

// Natural logarithms of the probability of one particular slip (the noisy channel), by the operation that undoes it.
// Only their differences change a decision. Tuned on the tuning half of the synthetic development typos of
// tools/evaluate_typo_correction.py (equal shares of the four operations: no source we could open gives the shares
// for Russian typing): a swap of two neighbouring letters is the likeliest single slip, a letter left out next, a
// letter typed with a neighbouring key and a letter typed in excess (a key pressed twice or one brushed beside the
// key meant) after them.
pub const TYPO_TRANSPOSITION_LOG_PROBABILITY: f64 = -1.4;

pub const TYPO_INSERTION_LOG_PROBABILITY: f64 = -2.5;

pub const TYPO_DELETION_LOG_PROBABILITY: f64 = -5.0;

pub const TYPO_SUBSTITUTION_LOG_PROBABILITY: f64 = -4.0;

// Letters a transposition swaps.
pub const TYPO_TRANSPOSED_LETTERS: usize = 2;

// The physical key rows of layouts.US_KEYS and layouts.RU_KEYS, in order: the key left of `1`, then the three letter
// rows. Each lower row sits half a key to the right of the one above it, so a key touches the key above it and the one
// after that; the key of the top row stands above Tab and touches no letter.
pub const KEYBOARD_ROW_LENGTHS: &[usize] = &[1, 13, 11, 9];

// Resource format (tools/build_typo_frequency.py writes it, typo_correction.TypoLexicon reads it).
pub const TYPO_FREQUENCY_SCHEMA_VERSION: usize = 1;

// Largest typo frequency file loaded; the shipped ones are about two mebibytes each.
pub const TYPO_FREQUENCY_MAX_BYTES: usize = 8_388_608;

// Stored counts are natural logarithms times this scale, rounded, one byte each: the decision compares ratios, and an
// eighth of a natural logarithm (thirteen per cent) is finer than any threshold it uses.
pub const TYPO_COUNT_LOG_SCALE: usize = 8;

// Bytes of the SHA-256 digest of a form read as each of the two hashes the membership filter positions are made from.
pub const TYPO_OCCURRENCE_HASH_BYTES: usize = 8;

// The sections after the header line, in order: the stored forms and their counts (each compressed with zlib), then
// the membership filter's bits.
pub const TYPO_SECTIONS: &[&str] = &["forms", "weights", "seen"];

// The byte that ends each stored form; the letters are coded from one up, in the order of their alphabet.
pub const TYPO_FORM_SEPARATOR: usize = 0;
/// How many sorted forms of a typo list share one offset: a lookup finds the block by binary search and walks it.
pub const TYPO_BLOCK_FORMS: usize = 16;
/// The most bytes a stored form of a typo list keeps of the form before it: the count is one byte.
pub const TYPO_SHARED_MAX_BYTES: usize = 255;

// The letters of each language's forms, in their coding order.
pub const TYPO_ALPHABETS: &[(&str, &str)] = &[("en_US", "abcdefghijklmnopqrstuvwxyz"), ("ru_RU", "абвгдеёжзийклмнопрстуфхцчшщъыьэюя")];

// ---- constants/units.py

pub const BITS_PER_BYTE: usize = 8;

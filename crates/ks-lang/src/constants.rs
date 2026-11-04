//! Values used by the modules of this crate, each once, as in the Python's `constants` folder.

/// `constants/units.py`.
const BYTES_PER_KIBIBYTE: usize = 1024;
const BYTES_PER_MEBIBYTE: usize = 1024 * BYTES_PER_KIBIBYTE;

// ---------------------------------------------------------------- file_formats.py
pub const ARPA_BIGRAM_SECTION: u32 = 2;
pub const IDENTIFIER_LEXICON_MAX_BYTES: usize = 4 * BYTES_PER_MEBIBYTE;
pub const IDENTIFIER_LEXICON_MAX_ENTRIES: usize = 200_000;
pub const VERSION_HASH_CHARACTERS: usize = 12;
pub const LEXICON_SUPPLEMENT_MAX_BYTES: usize = 8 * BYTES_PER_MEBIBYTE;

// ---------------------------------------------------------------- models.py
pub const SYNTHETIC_FREQUENCY_FLOOR: i64 = 1000;
pub const SYNTHETIC_FREQUENCY_DIVISOR: i64 = 20;
pub const CONTEXT_TYPO_MIN_CHARACTERS: usize = 5;
pub const LANGUAGE_MODEL_NGRAM_ORDERS: [usize; 3] = [2, 3, 4];
pub const LANGUAGE_MODEL_TRIGRAM_ORDER: usize = 3;
pub const LANGUAGE_MODEL_MIN_WORD_CHARACTERS: usize = 2;
pub const LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT: usize = 12_000;
pub const LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS: usize = 3;
pub const LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN: f64 = -10.0;
pub const LANGUAGE_MODEL_MIN_NGRAM_DEVIATION: f64 = 0.05;
pub const LANGUAGE_MODEL_MAX_GRAM_WEIGHT: i64 = 32;
pub const LANGUAGE_MODEL_GRAM_SMOOTHING: f64 = 0.2;
pub const LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY: usize = 2048;
pub const LANGUAGE_MODEL_EMPTY_TOKEN_SCORE: f64 = -30.0;
pub const LANGUAGE_MODEL_NATURALNESS_MIN: f64 = -15.0;
pub const LANGUAGE_MODEL_NATURALNESS_MAX: f64 = 4.0;
pub const LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR: f64 = -4.0;
pub const LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE: f64 = 7.0;
pub const LANGUAGE_MODEL_POPULARITY_WEIGHT: f64 = 3.0;
pub const LANGUAGE_MODEL_SPELL_KNOWN_SCORE: f64 = 6.5;
pub const LANGUAGE_MODEL_NATURALNESS_WEIGHT: f64 = 1.15;
pub const LANGUAGE_MODEL_INVALID_RATIO_WEIGHT: f64 = 0.75;
pub const LANGUAGE_MODEL_DELETION_MIN_CHARACTERS: usize = 4;
pub const LANGUAGE_MODEL_DELETION_POSITIONS: usize = 12;

// ---------------------------------------------------------------- text.py
pub const HUNSPELL_MAX_CHECK_CHARACTERS: usize = 128;

// ---------------------------------------------------------------- detection.py
pub const ACRONYM_MIN_LETTERS: usize = 2;

// ---------------------------------------------------------------- settings_defaults.py
pub const DEFAULT_EARLY_SWITCH_MIN_LENGTH: usize = 4;

// ---------------------------------------------------------------- ortho.py
/// Largest artifact file read (`MAX_ARTIFACT_BYTES` of `ortho_model`).
pub const ORTHO_ARTIFACT_MAX_BYTES: usize = 24 * BYTES_PER_MEBIBYTE;
/// Most n-grams one language channel may carry (`MAX_GRAMS`).
pub const ORTHO_CHANNEL_MAX_GRAMS: usize = 4_000_000;
/// Orders of the character model an artifact may declare (`MIN_ORDER`, `MAX_ORDER`).
pub const ORTHO_MIN_ORDER: i64 = 2;
pub const ORTHO_MAX_ORDER: i64 = 8;
/// Largest quantisation scale an artifact may declare: integer steps per nat of every stored weight and threshold.
pub const ORTHO_MAX_SCALE: i64 = 4096;
/// Largest shortest-scored-token length an artifact may declare.
pub const ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS: i64 = 32;
/// A stored backoff weight is a [context, weight] pair.
pub const ORTHO_BACKOFF_ENTRY_FIELDS: usize = 2;
/// The artifact schema that carries feature weights, and the number of parts a hyphenated token splits into for the
/// compound features.
pub const ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION: i64 = 2;
pub const ORTHO_COMPOUND_PARTS: usize = 2;
/// Artifact schema 3: schema 2 and the weight of `extra_key`.
pub const ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION: i64 = 3;
/// The shortest run of one key an artifact may collapse a longer run to (`collapse_runs`).
pub const ORTHO_MIN_COLLAPSED_RUN: i64 = 2;
/// An artifact of schema 4 is schema 3 with `stretch_runs`; the shortest such run is two keys: one key is no run.
pub const ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION: i64 = 4;
pub const ORTHO_MIN_STRETCH_RUN: i64 = 2;
/// An artifact of schema 5 is schema 4 that also reads a hyphen between two copies of one key as a drawn-out sound
/// and weighs whether the replacement is a word.
pub const ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION: i64 = 5;
/// An artifact of schema 6 is schema 5 that also weighs how word-like the replacement is in its own language and may
/// count a feature only for a token less plausible than the centre of the layout typed.
pub const ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION: i64 = 6;

// ---------------------------------------------------------------- packing.py
/// A gram's characters become one byte each, 1 to 255; a zero byte marks a character the table never holds.
pub const PACKED_GRAM_LAST_CODE: usize = 255;
/// The bytes of a gram are read as one signed 64-bit key, so a gram may have at most this many characters.
pub const PACKED_GRAM_KEY_BYTES: usize = 8;
pub const PACKED_GRAM_MAX_CHARACTERS: usize = PACKED_GRAM_KEY_BYTES - 1;
/// Below this many entries a table is left as it is: packing saves nothing worth the slower lookups.
pub const PACKED_GRAM_MIN_ENTRIES: usize = 4096;

// ---------------------------------------------------------------- prefix.py
/// The legacy early-switch rule: a prefix impossible in its own language switches when its rendering in the other
/// layout starts at least this many words, one of them at least this frequent, or one word dominant by frequency.
pub const EARLY_SWITCH_MIN_TARGET_COMPLETIONS: i64 = 10;
pub const EARLY_SWITCH_MIN_TARGET_FREQUENCY: i64 = 2000;
pub const EARLY_SWITCH_DOMINANT_TARGET_FREQUENCY: i64 = 100_000;
/// A prefix index scans at most this many matching words for their best frequency; the count of matches is exact.
pub const PREFIX_INDEX_COMPLETION_SCAN_LIMIT: usize = 4096;
/// A Hunspell .dic larger than this adds no stems to a prefix index.
pub const PREFIX_INDEX_DICTIONARY_MAX_BYTES: usize = 16 * BYTES_PER_MEBIBYTE;

// ---------------------------------------------------------------- boundary.py
/// Keys that are punctuation in the US layout and letters in the Russian one. A completed token's trailing run of
/// them is its ambiguous tail; the v2 features list them in this order.
pub const BOUNDARY_AMBIGUOUS_PUNCTUATION: &str = ",.;[]'`";
/// A completed token keeps at most this many literal trailing characters (`boundary_model.MAX_SUFFIX`).
pub const BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS: i64 = 8;
pub const BOUNDARY_V1_FEATURE_VERSION: i64 = 1;
pub const BOUNDARY_V2_FEATURE_VERSION: i64 = 2;
/// Feature version 3 adds, for each reading, whether it is a known word with one letter missing (not the last one).
pub const BOUNDARY_V3_FEATURE_VERSION: i64 = 3;
pub const BOUNDARY_SUPPORTED_FEATURE_VERSIONS: [i64; 2] = [BOUNDARY_V2_FEATURE_VERSION, BOUNDARY_V3_FEATURE_VERSION];
/// Letters a misspelled reading may be missing, by locale of the word model that reads it.
pub const BOUNDARY_MISSING_LETTERS: [(&str, &str); 2] =
    [("en_US", "abcdefghijklmnopqrstuvwxyz"), ("ru_RU", "абвгдеёжзийклмнопрстуфхцчшщъыьэюя")];
/// Every boundary-v2 artifact version starts with this; the trainer appends a digest of the weights.
pub const BOUNDARY_V2_VERSION_PREFIX: &str = "boundary-v2-";
/// Candidate features: the word length is capped at and divided by the scale; log1p of the word frequency is divided
/// by the divisor; the n-gram score is clamped to the bound and divided by it.
pub const BOUNDARY_FEATURE_LENGTH_SCALE_CHARACTERS: i64 = 24;
pub const BOUNDARY_FEATURE_FREQUENCY_LOG_DIVISOR: f64 = 20.0;
pub const BOUNDARY_FEATURE_NGRAM_SCORE_BOUND: f64 = 3.0;
/// An artifact's threshold must exceed one half: a decided span then outweighs all others together.
pub const BOUNDARY_THRESHOLD_EXCLUSIVE_MIN: f64 = 0.5;
/// A boundary-v2 artifact larger than this is refused unread.
pub const BOUNDARY_POLICY_MAX_BYTES: usize = 64 * BYTES_PER_KIBIBYTE;

"""JSON indentation, schema and format versions, size caps, hash and prefix lengths."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_KIBIBYTE, BYTES_PER_MEBIBYTE

# Indentation of the settings and learning files the application keeps for the user.
USER_DATA_JSON_INDENT: Final = 2
# Decimal places a correction's confidence keeps in the history file.
HISTORY_CONFIDENCE_DECIMALS: Final = 2
IDENTIFIER_LEXICON_MAX_BYTES: Final = 4 * BYTES_PER_MEBIBYTE
IDENTIFIER_LEXICON_MAX_ENTRIES: Final = 200000
# Hex characters of a SHA-256 kept in a readable model or resource version string
# (context-v1-<hash>, context-v3-<hash>, prefix-v2-<hash>, intent-v1-<hash>, identifiers-<hash>)
# and in the intent model's status summary (sha256:<hash>).
VERSION_HASH_CHARACTERS: Final[int] = 12
# The text after the ":" separator from str.partition(":"), which always returns a 3-tuple (before,
# separator, after).
PARTITION_AFTER_SEPARATOR_INDEX: Final = 2
# schema_version of the learning file: 3 keeps a list of rules from the rule window; 2 and older
# kept counters of manual conversions and forbidden directions, converted on first load.
LEARNING_STORE_SCHEMA_VERSION: Final = 3
# Name suffix of the copy an older learning file is kept as before it is converted.
LEARNING_STORE_BACKUP_SUFFIX: Final = ".v2-backup"
# The confirmation threshold a learning file of schema 2 was used with unless the settings said
# otherwise (the removed detection.learning_confirmations defaulted to it): a counter that reached
# it acted as a rule and is carried over as one.
LEGACY_RULE_CONFIRMATIONS_REQUIRED: Final = 2
LEXICON_SUPPLEMENT_MAX_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Largest prefix model artifact loaded.
MAX_PREFIX_MODEL_BYTES: Final = 2 * BYTES_PER_MEBIBYTE
# Largest context model artifact loaded (context_model.py exports it as MAX_ARTIFACT_BYTES).
MAX_CONTEXT_MODEL_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Largest term frequency table the context model reads with feature schema 7.
MAX_CONTEXT_TERM_FREQUENCY_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Indentation of every diagnostics report printed or copied for a person to read.
DIAGNOSTICS_JSON_INDENT: Final = 2
# Read size when a file is streamed through SHA-256 (and copied while hashed).
HASH_CHUNK_BYTES: Final[int] = BYTES_PER_MEBIBYTE
# Hex characters of a SHA-256 digest.
SHA256_HEX_CHARACTERS: Final = 64
# Indentation of the JSON result a verifier, evaluator or trainer prints for a person to read.
REPORT_JSON_INDENT: Final[int] = 2
# Radix a hex digest prefix is parsed in when it becomes a deterministic integer.
HEXADECIMAL_BASE: Final = 16
# Added to zlib.MAX_WBITS, this makes zlib.decompressobj read and check the gzip header and trailer
# itself, so a multi-member gzip file splits member by member.
ZLIB_GZIP_HEADER_WBITS_OFFSET: Final = 16
# Largest KSLM container (.ksm) the intent runtime loads and the tools accept.
KSLM_MAX_CONTAINER_BYTES: Final[int] = 14 * BYTES_PER_MEBIBYTE
# Limits of a KSLM container's parts.
KSLM_MAX_MANIFEST_BYTES: Final[int] = BYTES_PER_MEBIBYTE
KSLM_MAX_PAYLOAD_BYTES: Final[int] = 12 * BYTES_PER_MEBIBYTE
KSLM_MAX_FINGERPRINTS: Final[int] = 1 << 20
# KSLM schema 4: a minimal embedded-manifest JSON object ("{}"), then int16 weight and uint64
# fingerprint records.
KSLM_SCHEMA_VERSION: Final[int] = 4
KSLM_MIN_MANIFEST_BYTES: Final[int] = 2
KSLM_WEIGHT_ENTRY_BYTES: Final[int] = 2
KSLM_FINGERPRINT_ENTRY_BYTES: Final[int] = 8
# A KSLM model dimension is a power of two up to 2 ** KSLM_MAX_DIMENSION_LOG2.
KSLM_MAX_DIMENSION_LOG2: Final[int] = 21
KSLM_MAX_DIMENSION: Final[int] = 1 << KSLM_MAX_DIMENSION_LOG2
# Weights are quantised symmetrically into int16: the largest magnitude maps to this value. The
# float twin divides a float weight without changing its type.
KSLM_QUANTIZED_WEIGHT_LIMIT: Final[int] = 32767
KSLM_QUANTIZED_WEIGHT_LIMIT_FLOAT: Final[float] = float(KSLM_QUANTIZED_WEIGHT_LIMIT)
# All 64 bits set: the largest uint64 (seeds, fingerprints, metadata integers) and the mask that
# keeps FNV-1a arithmetic in 64 bits.
UINT64_MASK: Final[int] = (1 << 64) - 1
# The smallest int64 a KSLM metadata integer may hold.
INT64_MIN: Final[int] = -(1 << 63)
# Keeps zlib.crc32 an unsigned 32-bit value.
UINT32_MASK: Final[int] = 0xFFFFFFFF
# Permissions of a published model artifact and of the files staged beside it.
PUBLISHED_MODEL_FILE_MODE: Final[int] = 0o644
# The ARPA section ("\2-grams:") holding bigrams; section 1 holds unigrams.
ARPA_BIGRAM_SECTION: Final[int] = 2
# Largest intent training config, frozen source file, publication backup and seal registry the
# intent trainer reads.
INTENT_TRAINING_CONFIG_MAX_BYTES: Final[int] = 64 * BYTES_PER_KIBIBYTE
INTENT_FROZEN_SOURCE_MAX_BYTES: Final[int] = 64 * BYTES_PER_MEBIBYTE
INTENT_PUBLICATION_BACKUP_MAX_BYTES: Final[int] = 64 * BYTES_PER_MEBIBYTE
INTENT_SEAL_REGISTRY_MAX_BYTES: Final[int] = 16 * BYTES_PER_KIBIBYTE
# Largest external-evaluation manifest, Hunspell affix and Hunspell dictionary the intent evaluator
# reads.
INTENT_EXTERNAL_MANIFEST_MAX_BYTES: Final[int] = BYTES_PER_MEBIBYTE
HUNSPELL_AFFIX_MAX_BYTES: Final[int] = BYTES_PER_MEBIBYTE
HUNSPELL_DICTIONARY_MAX_BYTES: Final[int] = 64 * BYTES_PER_MEBIBYTE
# schema_version of the intent training config, of its external_evaluation and
# hard_negative_development sections (the hard-negative provenance carries the same number), and of a
# presealed candidate record.
INTENT_TRAINING_CONFIG_SCHEMA_VERSION: Final[int] = 13
INTENT_EXTERNAL_EVALUATION_SCHEMA_VERSION: Final[int] = 2
HARD_NEGATIVE_DEVELOPMENT_SCHEMA_VERSION: Final[int] = 2
PRESEALED_CANDIDATE_RECORD_SCHEMA_VERSION: Final[int] = 2
# schema_version of the intent model manifest. Schema 2 adds toolchain.constants_sha256, the digest
# of the constants the toolchain files import; a schema 1 manifest predates it and is refused.
INTENT_MANIFEST_SCHEMA_VERSION: Final[int] = 2
# schema_version of an environment probe measurement.
ENVIRONMENT_PROBE_SCHEMA_VERSION: Final[int] = 1
# Indentation of the frozen hard-negative development corpus, of the preseal receipt, and of the
# JSON the intent trainer writes beside the KSLM (manifest, report, build environment, diagnostic)
# and prints; the bytes of all of them are sealed.
HARD_NEGATIVE_CORPUS_JSON_INDENT: Final[int] = 2
PRESEAL_RECEIPT_JSON_INDENT: Final[int] = 2
INTENT_MODEL_JSON_INDENT: Final[int] = 2
# Indentation of the release pipeline's state file and of the facts it prints.
PIPELINE_STATE_JSON_INDENT: Final[int] = 2
# Largest seal, receipt, recipe, manifest or report JSON a model verifier reads whole.
METADATA_JSON_LIMIT_BYTES: Final[int] = BYTES_PER_MEBIBYTE
# Largest context or prefix model artifact JSON the context-action verifier reads.
MODEL_ARTIFACT_JSON_LIMIT_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Largest full sequence-test report the context-action verifier reads.
FULL_REPORT_JSON_LIMIT_BYTES: Final = 64 * BYTES_PER_MEBIBYTE
# Largest intent strict report, toolchain file or preseal receipt the strict-report verifier reads;
# the rejection-receipt writer reads the strict report under the same bound.
INTENT_STRICT_REPORT_LIMIT_BYTES: Final[int] = 8 * BYTES_PER_MEBIBYTE
# Largest orthotactic seal, receipt or report JSON the ortho verifiers read whole.
ORTHO_METADATA_JSON_LIMIT_BYTES: Final = 4 * BYTES_PER_MEBIBYTE
# Largest other file (artifact, manifest, registry, report, evaluator) the rejection-receipt writer
# reads and hashes.
INTENT_REJECTION_INPUT_LIMIT_BYTES: Final[int] = 64 * BYTES_PER_MEBIBYTE
# A rejection receipt copies report values this many levels deep, and lists of at most this many
# items; larger values are summarised.
RECEIPT_COMPACT_MAX_DEPTH: Final[int] = 3
RECEIPT_COMPACT_MAX_INLINE_ITEMS: Final[int] = 16
# Indentation of the immutable rejection receipt written into model/intent_v1.
RECEIPT_JSON_INDENT: Final[int] = 2
# Largest member of the downloaded CC0 archive, and largest decompressed frozen phrase snapshot,
# the context-v2 phrase corpus reads.
CC0_ARCHIVE_MEMBER_MAX_BYTES: Final = 64 * BYTES_PER_MEBIBYTE
CC0_PHRASE_SOURCE_MAX_BYTES: Final = 32 * BYTES_PER_MEBIBYTE
# Largest decompressed frozen lexical evidence of context-v2, and the fields of one of its rows:
# baseline decision, source known, target known, score delta.
CONTEXT_LEXICAL_EVIDENCE_MAX_BYTES: Final = 64 * BYTES_PER_MEBIBYTE
CONTEXT_LEXICAL_EVIDENCE_ROW_FIELDS: Final = 4
# Indentation of the frozen corpus and dictionary receipts of context-v2 and ortho-v2; their bytes
# are pinned by the seals.
FROZEN_CORPUS_RECEIPT_JSON_INDENT: Final[int] = 2

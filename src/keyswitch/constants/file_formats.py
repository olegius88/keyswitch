"""JSON indentation, schema and format versions, size caps, hash and prefix lengths."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE

# Indentation of the settings and learning files the application keeps for the user.
USER_DATA_JSON_INDENT: Final = 2
# Decimal places a correction's confidence keeps in the history file.
HISTORY_CONFIDENCE_DECIMALS: Final = 2
IDENTIFIER_LEXICON_MAX_BYTES: Final = 4 * BYTES_PER_MEBIBYTE
IDENTIFIER_LEXICON_MAX_ENTRIES: Final = 200000
# Hex characters of a SHA-256 kept in a readable model or resource version string
# (context-v3-<hash>, prefix-v2-<hash>, intent-v1-<hash>, identifiers-<hash>).
VERSION_HASH_CHARACTERS: Final[int] = 12
# The text after the ":" separator from str.partition(":"), which always returns a 3-tuple (before,
# separator, after).
PARTITION_AFTER_SEPARATOR_INDEX: Final = 2
# schema_version of the learning file.
LEARNING_STORE_SCHEMA_VERSION: Final = 2
LEXICON_SUPPLEMENT_MAX_BYTES: Final = 8 * BYTES_PER_MEBIBYTE
# Largest prefix model artifact loaded.
MAX_PREFIX_MODEL_BYTES: Final = 2 * BYTES_PER_MEBIBYTE
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
# Largest KSLM container (.ksm); mirrors MAX_CONTAINER_BYTES of the frozen
# src/keyswitch/intent_model.py.
KSLM_MAX_CONTAINER_BYTES: Final[int] = 14 * BYTES_PER_MEBIBYTE
# Limits of a KSLM container's parts; mirror the frozen src/keyswitch/intent_model.py.
KSLM_MAX_MANIFEST_BYTES: Final[int] = BYTES_PER_MEBIBYTE
KSLM_MAX_PAYLOAD_BYTES: Final[int] = 12 * BYTES_PER_MEBIBYTE
KSLM_MAX_FINGERPRINTS: Final[int] = 1 << 20
# Mirrors src/keyswitch/intent_model.py (PENDING_RESEAL): schema 4, a minimal embedded-manifest JSON
# object, and int16 weight / uint64 fingerprint records.
KSLM_SCHEMA_VERSION: Final[int] = 4
KSLM_MIN_MANIFEST_BYTES: Final[int] = 2
KSLM_WEIGHT_ENTRY_BYTES: Final[int] = 2
KSLM_FINGERPRINT_ENTRY_BYTES: Final[int] = 8
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

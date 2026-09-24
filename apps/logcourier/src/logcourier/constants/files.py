"""File modes, sizes and formats."""

from __future__ import annotations

from typing import Final

SELECTION_HASH_PREFIX_CHARACTERS: Final = 20
# initial cost estimate, ahead of any fragment's own overhead
ZIP_MANIFEST_OVERHEAD_BYTES: Final = 1024
ZIP_ENTRY_OVERHEAD_BYTES: Final = 512
BATCH_SCHEMA_VERSION: Final = 2
INDEX_LIMIT: Final = 2 * 1024 * 1024
# collect_all's default chunk size: enough to read a real log file in one pass.
CHUNK_BYTES: Final = 2 * 1024 * 1024
ARCHIVE_LIMIT: Final = 10 * 1024 * 1024
FINGERPRINT_WINDOW_BYTES: Final = 128
# owner-only: settings, queue and secrets never group/world readable
PRIVATE_DIRECTORY_MODE: Final = 0o700
# owner-only: individual settings and downloaded files
PRIVATE_FILE_MODE: Final = 0o600
CONFIG_MAX_BYTES: Final = 1024 * 1024
JSON_INDENT_SPACES: Final = 2
BYTES_PER_KIBIBYTE: Final = 1024
BYTES_PER_MEBIBYTE: Final = BYTES_PER_KIBIBYTE * BYTES_PER_KIBIBYTE
# Telegram states its file limits in decimal megabytes.
BYTES_PER_MEGABYTE: Final = 1_000_000
HEADER_BYTES: Final = 256
PROBE_BYTES: Final = 1024 * 1024
# ord("0")
ASCII_ZERO: Final = 48
# ord("9")
ASCII_NINE: Final = 57

"""Hash lengths used by tests."""

from __future__ import annotations

from typing import Final

# A git commit hash's hex length, fixture-only (real or deliberately wrong).
GIT_COMMIT_SHA1_HEX_CHARACTERS: Final = 40
GENERATED_COMMAND_NAME_HEX_CHARACTERS: Final = 12
# Prefix length checked against the probe source, shorter than a full digest.
TRUNCATED_DIGEST_PROBE_CHARACTERS: Final = 16
CRC32_UNSIGNED_MASK: Final = 0xFFFFFFFF
# Expected FNV-1a 64-bit hashes and signed feature buckets of fixed strings.
FNV1A64_OF_A: Final = 0xAF63DC4C8601EC8C
FNV1A64_OF_FOOBAR: Final = 0x85944171F73967E8
FNV1A64_OF_PRIVET: Final = 0x1BD8A912173E871F
SIGNED_HASH_BUCKET_OF_A: Final = 140
SIGNED_HASH_BUCKET_OF_KSLM: Final = 136
MEMBERSHIP_HASH_CHAR_GROUP0_ORDER2: Final = 0xFB09B815328338F3
MEMBERSHIP_HASH_CHAR_GROUP1_ORDER5: Final = 0x5D034BC6D9921B20
SHA256_DIGEST_BYTES: Final = 32
INTENT_HASH_SEED_XOR_PERTURBATION: Final = 0x1234
INTENT_MEMBERSHIP_SEED_XOR_PERTURBATION: Final = 0x5678
PINNED_SOURCE_SHA256_PREFIX_CHARACTERS: Final = 16

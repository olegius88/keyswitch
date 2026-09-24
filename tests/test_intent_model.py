"""Exhaustive tests for the deterministic KSLM schema-4/feature-5 runtime."""

from __future__ import annotations

import gc
import hashlib
import json
import math
import os
import sys
import tempfile
import threading
import unittest
import zlib
from array import array
from collections.abc import Callable, Collection, Iterator, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import Mock, mock_open, patch

from keyswitch import intent_model as im
from keyswitch.intent_model import (
    CorrectionTrigger,
    FeatureVector,
    IntentModelFormatError,
    IntentModelInput,
    IntentModelStatus,
    LayoutDirection,
    LinearNgramModel,
    PlattParameters,
    ThresholdLogits,
    TRIGGERS,
    clear_model_cache,
    encode_model,
    extract_features,
    fnv1a64,
    normalize_token,
    signed_feature_hash,
    stable_sigmoid,
    write_model,
)
from keyswitch.language_model import WordScore
from fixture_values.clock import (
    INTENT_LOADER_SHORT_POLL_SECONDS,
    INTENT_LOADER_WAIT_TIMEOUT_SECONDS,
    MTIME_BUMP_NANOSECONDS,
)
from fixture_values.corpora import (
    INTENT_EVIDENCE_TARGET_FREQUENCY,
    INTENT_HUGE_FREQUENCY,
    INTENT_IMPLAUSIBLE_FREQUENCY_MAGNITUDE,
    INTENT_SCORER_ONLY_SOURCE_FREQUENCY,
    INTENT_SCORER_ONLY_TARGET_FREQUENCY,
)
from fixture_values.counts import (
    INTENT_DETERMINISTIC_WEIGHT_MODULUS,
    INTENT_DETERMINISTIC_WEIGHT_OFFSET,
    INTENT_DISCOVERY_EXPECTED_RELOAD_CALLS,
    INTENT_DISCOVERY_SECOND_LOAD_CALL_COUNT,
    INTENT_DISCOVERY_THIRD_LOAD_CALL_COUNT,
    INTENT_EXPECTED_FINGERPRINT_COUNT,
    INTENT_GOLDEN_FEATURE_VALUE_COUNT,
    INTENT_GOLDEN_FINGERPRINT_COUNT,
    INTENT_LOADER_EXPECTED_DECODE_CALLS,
    INTENT_LOADER_THREAD_POOL_WORKERS,
    INTENT_LONG_TOKEN_CHARACTERS,
    INTENT_MATRIX_DIRECTION_ALTERNATION_MODULUS,
    INTENT_MATRIX_TOKEN_LENGTHS,
    INTENT_MEDIUM_TOKEN_CHARACTERS,
    INTENT_METADATA_EXAMPLE_COUNT,
    INTENT_METADATA_NESTING_DEPTH_OVER_LIMIT,
    INTENT_RUNTIME_MATRIX_WEIGHT_MODULUS,
    INTENT_RUNTIME_MATRIX_WEIGHT_OFFSET,
    INTENT_SHORT_TOKEN_CHARACTERS,
    INTENT_TINY_FINGERPRINT_CAP,
    INTENT_TINY_PAYLOAD_CAP_BYTES,
    JSON_NESTING_OVERFLOW_DEPTH,
    MIDPOINT_DIVISOR,
)
from fixture_values.hashes import (
    CRC32_UNSIGNED_MASK,
    FNV1A64_OF_A,
    FNV1A64_OF_FOOBAR,
    FNV1A64_OF_PRIVET,
    MEMBERSHIP_HASH_CHAR_GROUP0_ORDER2,
    MEMBERSHIP_HASH_CHAR_GROUP1_ORDER5,
    SHA256_DIGEST_BYTES,
    SIGNED_HASH_BUCKET_OF_A,
    SIGNED_HASH_BUCKET_OF_KSLM,
)
from fixture_values.keys import (
    INTENT_EXTREME_GROUP,
    INTENT_FEATURE_ARBITRARY_CONTEXT_GROUP,
    INTENT_GOLDEN_CONTEXT_GROUP,
    INTENT_MATRIX_UNKNOWN_CONTEXT_GROUP,
    INTENT_MAX_PLAUSIBLE_CONTEXT_GROUP,
)
from fixture_values.models import (
    EXPECTED_INTENT_MINIMUM_RUNTIME_TOKEN_LENGTH,
    EXPECTED_KSLM_MAX_CONTAINER_MEBIBYTES,
    EXPECTED_KSLM_MAX_FINGERPRINTS_LOG2,
    EXPECTED_KSLM_MAX_PAYLOAD_MEBIBYTES,
    INCOMPLETE_INTENT_NGRAM_ORDERS,
    INT16_MAX,
    INTENT_GC_TEST_FINGERPRINTS,
    INTENT_IMMUTABLE_WRITE_PROBE,
    INTENT_LARGE_TEST_DIMENSION,
    INTENT_LOADER_FIXTURE_FINGERPRINT,
    INTENT_MATRIX_DIMENSION,
    INTENT_MINI_TEST_DIMENSION,
    INTENT_MODEL_VERSION_MAX_CHARACTERS,
    INTENT_MUTATION_PROBE_VALUE,
    INTENT_OVERSIZED_METADATA_INT,
    INTENT_SAMPLE_FINGERPRINT,
    INTENT_SAMPLE_FINGERPRINTS,
    INTENT_SAMPLE_NEGATIVE_WEIGHT,
    INTENT_SECOND_SAMPLE_FINGERPRINT,
    INTENT_SMALL_TEST_DIMENSION,
    INTENT_TEST_DIMENSION,
    INTENT_WRONG_DIMENSION_FOR_PAYLOAD,
    INTENT_WRONG_FEATURE_VERSION_FLOAT,
    INTENT_WRONG_FINGERPRINT_COUNT_FOR_PAYLOAD,
    INTENT_WRONG_SCHEMA_FLOAT,
    INVALID_INTENT_DIMENSION,
    KSLM_HEADER_CRC_INDEX,
    KSLM_HEADER_DIGEST_INDEX,
    KSLM_HEADER_MANIFEST_LENGTH_INDEX,
    KSLM_HEADER_PAYLOAD_LENGTH_INDEX,
    UINT64_MAX,
    UINT64_OVERFLOW,
)
from fixture_values.platform import (
    EXPECTED_INTENT_MODEL_FILE_MODE,
    FAKE_FILE_DESCRIPTOR,
    FILE_PERMISSION_BITS_MASK,
    WRONG_ARRAY_ITEM_SIZE,
)
from fixture_values.scores import (
    INTENT_ARBITRARY_THRESHOLD_LOGIT,
    INTENT_BIAS_MAGNITUDE,
    INTENT_DETERMINISTIC_WEIGHT_SCALE,
    INTENT_DIRECTIONAL_PLATT_BIAS,
    INTENT_EVIDENCE_SOURCE_NGRAM_SCORE,
    INTENT_EVIDENCE_SOURCE_WORD_VALUE,
    INTENT_EVIDENCE_TARGET_GRAM_RATIO,
    INTENT_EVIDENCE_TARGET_WORD_VALUE,
    INTENT_EXACT_BOUNDARY_PLATT_SCALE,
    INTENT_GOLDEN_CONTEXT_DELTA,
    INTENT_MATRIX_CONTEXT_DELTAS,
    INTENT_METADATA_LOSS,
    INTENT_RUNTIME_MATRIX_BIAS,
    INTENT_RUNTIME_MATRIX_CONTEXT_DELTAS,
    INTENT_RUNTIME_MATRIX_PLATT_BIAS,
    INTENT_RUNTIME_MATRIX_PLATT_SCALE,
    INTENT_RUNTIME_MATRIX_WEIGHT_SCALE,
    INTENT_SATURATED_BIAS,
    INTENT_SATURATED_LOGIT,
    INTENT_SCORER_ONLY_HIGH_RATIO,
    INTENT_SCORER_ONLY_LOW_RATIO,
    INTENT_SCORER_ONLY_MAGNITUDE,
    INTENT_SCORER_ONLY_NGRAM_MAGNITUDE,
    INTENT_SCORER_ONLY_TARGET_RAW_NGRAM_SCORE,
    INTENT_SOURCE_CHARACTER_GRAM_RATIO,
    INTENT_SOURCE_CHARACTER_INVALID_RATIO,
    INTENT_SOURCE_CHARACTER_RAW_NGRAM_SCORE,
    INTENT_STEEP_PLATT_SCALE,
    INTENT_TARGET_CHARACTER_GRAM_RATIO,
    INTENT_TARGET_CHARACTER_INVALID_RATIO,
    INTENT_TARGET_CHARACTER_NGRAM_SCORE,
    INTENT_TARGET_CHARACTER_RAW_NGRAM_SCORE,
    INTENT_TEST_BIAS,
    INTENT_TEST_VETO_THRESHOLD,
    INTENT_THRESHOLD_CROSSING_PLATT_SCALE,
    INTENT_THRESHOLD_LOGITS,
    INTENT_THRESHOLD_WRITE_PROBE,
    SATURATING_LOGIT_MAGNITUDE,
    SIGMOID_OF_ZERO,
)
from keyswitch.constants.file_formats import (
    KSLM_FINGERPRINT_ENTRY_BYTES,
    KSLM_MAX_CONTAINER_BYTES,
    KSLM_WEIGHT_ENTRY_BYTES,
    SHA256_HEX_CHARACTERS,
)
from keyswitch.constants.units import BYTES_PER_MEBIBYTE


def platt_calibration(
    scale: float = 1.0,
    bias: float = 0.0,
) -> dict[LayoutDirection, PlattParameters]:
    return {
        "0>1": PlattParameters(scale, bias),
        "1>0": PlattParameters(scale, bias),
    }


def directional_threshold_logits(
    values: Mapping[CorrectionTrigger, float] = INTENT_THRESHOLD_LOGITS,
) -> dict[CorrectionTrigger, dict[LayoutDirection, float]]:
    return {
        trigger: {
            direction: value for direction in ("0>1", "1>0")
        }
        for trigger, value in values.items()
    }


def word_score(
    value: float = 0.0,
    *,
    known: bool = False,
    frequency: int = 0,
    gram_ratio: float = 0.0,
    exact: bool = False,
    spell_known: bool = False,
    ngram_score: float = 0.0,
    invalid_ratio: float = 1.0,
    raw_ngram_score: float | None = None,
) -> WordScore:
    return WordScore(
        value,
        known,
        frequency,
        gram_ratio,
        exact,
        spell_known,
        ngram_score,
        invalid_ratio,
        ngram_score if raw_ngram_score is None else raw_ngram_score,
    )


def evidence(
    *,
    original: str = "ghbdtn",
    alternative: str = "привет",
    trigger: CorrectionTrigger = "space",
    source_group: int = 0,
    target_group: int = 1,
    source_score: WordScore | None = None,
    target_score: WordScore | None = None,
    context_delta: float = 0.0,
    context_group: int | None = None,
) -> IntentModelInput:
    return IntentModelInput(
        original,
        alternative,
        source_group,
        target_group,
        trigger,
        source_score or word_score(INTENT_EVIDENCE_SOURCE_WORD_VALUE, ngram_score=INTENT_EVIDENCE_SOURCE_NGRAM_SCORE),
        target_score
        or word_score(
            INTENT_EVIDENCE_TARGET_WORD_VALUE,
            known=True,
            frequency=INTENT_EVIDENCE_TARGET_FREQUENCY,
            gram_ratio=INTENT_EVIDENCE_TARGET_GRAM_RATIO,
            exact=True,
            ngram_score=1.0,
            invalid_ratio=0.0,
        ),
        context_delta,
        context_group,
    )


def model_bytes(
    *,
    dimension: int = INTENT_TEST_DIMENSION,
    weights: list[float] | None = None,
    fingerprints: set[int] | None = None,
    threshold_logits: Mapping[CorrectionTrigger, float] = INTENT_THRESHOLD_LOGITS,
    veto_threshold: float = INTENT_TEST_VETO_THRESHOLD,
    bias: float = INTENT_TEST_BIAS,
    platt_scale: float = 1.0,
    platt_bias: float = 0.0,
    directional_platt: Mapping[LayoutDirection, PlattParameters] | None = None,
    version: str = "intent-test-1",
    metadata: Mapping[str, object] | None = None,
) -> bytes:
    supported = (
        fingerprints
        if fingerprints is not None
        else set(
            extract_features(
                evidence(),
                dimension=dimension,
            ).character_fingerprints
        )
    )
    return encode_model(
        model_version=version,
        dimension=dimension,
        weights=weights if weights is not None else [0.0] * dimension,
        supported_fingerprints=supported,
        threshold_logits=directional_threshold_logits(threshold_logits),
        veto_threshold=veto_threshold,
        bias=bias,
        platt_calibration=(
            platt_calibration(platt_scale, platt_bias)
            if directional_platt is None
            else directional_platt
        ),
        metadata=metadata,
    )


def encode_test_case(
    *,
    model_version: object = "v1",
    dimension: object = INTENT_MINI_TEST_DIMENSION,
    weights: object = None,
    supported_fingerprints: object = None,
    threshold_logits: object = INTENT_THRESHOLD_LOGITS,
    veto_threshold: object = 0.0,
    bias: object = 0.0,
    platt_scale: object = 1.0,
    platt_bias: object = 0.0,
    fnv_seed: object = im.DEFAULT_FNV_SEED,
    membership_seed: object = im.DEFAULT_MEMBERSHIP_FNV_SEED,
    ngram_orders: object = im.NGRAM_ORDERS,
    metadata: object = None,
) -> bytes:
    """Pass deliberately malformed runtime values without weakening mypy."""

    actual_weights = [0.0] * INTENT_MINI_TEST_DIMENSION if weights is None else weights
    actual_support = (
        set() if supported_fingerprints is None else supported_fingerprints
    )
    return encode_model(
        model_version=cast(str, model_version),
        dimension=cast(int, dimension),
        weights=cast(Sequence[float], actual_weights),
        supported_fingerprints=cast(Collection[int], actual_support),
        threshold_logits=cast(
            ThresholdLogits,
            directional_threshold_logits(
                cast(Mapping[CorrectionTrigger, float], threshold_logits)
            ),
        ),
        veto_threshold=cast(float, veto_threshold),
        bias=cast(float, bias),
        platt_calibration=cast(
            Mapping[LayoutDirection, PlattParameters],
            {
                "0>1": PlattParameters(
                    cast(float, platt_scale),
                    cast(float, platt_bias),
                ),
                "1>0": PlattParameters(
                    cast(float, platt_scale),
                    cast(float, platt_bias),
                ),
            },
        ),
        fnv_seed=cast(int, fnv_seed),
        membership_seed=cast(int, membership_seed),
        ngram_orders=cast(Sequence[int], ngram_orders),
        metadata=cast(Mapping[str, object] | None, metadata),
    )


def manifest_setter(key: str, value: object) -> Callable[[dict[str, object]], None]:
    def mutate(manifest: dict[str, object]) -> None:
        manifest[key] = value

    return mutate


def manifest_remover(key: str) -> Callable[[dict[str, object]], None]:
    def mutate(manifest: dict[str, object]) -> None:
        del manifest[key]

    return mutate


def unpack_artifact(data: bytes) -> tuple[dict[str, object], bytes]:
    header = cast(
        tuple[bytes, int, int, int, int, int, bytes],
        im.HEADER.unpack_from(data),
    )
    manifest_length = header[KSLM_HEADER_MANIFEST_LENGTH_INDEX]
    start = im.HEADER.size
    manifest = cast(
        dict[str, object],
        json.loads(data[start : start + manifest_length].decode("utf-8")),
    )
    return manifest, data[start + manifest_length :]


def repack_artifact(
    data: bytes,
    *,
    mutate: Callable[[dict[str, object]], None] | None = None,
    payload: bytes | None = None,
    manifest_bytes: bytes | None = None,
) -> bytes:
    manifest, original_payload = unpack_artifact(data)
    if mutate is not None:
        mutate(manifest)
    final_payload = original_payload if payload is None else payload
    if manifest_bytes is None:
        if payload is not None and "payload_sha256" in manifest:
            manifest["payload_sha256"] = hashlib.sha256(final_payload).hexdigest()
        final_manifest = im._canonical_json(manifest)
    else:
        final_manifest = manifest_bytes
    header = im.HEADER.pack(
        im.MAGIC,
        im.SCHEMA_VERSION,
        0,
        len(final_manifest),
        len(final_payload),
        zlib.crc32(final_payload) & CRC32_UNSIGNED_MASK,
        hashlib.sha256(final_manifest).digest(),
    )
    return header + final_manifest + final_payload


class StatusAndPrimitiveTests(unittest.TestCase):

    def test_hard_size_caps_are_exact_and_internally_consistent(self) -> None:
        self.assertEqual(im.MINIMUM_RUNTIME_TOKEN_LENGTH, EXPECTED_INTENT_MINIMUM_RUNTIME_TOKEN_LENGTH)
        self.assertEqual(im.MAX_SUPPORTED_FINGERPRINTS, 1 << EXPECTED_KSLM_MAX_FINGERPRINTS_LOG2)
        self.assertEqual(im.MAX_PAYLOAD_BYTES, EXPECTED_KSLM_MAX_PAYLOAD_MEBIBYTES * BYTES_PER_MEBIBYTE)
        self.assertEqual(im.MAX_MANIFEST_BYTES, 1 * BYTES_PER_MEBIBYTE)
        self.assertEqual(im.MAX_CONTAINER_BYTES, EXPECTED_KSLM_MAX_CONTAINER_MEBIBYTES * BYTES_PER_MEBIBYTE)
        self.assertEqual(
            (im.MAX_DIMENSION * KSLM_WEIGHT_ENTRY_BYTES)
            + (im.MAX_SUPPORTED_FINGERPRINTS * KSLM_FINGERPRINT_ENTRY_BYTES),
            im.MAX_PAYLOAD_BYTES,
        )
        self.assertGreaterEqual(
            im.MAX_CONTAINER_BYTES,
            im.HEADER.size
            + im.MAX_MANIFEST_BYTES
            + im.MAX_PAYLOAD_BYTES,
        )

    def test_status_summary_and_dictionary_are_diagnostics_safe(self) -> None:
        path = Path("/tmp/model.ksm")
        status = IntentModelStatus(True, path, "v1", "a" * SHA256_HEX_CHARACTERS, None)
        self.assertEqual(status.summary, "v1 · sha256:aaaaaaaaaaaa")
        self.assertEqual(
            status.as_dict(),
            {
                "available": True,
                "path": str(path),
                "version": "v1",
                "checksum": "a" * SHA256_HEX_CHARACTERS,
                "error": None,
                "summary": "v1 · sha256:aaaaaaaaaaaa",
            },
        )
        self.assertEqual(IntentModelStatus(True, None, None, None, None).summary, "unknown")
        self.assertEqual(
            IntentModelStatus(False, None, None, None, "broken").summary,
            "недоступна: broken",
        )
        self.assertIn(
            "файл модели",
            IntentModelStatus(False, None, None, None, None).summary,
        )

    def test_unicode_normalization_and_raw_limit(self) -> None:
        self.assertEqual(normalize_token("A\u030A"), "å")
        self.assertEqual(normalize_token("Straße"), "strasse")
        self.assertEqual(normalize_token("A" * im.RAW_TOKEN_LIMIT + "B"), "a" * im.RAW_TOKEN_LIMIT)

    def test_fnv_golden_vectors_and_signed_buckets(self) -> None:
        self.assertEqual(fnv1a64(""), im.DEFAULT_FNV_SEED)
        self.assertEqual(fnv1a64("a"), FNV1A64_OF_A)
        self.assertEqual(fnv1a64("foobar"), FNV1A64_OF_FOOBAR)
        self.assertEqual(fnv1a64("привет"), FNV1A64_OF_PRIVET)
        self.assertEqual(signed_feature_hash("a", INTENT_TEST_DIMENSION), (SIGNED_HASH_BUCKET_OF_A, -1))
        self.assertEqual(signed_feature_hash("KSLM", INTENT_TEST_DIMENSION), (SIGNED_HASH_BUCKET_OF_KSLM, 1))
        self.assertEqual(
            fnv1a64("char:g0:n2:^a", im.DEFAULT_MEMBERSHIP_FNV_SEED),
            MEMBERSHIP_HASH_CHAR_GROUP0_ORDER2,
        )
        self.assertEqual(
            fnv1a64("char:g1:n5:вет$", im.DEFAULT_MEMBERSHIP_FNV_SEED),
            MEMBERSHIP_HASH_CHAR_GROUP1_ORDER5,
        )
        self.assertNotEqual(
            fnv1a64("char:g0:n2:^a", im.DEFAULT_MEMBERSHIP_FNV_SEED),
            fnv1a64("char:g0:n2:^a", im.DEFAULT_FNV_SEED),
        )
        for invalid in (-1, UINT64_OVERFLOW, True):
            with self.assertRaises(ValueError):
                fnv1a64("x", invalid)
        for invalid_dimension in (0, INVALID_INTENT_DIMENSION, im.MAX_DIMENSION + 1, True):
            with self.assertRaises(ValueError):
                signed_feature_hash("x", invalid_dimension)

    def test_stable_sigmoid_extremes_and_nan(self) -> None:
        self.assertEqual(stable_sigmoid(0.0), SIGMOID_OF_ZERO)
        self.assertEqual(stable_sigmoid(SATURATING_LOGIT_MAGNITUDE), 1.0)
        self.assertEqual(stable_sigmoid(-SATURATING_LOGIT_MAGNITUDE), 0.0)
        self.assertEqual(stable_sigmoid(math.inf), 1.0)
        self.assertEqual(stable_sigmoid(-math.inf), 0.0)
        with self.assertRaises(ValueError):
            stable_sigmoid(math.nan)


class FeatureExtractionTests(unittest.TestCase):
    GOLDEN_FEATURE_HASH = "c3062140658b4b743391d85d649b1530adeaa8b891cb554d25cf524c3ec0c01c"
    GOLDEN_MATRIX_HASH = "8e6a2c648f3ed046a0b6cbdbdba1c2181d545dfa62b6bfcff1050a4e8ec4969a"

    def test_character_features_are_count_normalized_and_antisymmetric(self) -> None:
        left: dict[str, float] = {}
        right: dict[str, float] = {}

        def add_left(name: str, value: float, *, character: bool = False) -> None:
            self.assertTrue(character)
            left[name] = left.get(name, 0.0) + value

        def add_right(name: str, value: float, *, character: bool = False) -> None:
            self.assertTrue(character)
            right[name] = right.get(name, 0.0) + value

        im._add_character_features("aaaa", 0, -1.0, im.NGRAM_ORDERS, add_left)
        im._add_character_features("bbbb", 1, 1.0, im.NGRAM_ORDERS, add_left)
        im._add_character_features("bbbb", 1, -1.0, im.NGRAM_ORDERS, add_right)
        im._add_character_features("aaaa", 0, 1.0, im.NGRAM_ORDERS, add_right)
        self.assertEqual(set(left), set(right))
        for name, value in left.items():
            self.assertAlmostEqual(value, -right[name])
        self.assertGreater(abs(left["char:g0:n2:aa"]), abs(left["char:g0:n2:^a"]))

    def test_feature_vector_is_deterministic_bounded_and_sparse(self) -> None:
        item = evidence(
            original="G" * im.RAW_TOKEN_LIMIT + "ignored",
            alternative="П" * im.RAW_TOKEN_LIMIT + "лишнее",
            source_group=-INTENT_EXTREME_GROUP,
            target_group=INTENT_EXTREME_GROUP,
            source_score=word_score(
                -math.inf,
                frequency=-INTENT_IMPLAUSIBLE_FREQUENCY_MAGNITUDE,
                gram_ratio=math.nan,
                ngram_score=-math.inf,
                invalid_ratio=math.inf,
            ),
            target_score=word_score(
                math.inf,
                frequency=INTENT_HUGE_FREQUENCY,
                gram_ratio=math.inf,
                ngram_score=math.inf,
                invalid_ratio=-math.inf,
            ),
            context_delta=math.inf,
            context_group=INTENT_FEATURE_ARBITRARY_CONTEXT_GROUP,
        )
        first = extract_features(item, dimension=INTENT_TEST_DIMENSION)
        second = extract_features(item, dimension=INTENT_TEST_DIMENSION)
        self.assertEqual(first, second)
        self.assertIsInstance(first, FeatureVector)
        self.assertTrue(first.values)
        self.assertTrue(first.character_fingerprints)
        self.assertTrue(all(0 <= bucket < INTENT_TEST_DIMENSION for bucket, _value in first.values))
        self.assertTrue(all(math.isfinite(value) for _bucket, value in first.values))

    def test_feature_categories_cover_lengths_directions_triggers_and_collisions(
        self,
    ) -> None:
        cases = (
            evidence(original="", alternative=""),
            evidence(original="x", alternative="y"),
            evidence(original="abcde", alternative="qwert"),
            evidence(
                original="abcde",
                alternative="qwert",
                source_group=1,
                target_group=0,
            ),
            evidence(original="a" * INTENT_SHORT_TOKEN_CHARACTERS, alternative="b" * INTENT_SHORT_TOKEN_CHARACTERS),
            evidence(original="a" * INTENT_MEDIUM_TOKEN_CHARACTERS, alternative="b" * INTENT_MEDIUM_TOKEN_CHARACTERS),
            evidence(original="a" * INTENT_LONG_TOKEN_CHARACTERS, alternative="b" * INTENT_LONG_TOKEN_CHARACTERS, trigger="pause"),
            evidence(trigger="punctuation"),
        )
        vectors = [extract_features(item, dimension=INTENT_SMALL_TEST_DIMENSION) for item in cases]
        self.assertEqual(len(set(vectors)), len(vectors))
        collided = extract_features(evidence(), dimension=1)
        self.assertLessEqual(len(collided.values), 1)

    def test_classifier_projection_ignores_every_language_model_field(self) -> None:
        source_character = word_score(
            INTENT_EVIDENCE_SOURCE_WORD_VALUE,
            gram_ratio=INTENT_SOURCE_CHARACTER_GRAM_RATIO,
            ngram_score=INTENT_EVIDENCE_SOURCE_NGRAM_SCORE,
            invalid_ratio=INTENT_SOURCE_CHARACTER_INVALID_RATIO,
            raw_ngram_score=INTENT_SOURCE_CHARACTER_RAW_NGRAM_SCORE,
        )
        target_character = word_score(
            1.0,
            gram_ratio=INTENT_TARGET_CHARACTER_GRAM_RATIO,
            ngram_score=INTENT_TARGET_CHARACTER_NGRAM_SCORE,
            invalid_ratio=INTENT_TARGET_CHARACTER_INVALID_RATIO,
            raw_ngram_score=INTENT_TARGET_CHARACTER_RAW_NGRAM_SCORE,
        )
        baseline = extract_features(
            evidence(
                source_score=source_character,
                target_score=target_character,
            ),
            dimension=INTENT_LARGE_TEST_DIMENSION,
        )
        scorer_only_change = extract_features(
            evidence(
                source_score=word_score(
                    INTENT_SCORER_ONLY_MAGNITUDE,
                    known=True,
                    frequency=INTENT_SCORER_ONLY_SOURCE_FREQUENCY,
                    gram_ratio=INTENT_SCORER_ONLY_HIGH_RATIO,
                    exact=True,
                    spell_known=True,
                    ngram_score=INTENT_SCORER_ONLY_NGRAM_MAGNITUDE,
                    invalid_ratio=INTENT_SCORER_ONLY_LOW_RATIO,
                    raw_ngram_score=INTENT_SCORER_ONLY_NGRAM_MAGNITUDE,
                ),
                target_score=word_score(
                    -INTENT_SCORER_ONLY_MAGNITUDE,
                    known=True,
                    frequency=INTENT_SCORER_ONLY_TARGET_FREQUENCY,
                    gram_ratio=INTENT_SCORER_ONLY_LOW_RATIO,
                    exact=True,
                    spell_known=True,
                    ngram_score=-INTENT_SCORER_ONLY_NGRAM_MAGNITUDE,
                    invalid_ratio=INTENT_SCORER_ONLY_HIGH_RATIO,
                    raw_ngram_score=INTENT_SCORER_ONLY_TARGET_RAW_NGRAM_SCORE,
                ),
            ),
            dimension=INTENT_LARGE_TEST_DIMENSION,
        )
        self.assertEqual(baseline, scorer_only_change)
        changed_token = extract_features(
            evidence(
                original="qwerty",
                source_score=source_character,
                target_score=target_character,
            ),
            dimension=INTENT_LARGE_TEST_DIMENSION,
        )
        self.assertNotEqual(baseline, changed_token)

    def test_feature_v5_golden_contract_and_hash_separation(self) -> None:
        item = evidence(
            original="ghbdtn",
            alternative="привет",
            context_delta=INTENT_GOLDEN_CONTEXT_DELTA,
            context_group=INTENT_GOLDEN_CONTEXT_GROUP,
        )
        vector = extract_features(item, dimension=INTENT_LARGE_TEST_DIMENSION)
        canonical = {
            "values": [
                [bucket, value.hex()]
                for bucket, value in vector.values
            ],
            "fingerprints": [
                f"{fingerprint:016x}"
                for fingerprint in sorted(vector.character_fingerprints)
            ],
        }
        encoded = json.dumps(
            canonical,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        ).encode("ascii")
        self.assertEqual(
            hashlib.sha256(encoded).hexdigest(),
            self.GOLDEN_FEATURE_HASH,
        )
        self.assertEqual(len(vector.values), INTENT_GOLDEN_FEATURE_VALUE_COUNT)
        self.assertEqual(len(vector.character_fingerprints), INTENT_GOLDEN_FINGERPRINT_COUNT)

        changed_feature_hash = extract_features(
            item,
            dimension=INTENT_LARGE_TEST_DIMENSION,
            hash_seed=im.DEFAULT_FNV_SEED ^ 1,
        )
        self.assertNotEqual(vector.values, changed_feature_hash.values)
        self.assertEqual(
            vector.character_fingerprints,
            changed_feature_hash.character_fingerprints,
        )

        changed_membership_hash = extract_features(
            item,
            dimension=INTENT_LARGE_TEST_DIMENSION,
            membership_seed=im.DEFAULT_MEMBERSHIP_FNV_SEED ^ 1,
        )
        self.assertEqual(vector.values, changed_membership_hash.values)
        self.assertNotEqual(
            vector.character_fingerprints,
            changed_membership_hash.character_fingerprints,
        )

        matrix: list[dict[str, object]] = []
        lengths = INTENT_MATRIX_TOKEN_LENGTHS
        deltas = INTENT_MATRIX_CONTEXT_DELTAS
        for index, (length, delta) in enumerate(
            zip(lengths, deltas, strict=True)
        ):
            source_group, target_group = (
                (0, 1) if index % INTENT_MATRIX_DIRECTION_ALTERNATION_MODULUS == 0 else (1, 0)
            )
            context_groups = (None, source_group, target_group, INTENT_MATRIX_UNKNOWN_CONTEXT_GROUP)
            matrix_vector = extract_features(
                evidence(
                    original=("a" if source_group == 0 else "б") * length,
                    alternative=("в" if target_group == 1 else "c") * length,
                    trigger=TRIGGERS[index % len(TRIGGERS)],
                    source_group=source_group,
                    target_group=target_group,
                    context_delta=delta,
                    context_group=context_groups[index % len(context_groups)],
                ),
                dimension=INTENT_MATRIX_DIMENSION,
            )
            matrix.append(
                {
                    "values": [
                        [bucket, value.hex()]
                        for bucket, value in matrix_vector.values
                    ],
                    "fingerprints": [
                        f"{fingerprint:016x}"
                        for fingerprint in sorted(
                            matrix_vector.character_fingerprints
                        )
                    ],
                }
            )
        matrix_bytes = json.dumps(
            matrix,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        ).encode("ascii")
        self.assertEqual(
            hashlib.sha256(matrix_bytes).hexdigest(),
            self.GOLDEN_MATRIX_HASH,
        )

    def test_invalid_feature_contract_is_rejected(self) -> None:
        invalid_trigger = evidence()
        object.__setattr__(invalid_trigger, "trigger", cast(CorrectionTrigger, "timer"))
        with self.assertRaises(ValueError):
            extract_features(invalid_trigger, dimension=INTENT_SMALL_TEST_DIMENSION)
        with self.assertRaises(ValueError):
            extract_features(evidence(), dimension=INTENT_SMALL_TEST_DIMENSION, hash_seed=-1)
        with self.assertRaises(ValueError):
            extract_features(evidence(), dimension=INTENT_SMALL_TEST_DIMENSION, membership_seed=-1)
        with self.assertRaises(ValueError):
            extract_features(
                evidence(),
                dimension=INTENT_SMALL_TEST_DIMENSION,
                membership_seed=im.DEFAULT_FNV_SEED,
            )
        with self.assertRaises(ValueError):
            extract_features(evidence(), dimension=INTENT_SMALL_TEST_DIMENSION, ngram_orders=INCOMPLETE_INTENT_NGRAM_ORDERS)
        with self.assertRaises(ValueError):
            extract_features(evidence(), dimension=INTENT_SMALL_TEST_DIMENSION, ngram_orders=(*INCOMPLETE_INTENT_NGRAM_ORDERS, True))


class EncodingAndPredictionTests(unittest.TestCase):

    def tearDown(self) -> None:
        clear_model_cache()

    def test_feature_v5_is_exactly_context_invariant_across_runtime_matrix(
        self,
    ) -> None:
        dimension = INTENT_LARGE_TEST_DIMENSION
        model = im._decode_container(
            model_bytes(
                dimension=dimension,
                weights=[
                    ((index % INTENT_RUNTIME_MATRIX_WEIGHT_MODULUS) - INTENT_RUNTIME_MATRIX_WEIGHT_OFFSET)
                    / INTENT_RUNTIME_MATRIX_WEIGHT_SCALE
                    for index in range(dimension)
                ],
                bias=INTENT_RUNTIME_MATRIX_BIAS,
                platt_scale=INTENT_RUNTIME_MATRIX_PLATT_SCALE,
                platt_bias=INTENT_RUNTIME_MATRIX_PLATT_BIAS,
            ),
            None,
        )
        context_deltas = INTENT_RUNTIME_MATRIX_CONTEXT_DELTAS
        for source_group, target_group in ((0, 1), (1, 0)):
            for trigger in TRIGGERS:
                baseline_input = evidence(
                    original="ghbdtn" if source_group == 0 else "руддщ",
                    alternative="привет" if target_group == 1 else "hello",
                    source_group=source_group,
                    target_group=target_group,
                    trigger=trigger,
                )
                baseline_features = extract_features(
                    baseline_input,
                    dimension=dimension,
                )
                baseline_prediction = model.predict(baseline_input)
                context_groups = (
                    None,
                    source_group,
                    target_group,
                    -1,
                    INTENT_MAX_PLAUSIBLE_CONTEXT_GROUP,
                )
                for context_group in context_groups:
                    for context_delta in context_deltas:
                        with self.subTest(
                            trigger=trigger,
                            direction=(source_group, target_group),
                            context_group=context_group,
                            context_delta=context_delta,
                        ):
                            contextual_input = replace(
                                baseline_input,
                                context_delta=context_delta,
                                context_group=context_group,
                            )
                            self.assertEqual(
                                extract_features(
                                    contextual_input,
                                    dimension=dimension,
                                ),
                                baseline_features,
                            )
                            contextual_prediction = model.predict(
                                contextual_input
                            )
                            self.assertEqual(
                                contextual_prediction.logit,
                                baseline_prediction.logit,
                            )
                            self.assertEqual(
                                contextual_prediction.probability,
                                baseline_prediction.probability,
                            )
                            self.assertEqual(
                                contextual_prediction.should_switch,
                                baseline_prediction.should_switch,
                            )
                            self.assertEqual(
                                contextual_prediction,
                                baseline_prediction,
                            )

    def test_deterministic_roundtrip_metadata_and_trigger_thresholds(self) -> None:
        weights = [
            ((index % INTENT_DETERMINISTIC_WEIGHT_MODULUS) - INTENT_DETERMINISTIC_WEIGHT_OFFSET)
            / INTENT_DETERMINISTIC_WEIGHT_SCALE
            for index in range(INTENT_TEST_DIMENSION)
        ]
        metadata: dict[str, object] = {
            "training": {"examples": INTENT_METADATA_EXAMPLE_COUNT, "loss": INTENT_METADATA_LOSS},
            "tags": ["deterministic", True, None],
        }
        first = model_bytes(
            weights=weights,
            fingerprints={0, INTENT_SAMPLE_FINGERPRINT, UINT64_MAX},
            metadata=metadata,
        )
        second = model_bytes(
            weights=weights,
            fingerprints={UINT64_MAX, INTENT_SAMPLE_FINGERPRINT, 0},
            metadata=metadata,
        )
        self.assertEqual(first, second)
        manifest, payload = unpack_artifact(first)
        self.assertEqual(manifest["schema"], im.SCHEMA_VERSION)
        self.assertEqual(manifest["feature_version"], im.FEATURE_VERSION)
        self.assertEqual(
            manifest["membership_algorithm"],
            im.MEMBERSHIP_ALGORITHM,
        )
        self.assertEqual(
            manifest["membership_seed"],
            im.DEFAULT_MEMBERSHIP_FNV_SEED,
        )
        self.assertEqual(manifest["supported_fingerprint_count"], INTENT_EXPECTED_FINGERPRINT_COUNT)
        self.assertEqual(
            im._decode_uint64_little_endian(payload[INTENT_TEST_DIMENSION * KSLM_WEIGHT_ENTRY_BYTES :]).tolist(),
            [0, INTENT_SAMPLE_FINGERPRINT, UINT64_MAX],
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.ksm"
            path.write_bytes(first)
            with patch.object(
                im,
                "_normalize_supported_fingerprints",
                side_effect=AssertionError(
                    "a decoded canonical payload must not be resorted"
                ),
            ):
                model = LinearNgramModel.load(path)
            self.assertEqual(model.source_path, path)
            self.assertEqual(model.model_version, "intent-test-1")
            self.assertEqual(model.veto_threshold, INTENT_TEST_VETO_THRESHOLD)
            self.assertEqual(
                model.membership_seed,
                im.DEFAULT_MEMBERSHIP_FNV_SEED,
            )
            self.assertEqual(model.checksum, hashlib.sha256(first).hexdigest())
            snapshot = model.metadata
            self.assertEqual(snapshot, metadata)
            cast(dict[str, object], snapshot["training"])["examples"] = 0
            self.assertEqual(cast(dict[str, object], model.metadata["training"])["examples"], INTENT_METADATA_EXAMPLE_COUNT)

            space = model.predict(evidence(trigger="space"))
            pause = model.predict(evidence(trigger="pause"))
            self.assertEqual(
                space.threshold,
                stable_sigmoid(INTENT_THRESHOLD_LOGITS["space"]),
            )
            self.assertEqual(
                pause.threshold,
                stable_sigmoid(INTENT_THRESHOLD_LOGITS["pause"]),
            )
            self.assertEqual(space.model_version, "intent-test-1")
            self.assertGreaterEqual(space.coverage, 0.0)
            self.assertLessEqual(space.coverage, 1.0)

    def test_prediction_calibration_switch_and_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.ksm"
            path.write_bytes(model_bytes(bias=1.0, platt_scale=INTENT_STEEP_PLATT_SCALE, platt_bias=-1.0))
            model = LinearNgramModel.load(path)
            prediction = model.predict(evidence(trigger="space"))
            self.assertEqual(prediction.logit, 1.0)
            self.assertAlmostEqual(prediction.probability, stable_sigmoid(1.0))
            self.assertTrue(prediction.should_switch)
            self.assertEqual(prediction.coverage, 1.0)

            zero_thresholds: dict[CorrectionTrigger, float] = {
                trigger: 0.0 for trigger in TRIGGERS
            }
            path.write_bytes(
                model_bytes(
                    bias=INTENT_BIAS_MAGNITUDE,
                    platt_scale=INTENT_THRESHOLD_CROSSING_PLATT_SCALE,
                    platt_bias=-1.0,
                    threshold_logits=zero_thresholds,
                )
            )
            raw_above_but_calibrated_below = LinearNgramModel.load(path).predict(
                evidence(trigger="space")
            )
            self.assertGreater(raw_above_but_calibrated_below.logit, 0.0)
            self.assertLess(
                (INTENT_THRESHOLD_CROSSING_PLATT_SCALE * raw_above_but_calibrated_below.logit) - 1.0,
                0.0,
            )
            self.assertFalse(raw_above_but_calibrated_below.should_switch)

            path.write_bytes(
                model_bytes(
                    bias=-INTENT_BIAS_MAGNITUDE,
                    platt_scale=INTENT_THRESHOLD_CROSSING_PLATT_SCALE,
                    platt_bias=1.0,
                    threshold_logits=zero_thresholds,
                )
            )
            raw_below_but_calibrated_above = LinearNgramModel.load(path).predict(
                evidence(trigger="space")
            )
            self.assertLess(raw_below_but_calibrated_above.logit, 0.0)
            self.assertGreater(
                (INTENT_THRESHOLD_CROSSING_PLATT_SCALE * raw_below_but_calibrated_above.logit) + 1.0,
                0.0,
            )
            self.assertTrue(raw_below_but_calibrated_above.should_switch)

            path.write_bytes(
                model_bytes(
                    bias=INTENT_BIAS_MAGNITUDE,
                    platt_scale=INTENT_EXACT_BOUNDARY_PLATT_SCALE,
                    platt_bias=-1.0,
                    threshold_logits=zero_thresholds,
                )
            )
            exact_boundary = LinearNgramModel.load(path).predict(
                evidence(trigger="space")
            )
            self.assertEqual((INTENT_EXACT_BOUNDARY_PLATT_SCALE * exact_boundary.logit) - 1.0, 0.0)
            self.assertTrue(exact_boundary.should_switch)

            path.write_bytes(model_bytes(fingerprints=set(), bias=0.0))
            unsupported = LinearNgramModel.load(path).predict(evidence(trigger="pause"))
            self.assertEqual(unsupported.coverage, 0.0)
            self.assertFalse(unsupported.should_switch)

            with patch(
                "keyswitch.intent_model.extract_features",
                return_value=FeatureVector((), frozenset()),
            ):
                no_characters = model.predict(evidence())
            self.assertEqual(no_characters.coverage, 0.0)

            saturated_logits: dict[CorrectionTrigger, float] = {
                trigger: INTENT_SATURATED_LOGIT for trigger in TRIGGERS
            }
            path.write_bytes(
                model_bytes(
                    bias=INTENT_SATURATED_BIAS,
                    threshold_logits=saturated_logits,
                )
            )
            saturated = LinearNgramModel.load(path).predict(evidence())
            self.assertEqual(saturated.probability, 1.0)
            self.assertEqual(saturated.threshold, 1.0)
            self.assertFalse(saturated.should_switch)

    def test_prediction_uses_the_exact_layout_direction_calibration(self) -> None:
        thresholds: dict[CorrectionTrigger, float] = {
            trigger: 0.0 for trigger in TRIGGERS
        }
        calibration: dict[LayoutDirection, PlattParameters] = {
            "0>1": PlattParameters(1.0, INTENT_DIRECTIONAL_PLATT_BIAS),
            "1>0": PlattParameters(INTENT_EXACT_BOUNDARY_PLATT_SCALE, -INTENT_DIRECTIONAL_PLATT_BIAS),
        }
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "directional.ksm"
            path.write_bytes(
                model_bytes(
                    bias=0.0,
                    threshold_logits=thresholds,
                    directional_platt=calibration,
                )
            )
            model = LinearNgramModel.load(path)
            forward = model.predict(
                evidence(source_group=0, target_group=1)
            )
            reverse = model.predict(
                evidence(source_group=1, target_group=0)
            )
            self.assertEqual(forward.logit, reverse.logit)
            self.assertEqual(forward.probability, stable_sigmoid(INTENT_DIRECTIONAL_PLATT_BIAS))
            self.assertEqual(reverse.probability, stable_sigmoid(-INTENT_DIRECTIONAL_PLATT_BIAS))
            self.assertTrue(forward.should_switch)
            self.assertFalse(reverse.should_switch)
            self.assertEqual(model.platt_calibration, calibration)
            with self.assertRaises(TypeError):
                model.platt_calibration["0>1"] = PlattParameters(1.0, 0.0)  # type: ignore[index]
            with self.assertRaisesRegex(ValueError, "EN/RU"):
                model.predict(evidence(source_group=0, target_group=0))

    def test_constructor_defensively_copies_mutable_arrays(self) -> None:
        def construct(
            *,
            weights_value: array[int] | None = None,
            fingerprints_value: array[int] | None = None,
            weight_scale: float = 1.0,
            platt_scale: float = 1.0,
            payload_sha256: str = "1" * SHA256_HEX_CHARACTERS,
            checksum: str = "0" * SHA256_HEX_CHARACTERS,
        ) -> LinearNgramModel:
            return LinearNgramModel(
                dimension=INTENT_TEST_DIMENSION,
                weights=(
                    array("h", [1] * INTENT_TEST_DIMENSION)
                    if weights_value is None
                    else weights_value
                ),
                supported_fingerprints=(
                    array("Q", INTENT_SAMPLE_FINGERPRINTS)
                    if fingerprints_value is None
                    else fingerprints_value
                ),
                weight_scale=weight_scale,
                bias=0.0,
                platt_calibration=platt_calibration(platt_scale, 0.0),
                threshold_logits=directional_threshold_logits(),
                veto_threshold=-1.0,
                model_version="immutable",
                fnv_seed=im.DEFAULT_FNV_SEED,
                membership_seed=im.DEFAULT_MEMBERSHIP_FNV_SEED,
                ngram_orders=im.NGRAM_ORDERS,
                payload_sha256=payload_sha256,
                checksum=checksum,
                source_path=None,
                metadata={},
            )

        weights = array("h", [1] * INTENT_TEST_DIMENSION)
        fingerprints = array("Q", INTENT_SAMPLE_FINGERPRINTS)
        model = construct(
            weights_value=weights,
            fingerprints_value=fingerprints,
        )
        weights[0] = INTENT_MUTATION_PROBE_VALUE
        fingerprints[0] = INTENT_MUTATION_PROBE_VALUE
        self.assertEqual(model._weights[0], 1)
        self.assertEqual(model._supported_fingerprints.tolist(), list(INTENT_SAMPLE_FINGERPRINTS))
        self.assertIsInstance(model._weights.obj, bytes)
        self.assertIsInstance(model._supported_fingerprints.obj, bytes)
        with self.assertRaisesRegex(AttributeError, "immutable"):
            setattr(model, "bias", INTENT_BIAS_MAGNITUDE)
        with self.assertRaises(TypeError):
            model._weights[0] = INTENT_IMMUTABLE_WRITE_PROBE
        with self.assertRaises(TypeError):
            cast(bytearray, model._weights.obj)[0] = 0
        with self.assertRaises(TypeError):
            cast(bytearray, model._supported_fingerprints.obj)[0] = 0
        with self.assertRaises(TypeError):
            model.thresholds["space"] = INTENT_THRESHOLD_WRITE_PROBE  # type: ignore[index]
        with self.assertRaises(TypeError):
            model.threshold_logits["space"] = INTENT_THRESHOLD_WRITE_PROBE  # type: ignore[index]
        with self.assertRaises(TypeError):
            model.threshold_logits["space"]["0>1"] = INTENT_THRESHOLD_WRITE_PROBE  # type: ignore[index]
        with self.assertRaises(TypeError):
            model.thresholds["space"]["0>1"] = INTENT_THRESHOLD_WRITE_PROBE  # type: ignore[index]

        invalid_cases = (
            {"weights_value": array("i", [1] * INTENT_TEST_DIMENSION)},
            {"weights_value": array("h", [1] * (INTENT_TEST_DIMENSION - 1))},
            {"fingerprints_value": array("I", INTENT_SAMPLE_FINGERPRINTS)},
            {"weight_scale": 0.0},
            {"platt_scale": 0.0},
            {"payload_sha256": ""},
            {"payload_sha256": "g" * SHA256_HEX_CHARACTERS},
            {"checksum": ""},
            {"checksum": "g" * SHA256_HEX_CHARACTERS},
        )
        for arguments in invalid_cases:
            with self.assertRaises(ValueError):
                construct(**arguments)
        with patch.object(im, "MAX_SUPPORTED_FINGERPRINTS", INTENT_TINY_FINGERPRINT_CAP):
            with self.assertRaisesRegex(ValueError, "fingerprint count"):
                construct(fingerprints_value=array("Q", INTENT_SAMPLE_FINGERPRINTS))

    def test_coverage_uses_exact_feature_membership_not_weight_buckets(self) -> None:
        known = evidence(original="a", alternative="б")
        out_of_domain = evidence(original="z", alternative="я")
        known_features = extract_features(known, dimension=1)
        out_of_domain_features = extract_features(out_of_domain, dimension=1)
        self.assertEqual({bucket for bucket, _value in known_features.values}, {0})
        self.assertEqual(
            {bucket for bucket, _value in out_of_domain_features.values},
            {0},
        )
        shared_fingerprints = (
            known_features.character_fingerprints
            & out_of_domain_features.character_fingerprints
        )
        self.assertTrue(shared_fingerprints)
        expected_out_of_domain_coverage = len(shared_fingerprints) / len(
            out_of_domain_features.character_fingerprints
        )
        self.assertGreater(expected_out_of_domain_coverage, 0.0)
        self.assertLess(expected_out_of_domain_coverage, 1.0)

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "membership.ksm"
            path.write_bytes(
                model_bytes(
                    dimension=1,
                    fingerprints=set(known_features.character_fingerprints),
                )
            )
            model = LinearNgramModel.load(path)
            self.assertEqual(model.predict(known).coverage, 1.0)
            self.assertEqual(
                model.predict(out_of_domain).coverage,
                expected_out_of_domain_coverage,
            )
            ordered = sorted(known_features.character_fingerprints)
            self.assertTrue(model._is_supported(ordered[0]))
            self.assertTrue(model._is_supported(ordered[len(ordered) // MIDPOINT_DIVISOR]))
            self.assertTrue(model._is_supported(ordered[-1]))
            missing_inside = next(
                candidate
                for candidate in range(ordered[0] + 1, ordered[1])
                if candidate not in known_features.character_fingerprints
            )
            self.assertFalse(model._is_supported(missing_inside))
            self.assertFalse(model._is_supported(UINT64_MAX))

            path.write_bytes(
                model_bytes(
                    dimension=1,
                    fingerprints={ordered[len(ordered) // MIDPOINT_DIVISOR]},
                )
            )
            partial = LinearNgramModel.load(path).predict(known)
            self.assertAlmostEqual(
                partial.coverage,
                1.0 / len(known_features.character_fingerprints),
            )

    def test_atomic_writer_creates_valid_reloadable_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "nested" / "model.ksm"
            model = write_model(
                path,
                model_version="atomic-v1",
                dimension=INTENT_SMALL_TEST_DIMENSION,
                weights=[0.0] * INTENT_SMALL_TEST_DIMENSION,
                supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT},
                threshold_logits=directional_threshold_logits(),
                veto_threshold=-1.0,
                metadata={"source": "test"},
            )
            self.assertEqual(model.model_version, "atomic-v1")
            if os.name == "posix":
                self.assertEqual(path.stat().st_mode & FILE_PERMISSION_BITS_MASK, EXPECTED_INTENT_MODEL_FILE_MODE)
            self.assertFalse(list(path.parent.glob("*.tmp")))
            self.assertEqual(LinearNgramModel.load(path).metadata, {"source": "test"})

    def test_writer_validation_rejects_invalid_arguments(self) -> None:
        invalid_dimensions = (0, INVALID_INTENT_DIMENSION, im.MAX_DIMENSION + 1, True)
        for dimension in invalid_dimensions:
            with self.assertRaises(ValueError):
                encode_test_case(dimension=dimension)
        with self.assertRaises(ValueError):
            encode_test_case(weights=[0.0])

        class UnderreportedWeights(list[float]):
            def __len__(self) -> int:
                return INTENT_MINI_TEST_DIMENSION

        with self.assertRaises(ValueError):
            encode_test_case(weights=UnderreportedWeights([0.0] * (INTENT_MINI_TEST_DIMENSION + 1)))

        class OverreportedWeights(list[float]):
            def __len__(self) -> int:
                return INTENT_MINI_TEST_DIMENSION

        with self.assertRaises(ValueError):
            encode_test_case(weights=OverreportedWeights([0.0] * (INTENT_MINI_TEST_DIMENSION - 1)))

        class WeightBomb:
            def __len__(self) -> int:
                return INTENT_MINI_TEST_DIMENSION

            def __iter__(self) -> Iterator[float]:
                yield from (0.0 for _index in range(INTENT_MINI_TEST_DIMENSION + 1))
                raise AssertionError("writer read past dimension + 1")

        with self.assertRaises(ValueError):
            encode_test_case(weights=WeightBomb())
        for invalid_weight in (math.nan, math.inf, im._MAX_MODEL_FLOAT + 1, True, "bad"):
            weights: list[object] = [0.0] * INTENT_MINI_TEST_DIMENSION
            weights[0] = invalid_weight
            with self.assertRaises(ValueError):
                encode_test_case(weights=weights)
        for invalid_support in (
            {-1},
            {UINT64_OVERFLOW},
            {True},
            cast(set[int], {"bad"}),
            [1, 1],
        ):
            with self.assertRaises(ValueError):
                encode_test_case(supported_fingerprints=invalid_support)
        for invalid_version in ("", "x" * (INTENT_MODEL_VERSION_MAX_CHARACTERS + 1), "line\nbreak", 1):
            with self.assertRaises(ValueError):
                encode_test_case(model_version=invalid_version)
        for invalid_seed in (-1, UINT64_OVERFLOW, True):
            with self.assertRaises(ValueError):
                encode_test_case(fnv_seed=invalid_seed)
            with self.assertRaises(ValueError):
                encode_test_case(membership_seed=invalid_seed)
        with self.assertRaises(ValueError):
            encode_test_case(membership_seed=im.DEFAULT_FNV_SEED)
        with patch.object(im, "MAX_SUPPORTED_FINGERPRINTS", 1):
            with self.assertRaises(ValueError):
                encode_test_case(supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT})

            second_fingerprint = INTENT_SECOND_SAMPLE_FINGERPRINT

            class UnderreportedFingerprints(Collection[int]):
                def __contains__(self, value: object) -> bool:
                    return value in (1, second_fingerprint)

                def __iter__(self) -> Iterator[int]:
                    return iter((1, second_fingerprint))

                def __len__(self) -> int:
                    return 0

            with self.assertRaises(ValueError):
                encode_test_case(
                    supported_fingerprints=UnderreportedFingerprints()
                )

            class FingerprintBomb(Collection[int]):
                def __contains__(self, value: object) -> bool:
                    return value in (1, second_fingerprint)

                def __iter__(self) -> Iterator[int]:
                    yield 1
                    yield second_fingerprint
                    raise AssertionError(
                        "writer read past MAX_SUPPORTED_FINGERPRINTS + 1"
                    )

                def __len__(self) -> int:
                    return 0

            with self.assertRaises(ValueError):
                encode_test_case(supported_fingerprints=FingerprintBomb())
        with patch.object(im, "MAX_SUPPORTED_FINGERPRINTS", INTENT_TINY_FINGERPRINT_CAP):
            exact_fingerprint_cap = encode_test_case(
                supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT}
            )
            exact_manifest, _exact_payload = unpack_artifact(
                exact_fingerprint_cap
            )
            self.assertEqual(
                exact_manifest["supported_fingerprint_count"],
                INTENT_TINY_FINGERPRINT_CAP,
            )
            with self.assertRaises(ValueError):
                encode_test_case(supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT, INTENT_SAMPLE_FINGERPRINT})
        with patch.object(im, "MAX_PAYLOAD_BYTES", INTENT_TINY_PAYLOAD_CAP_BYTES):
            exact_payload_cap = encode_test_case(
                dimension=INTENT_MINI_TEST_DIMENSION,
                supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT},
            )
            _exact_manifest, exact_payload = unpack_artifact(
                exact_payload_cap
            )
            self.assertEqual(len(exact_payload), INTENT_TINY_PAYLOAD_CAP_BYTES)
            with self.assertRaisesRegex(
                IntentModelFormatError,
                "payload length",
            ):
                encode_test_case(
                    dimension=INTENT_MINI_TEST_DIMENSION,
                    supported_fingerprints={1, INTENT_SECOND_SAMPLE_FINGERPRINT, INTENT_SAMPLE_FINGERPRINT},
                )
        for invalid_orders in (INCOMPLETE_INTENT_NGRAM_ORDERS, (*INCOMPLETE_INTENT_NGRAM_ORDERS, True), (*INCOMPLETE_INTENT_NGRAM_ORDERS, "5")):
            with self.assertRaises(ValueError):
                encode_test_case(ngram_orders=invalid_orders)

    def test_writer_rejects_invalid_calibration_thresholds_and_metadata(self) -> None:
        threshold_logit_objects: dict[str, object] = {
            trigger: value for trigger, value in INTENT_THRESHOLD_LOGITS.items()
        }
        bad_threshold_logits: list[Mapping[str, object]] = [
            {"space": INTENT_ARBITRARY_THRESHOLD_LOGIT},
            {**threshold_logit_objects, "extra": INTENT_ARBITRARY_THRESHOLD_LOGIT},
            {**threshold_logit_objects, "space": math.nan},
            {**threshold_logit_objects, "space": True},
            {**threshold_logit_objects, "space": im._MAX_MODEL_FLOAT + 1},
        ]
        for threshold_logits in bad_threshold_logits:
            with self.assertRaises(ValueError):
                encode_test_case(threshold_logits=threshold_logits)
        for field, invalid in (
            ("bias", math.inf),
            ("platt_bias", math.nan),
            ("veto_threshold", "bad"),
            ("platt_scale", 0.0),
            ("platt_scale", -1.0),
        ):
            with self.assertRaises(ValueError):
                if field == "bias":
                    encode_test_case(bias=invalid)
                elif field == "platt_bias":
                    encode_test_case(platt_bias=invalid)
                elif field == "veto_threshold":
                    encode_test_case(veto_threshold=invalid)
                else:
                    encode_test_case(platt_scale=invalid)

        nested: object = None
        for _index in range(INTENT_METADATA_NESTING_DEPTH_OVER_LIMIT):
            nested = [nested]
        bad_metadata: tuple[Mapping[str, object], ...] = (
            {"nan": math.nan},
            {"large": INTENT_OVERSIZED_METADATA_INT},
            {"object": object()},
            {"nested": nested},
            cast(Mapping[str, object], {1: "non-string key"}),
        )
        for metadata in bad_metadata:
            with self.assertRaises(IntentModelFormatError):
                encode_test_case(metadata=metadata)
        with self.assertRaises(ValueError):
            encode_test_case(metadata={"huge": "x" * im.MAX_MANIFEST_BYTES})


class LoaderCorruptionTests(unittest.TestCase):

    def setUp(self) -> None:
        self.valid = model_bytes(dimension=INTENT_MINI_TEST_DIMENSION, fingerprints={0, INTENT_LOADER_FIXTURE_FINGERPRINT})

    def load_bytes(self, data: bytes) -> LinearNgramModel:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.ksm"
            path.write_bytes(data)
            return LinearNgramModel.load(path)

    def assert_format_error(self, data: bytes, text: str = "") -> None:
        with self.assertRaises(IntentModelFormatError) as raised:
            self.load_bytes(data)
        if text:
            self.assertIn(text, str(raised.exception))

    def replace_header(
        self,
        *,
        magic: bytes = im.MAGIC,
        schema: int = im.SCHEMA_VERSION,
        flags: int = 0,
        manifest_length: int | None = None,
        payload_length: int | None = None,
        crc: int | None = None,
        digest: bytes | None = None,
        body: bytes | None = None,
    ) -> bytes:
        header = cast(
            tuple[bytes, int, int, int, int, int, bytes],
            im.HEADER.unpack_from(self.valid),
        )
        return im.HEADER.pack(
            magic,
            schema,
            flags,
            header[KSLM_HEADER_MANIFEST_LENGTH_INDEX] if manifest_length is None else manifest_length,
            header[KSLM_HEADER_PAYLOAD_LENGTH_INDEX] if payload_length is None else payload_length,
            header[KSLM_HEADER_CRC_INDEX] if crc is None else crc,
            header[KSLM_HEADER_DIGEST_INDEX] if digest is None else digest,
        ) + (self.valid[im.HEADER.size :] if body is None else body)

    def test_header_magic_schema_flags_and_bounds(self) -> None:
        self.assert_format_error(b"short", "header")
        self.assert_format_error(self.replace_header(magic=b"NOPE"), "magic")
        self.assert_format_error(self.replace_header(schema=1), "schema")
        self.assert_format_error(self.replace_header(flags=1), "flags")
        self.assert_format_error(self.replace_header(manifest_length=1), "manifest length")
        self.assert_format_error(
            self.replace_header(manifest_length=im.MAX_MANIFEST_BYTES + 1),
            "manifest length",
        )
        self.assert_format_error(
            self.replace_header(manifest_length=im.MAX_MANIFEST_BYTES),
            "file length",
        )
        self.assert_format_error(self.replace_header(payload_length=0), "payload length")
        self.assert_format_error(
            self.replace_header(payload_length=im.MAX_PAYLOAD_BYTES + 1),
            "payload length",
        )
        self.assert_format_error(
            self.replace_header(payload_length=im.MAX_PAYLOAD_BYTES),
            "file length",
        )
        self.assert_format_error(self.valid + b"trailing", "file length")
        self.assert_format_error(self.valid[:-1], "file length")

    def test_manifest_digest_payload_crc_and_json_failures(self) -> None:
        self.assert_format_error(self.replace_header(digest=b"0" * SHA256_DIGEST_BYTES), "manifest checksum")
        self.assert_format_error(self.replace_header(crc=0), "CRC32")
        manifest, payload = unpack_artifact(self.valid)
        del manifest
        invalid_utf8 = repack_artifact(self.valid, manifest_bytes=b"\xff\xfe")
        self.assert_format_error(invalid_utf8, "UTF-8 JSON")
        invalid_json = repack_artifact(self.valid, manifest_bytes=b"{broken")
        self.assert_format_error(invalid_json, "UTF-8 JSON")
        deep_json = (
            b'{"deep":'
            + (b"[" * JSON_NESTING_OVERFLOW_DEPTH)
            + b"null"
            + (b"]" * JSON_NESTING_OVERFLOW_DEPTH)
            + b"}"
        )
        self.assert_format_error(
            repack_artifact(self.valid, manifest_bytes=deep_json),
            "nesting limit",
        )
        with patch(
            "keyswitch.intent_model.json.loads",
            side_effect=RecursionError,
        ):
            self.assert_format_error(self.valid, "UTF-8 JSON")
        array_manifest = repack_artifact(self.valid, manifest_bytes=b"[]")
        self.assert_format_error(array_manifest, "object")
        canonical, _payload = unpack_artifact(self.valid)
        noncanonical = json.dumps(canonical, ensure_ascii=False, indent=1).encode("utf-8")
        self.assert_format_error(
            repack_artifact(self.valid, manifest_bytes=noncanonical),
            "canonical",
        )
        changed = bytearray(payload)
        changed[0] ^= 1
        changed_payload = bytes(changed)
        artifact = repack_artifact(self.valid, payload=changed_payload)
        old_manifest, _old_payload = unpack_artifact(self.valid)
        old_manifest_bytes = im._canonical_json(old_manifest)
        header = im.HEADER.pack(
            im.MAGIC,
            im.SCHEMA_VERSION,
            0,
            len(old_manifest_bytes),
            len(changed_payload),
            zlib.crc32(changed_payload) & CRC32_UNSIGNED_MASK,
            hashlib.sha256(old_manifest_bytes).digest(),
        )
        self.assert_format_error(header + old_manifest_bytes + changed_payload, "SHA256")
        self.assertIsInstance(artifact, bytes)

    def test_manifest_schema_feature_hash_and_field_set(self) -> None:
        mutations: tuple[tuple[Callable[[dict[str, object]], None], str], ...] = (
            (manifest_remover("bias"), "fields"),
            (manifest_setter("extra", 1), "fields"),
            (manifest_setter("format", "OTHER"), "format/schema"),
            (manifest_setter("schema", 1), "format/schema"),
            (manifest_setter("schema", INTENT_WRONG_SCHEMA_FLOAT), "schema must be an integer"),
            (manifest_setter("feature_version", 1), "feature version"),
            (
                manifest_setter("feature_version", INTENT_WRONG_FEATURE_VERSION_FLOAT),
                "feature_version must be an integer",
            ),
            (manifest_setter("hash_algorithm", "python-hash"), "hash algorithm"),
            (
                manifest_setter("membership_algorithm", "weight-bucket-bitset"),
                "membership algorithm",
            ),
        )
        for mutate, message in mutations:
            self.assert_format_error(repack_artifact(self.valid, mutate=mutate), message)

    def test_manifest_dimension_seed_orders_version_and_float_validation(self) -> None:
        cases: tuple[tuple[str, object, str], ...] = (
            ("dimension", True, "dimension"),
            ("dimension", INVALID_INTENT_DIMENSION, "dimension"),
            ("fnv_seed", True, "fnv_seed"),
            ("fnv_seed", -1, "FNV seed"),
            ("membership_seed", True, "membership_seed"),
            ("membership_seed", -1, "FNV seed"),
            ("membership_seed", im.DEFAULT_FNV_SEED, "must differ"),
            ("supported_fingerprint_count", True, "integer"),
            ("supported_fingerprint_count", -1, "size limit"),
            (
                "supported_fingerprint_count",
                im.MAX_SUPPORTED_FINGERPRINTS + 1,
                "size limit",
            ),
            ("ngram_orders", "bad", "array"),
            ("ngram_orders", list(INCOMPLETE_INTENT_NGRAM_ORDERS), "n-gram"),
            ("ngram_orders", [*INCOMPLETE_INTENT_NGRAM_ORDERS, True], "integers"),
            ("model_version", 1, "model_version"),
            ("weight_scale", 0.0, "positive"),
            ("bias", "bad", "numeric"),
            ("veto_threshold", True, "numeric"),
        )
        for key, value, message in cases:
            self.assert_format_error(
                repack_artifact(self.valid, mutate=manifest_setter(key, value)),
                message,
            )

        for direction, field, value, message in (
            ("0>1", "scale", 0.0, "positive"),
            ("1>0", "bias", im._MAX_MODEL_FLOAT + 1, "bounded"),
        ):
            def mutate_calibration(
                manifest: dict[str, object],
                *,
                selected_direction: str = direction,
                selected_field: str = field,
                selected_value: object = value,
            ) -> None:
                calibration = cast(
                    dict[str, dict[str, object]],
                    manifest["platt_calibration"],
                )
                calibration[selected_direction][selected_field] = selected_value

            self.assert_format_error(
                repack_artifact(self.valid, mutate=mutate_calibration),
                message,
            )

        manifest, payload = unpack_artifact(self.valid)
        raw = im._canonical_json(manifest).replace(b'"bias":0.5', b'"bias":NaN')
        nan_artifact = im.HEADER.pack(
            im.MAGIC,
            im.SCHEMA_VERSION,
            0,
            len(raw),
            len(payload),
            zlib.crc32(payload) & CRC32_UNSIGNED_MASK,
            hashlib.sha256(raw).digest(),
        ) + raw + payload
        self.assert_format_error(nan_artifact, "finite")

    def test_threshold_payload_hash_shape_and_fingerprint_validation(self) -> None:
        cases: tuple[tuple[str, object, str], ...] = (
            ("threshold_logits", [], "object"),
            (
                "threshold_logits",
                {"space": INTENT_ARBITRARY_THRESHOLD_LOGIT},
                "every correction trigger",
            ),
            ("payload_sha256", 1, "lowercase hexadecimal"),
            ("payload_sha256", "A" * SHA256_HEX_CHARACTERS, "lowercase hexadecimal"),
            ("payload_sha256", "g" * SHA256_HEX_CHARACTERS, "lowercase hexadecimal"),
        )
        for key, value, message in cases:
            self.assert_format_error(
                repack_artifact(self.valid, mutate=manifest_setter(key, value)),
                message,
            )

        valid_thresholds: dict[str, object] = {
            trigger: dict(values)
            for trigger, values in directional_threshold_logits().items()
        }
        malformed_thresholds: tuple[tuple[dict[str, object], str], ...] = (
            ({**valid_thresholds, "space": INTENT_ARBITRARY_THRESHOLD_LOGIT}, "space must be an object"),
            (
                {**valid_thresholds, "space": {"0>1": INTENT_ARBITRARY_THRESHOLD_LOGIT}},
                "space must contain both directions",
            ),
        )
        for threshold_logits, message in malformed_thresholds:
            self.assert_format_error(
                repack_artifact(
                    self.valid,
                    mutate=manifest_setter("threshold_logits", threshold_logits),
                ),
                message,
            )

        calibration = {
            "0>1": {"scale": 1.0, "bias": 0.0},
            "1>0": {"scale": 1.0, "bias": 0.0},
        }
        malformed_calibrations: tuple[tuple[dict[str, object], str], ...] = (
            ({"0>1": calibration["0>1"]}, "both EN/RU directions"),
            (
                {**calibration, "0>1": {"scale": 1.0}},
                "0>1 fields do not match schema",
            ),
            ({**calibration, "0>1": 1.0}, "0>1 must be an object"),
        )
        for platt_calibration_value, message in malformed_calibrations:
            self.assert_format_error(
                repack_artifact(
                    self.valid,
                    mutate=manifest_setter(
                        "platt_calibration",
                        platt_calibration_value,
                    ),
                ),
                message,
            )
        self.assert_format_error(
            repack_artifact(self.valid, mutate=manifest_setter("dimension", INTENT_WRONG_DIMENSION_FOR_PAYLOAD)),
            "payload shape",
        )
        self.assert_format_error(
            repack_artifact(
                self.valid,
                mutate=manifest_setter("supported_fingerprint_count", INTENT_WRONG_FINGERPRINT_COUNT_FOR_PAYLOAD),
            ),
            "payload shape",
        )

        manifest, payload = unpack_artifact(self.valid)
        del manifest
        weights = payload[: INTENT_MINI_TEST_DIMENSION * KSLM_WEIGHT_ENTRY_BYTES]
        unsorted = weights + im._uint64_little_endian_bytes(array("Q", [INTENT_LOADER_FIXTURE_FINGERPRINT, 0]))
        duplicate = weights + im._uint64_little_endian_bytes(array("Q", [INTENT_LOADER_FIXTURE_FINGERPRINT, INTENT_LOADER_FIXTURE_FINGERPRINT]))
        self.assert_format_error(
            repack_artifact(self.valid, payload=unsorted),
            "sorted and unique",
        )
        self.assert_format_error(
            repack_artifact(self.valid, payload=duplicate),
            "sorted and unique",
        )

        with patch.object(im, "_decode_int16_little_endian", return_value=array("h")):
            self.assert_format_error(self.valid, "weight count")
        with patch.object(
            im,
            "_decode_uint64_little_endian",
            return_value=array("Q", [0]),
        ):
            self.assert_format_error(self.valid, "fingerprint count")

    def test_file_size_bound_and_missing_file(self) -> None:
        self.assertEqual(im.MAX_CONTAINER_BYTES, KSLM_MAX_CONTAINER_BYTES)
        opener = mock_open(read_data=b"short")
        with patch.object(Path, "open", opener):
            self.assertEqual(im._read_bounded(Path("bounded.ksm")), b"short")
        opener().read.assert_called_once_with(im.MAX_CONTAINER_BYTES + 1)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "huge.ksm"
            maximum = im.MAX_CONTAINER_BYTES
            with path.open("wb") as handle:
                handle.seek(maximum)
                handle.write(b"x")
            with self.assertRaises(IntentModelFormatError):
                LinearNgramModel.load(path)
            with self.assertRaises(FileNotFoundError):
                LinearNgramModel.load(Path(temporary) / "missing.ksm")

    def test_load_restores_cyclic_gc_after_success_and_failure(self) -> None:
        encoded = model_bytes(dimension=INTENT_MINI_TEST_DIMENSION, fingerprints=INTENT_GC_TEST_FINGERPRINTS)
        original_gc_state = gc.isenabled()
        try:
            gc.enable()
            observed_success: list[bool] = []
            real_decoder = im._decode_container

            def observing_decoder(
                data: bytes,
                source_path: Path | None,
            ) -> LinearNgramModel:
                observed_success.append(gc.isenabled())
                return real_decoder(data, source_path)

            with patch.object(im, "_read_bounded", return_value=encoded), patch.object(
                im,
                "_decode_container",
                side_effect=observing_decoder,
            ):
                LinearNgramModel.load("success.ksm")
            self.assertEqual(observed_success, [False])
            self.assertTrue(gc.isenabled())

            observed_failure: list[bool] = []

            def failing_decoder(
                _data: bytes,
                _source_path: Path | None,
            ) -> LinearNgramModel:
                observed_failure.append(gc.isenabled())
                raise IntentModelFormatError("expected failure")

            with patch.object(im, "_read_bounded", return_value=encoded), patch.object(
                im,
                "_decode_container",
                side_effect=failing_decoder,
            ):
                with self.assertRaisesRegex(IntentModelFormatError, "expected failure"):
                    LinearNgramModel.load("failure.ksm")
            self.assertEqual(observed_failure, [False])
            self.assertTrue(gc.isenabled())

            gc.disable()
            with patch.object(im, "_read_bounded", return_value=encoded), patch.object(
                im,
                "_decode_container",
                wraps=real_decoder,
            ):
                LinearNgramModel.load("already-disabled.ksm")
            self.assertFalse(gc.isenabled())
        finally:
            if original_gc_state:
                gc.enable()
            else:
                gc.disable()

    def test_concurrent_loads_serialize_the_process_global_gc_guard(self) -> None:
        encoded = model_bytes(dimension=INTENT_MINI_TEST_DIMENSION, fingerprints=INTENT_GC_TEST_FINGERPRINTS)
        first_decode_entered = threading.Event()
        second_worker_started = threading.Event()
        second_decode_entered = threading.Event()
        release_first_decode = threading.Event()
        call_lock = threading.Lock()
        decode_calls = 0
        sentinel = cast(LinearNgramModel, object())

        def controlled_decoder(
            _data: bytes,
            _source_path: Path | None,
        ) -> LinearNgramModel:
            nonlocal decode_calls
            with call_lock:
                decode_calls += 1
                call_number = decode_calls
            if call_number == 1:
                first_decode_entered.set()
                if not release_first_decode.wait(INTENT_LOADER_WAIT_TIMEOUT_SECONDS):
                    raise AssertionError("first decoder was not released")
            else:
                second_decode_entered.set()
            return sentinel

        def second_load() -> LinearNgramModel:
            second_worker_started.set()
            return LinearNgramModel.load("second.ksm")

        with patch.object(im, "_read_bounded", return_value=encoded), patch.object(
            im,
            "_decode_container",
            side_effect=controlled_decoder,
        ), ThreadPoolExecutor(max_workers=INTENT_LOADER_THREAD_POOL_WORKERS) as executor:
            first = executor.submit(LinearNgramModel.load, "first.ksm")
            self.assertTrue(first_decode_entered.wait(INTENT_LOADER_WAIT_TIMEOUT_SECONDS))
            second = executor.submit(second_load)
            self.assertTrue(second_worker_started.wait(INTENT_LOADER_WAIT_TIMEOUT_SECONDS))
            try:
                self.assertFalse(second_decode_entered.wait(INTENT_LOADER_SHORT_POLL_SECONDS))
            finally:
                release_first_decode.set()
            self.assertIs(first.result(timeout=INTENT_LOADER_WAIT_TIMEOUT_SECONDS), sentinel)
            self.assertIs(second.result(timeout=INTENT_LOADER_WAIT_TIMEOUT_SECONDS), sentinel)
        self.assertTrue(second_decode_entered.is_set())
        self.assertEqual(decode_calls, INTENT_LOADER_EXPECTED_DECODE_CALLS)


class DiscoveryCacheAndSystemBranchTests(unittest.TestCase):

    def tearDown(self) -> None:
        clear_model_cache()

    def packaged_path(self, root: Path) -> Path:
        return root / "resources" / "models" / "layout_intent_v1.ksm"

    def test_override_is_cached_and_file_change_invalidates_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            override = root / "override.ksm"
            override.write_bytes(model_bytes(version="cache-v1"))
            with patch.dict(os.environ, {"KEYSWITCH_INTENT_MODEL_PATH": str(override)}), patch.object(
                LinearNgramModel,
                "load",
                wraps=LinearNgramModel.load,
            ) as loader:
                first, first_status = LinearNgramModel.try_load_default()
                second, _second_status = LinearNgramModel.try_load_default()
                self.assertIs(first, second)
                self.assertTrue(first_status.available)
                self.assertEqual(loader.call_count, 1)
                replacement = model_bytes(version="cache-v2")
                override.write_bytes(replacement)
                status = override.stat()
                os.utime(override, ns=(status.st_atime_ns, status.st_mtime_ns + MTIME_BUMP_NANOSECONDS))
                third, _third_status = LinearNgramModel.try_load_default()
                self.assertIsNot(first, third)
                self.assertEqual(cast(LinearNgramModel, third).model_version, "cache-v2")
                self.assertEqual(loader.call_count, INTENT_DISCOVERY_SECOND_LOAD_CALL_COUNT)
                clear_model_cache()
                LinearNgramModel.try_load_default()
                self.assertEqual(loader.call_count, INTENT_DISCOVERY_THIRD_LOAD_CALL_COUNT)

    def test_invalid_override_falls_back_to_package_and_reports_failures(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fake_module = root / "intent_model.py"
            packaged = self.packaged_path(root)
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(model_bytes(version="packaged"))
            override = root / "bad.ksm"
            override.write_bytes(b"bad")
            with patch.object(im, "__file__", str(fake_module)), patch.dict(
                os.environ,
                {"KEYSWITCH_INTENT_MODEL_PATH": str(override)},
            ):
                model, status = LinearNgramModel.try_load_default()
            self.assertEqual(cast(LinearNgramModel, model).model_version, "packaged")
            self.assertEqual(cast(Path, status.path).resolve(), packaged.resolve())

            packaged.write_bytes(b"also bad")
            clear_model_cache()
            with patch.object(im, "__file__", str(fake_module)), patch.dict(
                os.environ,
                {"KEYSWITCH_INTENT_MODEL_PATH": str(override)},
            ):
                missing, failed = LinearNgramModel.try_load_default()
            self.assertIsNone(missing)
            self.assertFalse(failed.available)
            self.assertEqual(cast(Path, failed.path).resolve(), override.resolve())
            self.assertIn("bad.ksm", cast(str, failed.error))
            self.assertIn("layout_intent_v1.ksm", cast(str, failed.error))

    def test_default_path_deduplication_and_programming_errors_propagate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fake_module = root / "intent_model.py"
            packaged = self.packaged_path(root)
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(model_bytes())
            with patch.object(im, "__file__", str(fake_module)), patch.dict(
                os.environ,
                {"KEYSWITCH_INTENT_MODEL_PATH": str(packaged.resolve())},
            ), patch.object(LinearNgramModel, "load", side_effect=RuntimeError("bug")):
                with self.assertRaises(RuntimeError):
                    LinearNgramModel.try_load_default()

    def test_cache_reloads_if_file_changes_during_initial_load(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.ksm"
            first_bytes = model_bytes(version="before")
            second_bytes = model_bytes(version="after")
            path.write_bytes(first_bytes)
            real_load = LinearNgramModel.load
            calls = 0

            def changing_load(source: Path | str) -> LinearNgramModel:
                nonlocal calls
                calls += 1
                model = real_load(source)
                if calls == 1:
                    path.write_bytes(second_bytes)
                    status = path.stat()
                    os.utime(path, ns=(status.st_atime_ns, status.st_mtime_ns + MTIME_BUMP_NANOSECONDS))
                return model

            with patch.object(LinearNgramModel, "load", side_effect=changing_load):
                model = im._load_cached(path)
            self.assertEqual(calls, INTENT_DISCOVERY_EXPECTED_RELOAD_CALLS)
            self.assertEqual(model.model_version, "after")

    def test_endian_array_and_fsync_error_branches(self) -> None:
        values: array[int] = array("h", [1, -INTENT_SAMPLE_NEGATIVE_WEIGHT, INT16_MAX])
        fingerprints: array[int] = array("Q", [0, 1, UINT64_MAX])
        with patch.object(sys, "byteorder", "big"):
            encoded = im._int16_little_endian_bytes(values)
            self.assertEqual(im._decode_int16_little_endian(encoded).tolist(), values.tolist())
            encoded_fingerprints = im._uint64_little_endian_bytes(fingerprints)
            self.assertEqual(
                im._decode_uint64_little_endian(encoded_fingerprints).tolist(),
                fingerprints.tolist(),
            )

        with patch.object(os, "open", side_effect=OSError("unsupported")):
            im._fsync_directory(Path("/tmp"))
        with patch.object(os, "open", return_value=FAKE_FILE_DESCRIPTOR), patch.object(
            os,
            "fsync",
            side_effect=OSError("denied"),
        ), patch.object(os, "close") as close:
            im._fsync_directory(Path("/tmp"))
            close.assert_called_once_with(FAKE_FILE_DESCRIPTOR)

        fake_array = cast(array[int], Mock(itemsize=WRONG_ARRAY_ITEM_SIZE))
        with self.assertRaises(RuntimeError):
            im._int16_little_endian_bytes(fake_array)
        with patch.object(im, "array", return_value=fake_array):
            with self.assertRaises(IntentModelFormatError):
                im._decode_int16_little_endian(b"")
            with self.assertRaises(IntentModelFormatError):
                im._decode_uint64_little_endian(b"")
        with self.assertRaises(RuntimeError):
            im._uint64_little_endian_bytes(fake_array)
        with self.assertRaises(IntentModelFormatError):
            im._decode_uint64_little_endian(b"unaligned")

    def test_internal_json_key_guards_reject_non_string_keys(self) -> None:
        invalid = cast(dict[str, object], {1: "bad"})
        with self.assertRaises(IntentModelFormatError):
            im._as_mapping(invalid, "mapping")
        with self.assertRaises(IntentModelFormatError):
            im._validated_json_value(invalid, "value", 0)

    def test_writer_cleans_temporary_file_after_validation_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "model.ksm"
            with patch.object(
                LinearNgramModel,
                "load",
                side_effect=IntentModelFormatError("broken after write"),
            ):
                with self.assertRaises(IntentModelFormatError):
                    write_model(
                        destination,
                        model_version="v1",
                        dimension=INTENT_MINI_TEST_DIMENSION,
                        weights=[0.0] * INTENT_MINI_TEST_DIMENSION,
                        supported_fingerprints=set(),
                        threshold_logits=directional_threshold_logits(),
                        veto_threshold=0.0,
                    )
            self.assertFalse(destination.exists())
            self.assertFalse(list(destination.parent.glob("*.tmp")))

        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "model.ksm"
            with patch.object(
                LinearNgramModel,
                "load",
                side_effect=IntentModelFormatError("broken"),
            ), patch.object(Path, "unlink", side_effect=OSError("denied")):
                with self.assertRaises(IntentModelFormatError):
                    write_model(
                        destination,
                        model_version="v1",
                        dimension=INTENT_MINI_TEST_DIMENSION,
                        weights=[0.0] * INTENT_MINI_TEST_DIMENSION,
                        supported_fingerprints=set(),
                        threshold_logits=directional_threshold_logits(),
                        veto_threshold=0.0,
                    )
            self.assertFalse(destination.exists())
            self.assertTrue(list(destination.parent.glob("*.tmp")))

        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "model.ksm"
            fake = Mock(checksum="wrong")
            with patch.object(LinearNgramModel, "load", return_value=fake):
                with self.assertRaises(IntentModelFormatError):
                    write_model(
                        destination,
                        model_version="v1",
                        dimension=INTENT_MINI_TEST_DIMENSION,
                        weights=[0.0] * INTENT_MINI_TEST_DIMENSION,
                        supported_fingerprints=set(),
                        threshold_logits=directional_threshold_logits(),
                        veto_threshold=0.0,
                    )


if __name__ == "__main__":
    unittest.main()

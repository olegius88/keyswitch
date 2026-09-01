"""Deterministic, dependency-free linear intent model runtime.

KSLM is a deliberately small binary container.  It combines a canonical JSON
manifest with quantized little-endian int16 weights and sorted uint64
fingerprints of the exact character features observed during training.  The
fingerprints use an independent hash namespace: coverage therefore measures
feature membership rather than occupancy of collision-prone weight buckets.
Loading is strict and bounded so a broken optional model can never prevent
KeySwitch from falling back to its conservative rules.
"""

from __future__ import annotations

import gc
import hashlib
import json
import math
import os
import struct
import sys
import tempfile
import threading
import unicodedata
import zlib
from array import array
from bisect import bisect_left
from collections import Counter
from collections.abc import Collection, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Callable, Final, Literal, ParamSpec, Protocol, TypeVar, cast

from .language_model import WordScore


CorrectionTrigger = Literal[
    "boundary_probe",
    "pause",
    "space",
    "enter",
    "tab",
    "punctuation",
]
LayoutDirection = Literal["0>1", "1>0"]
ThresholdLogits = Mapping[
    CorrectionTrigger,
    Mapping[LayoutDirection, float],
]

TRIGGERS: Final[tuple[CorrectionTrigger, ...]] = (
    "boundary_probe",
    "pause",
    "space",
    "enter",
    "tab",
    "punctuation",
)
LAYOUT_DIRECTIONS: Final[tuple[LayoutDirection, ...]] = ("0>1", "1>0")
MAGIC: Final[bytes] = b"KSLM"
SCHEMA_VERSION: Final[int] = 4
FEATURE_VERSION: Final[int] = 5
HASH_ALGORITHM: Final[str] = "fnv1a64-signed-v1"
MEMBERSHIP_ALGORITHM: Final[str] = "fnv1a64-unsigned-v1"
DEFAULT_FNV_SEED: Final[int] = 0xCBF29CE484222325
DEFAULT_MEMBERSHIP_FNV_SEED: Final[int] = 0x9E3779B97F4A7C15
NGRAM_ORDERS: Final[tuple[int, ...]] = (1, 2, 3, 4, 5)
RAW_TOKEN_LIMIT: Final[int] = 64
MINIMUM_RUNTIME_TOKEN_LENGTH: Final[int] = 5
MAX_DIMENSION: Final[int] = 1 << 21
MAX_MANIFEST_BYTES: Final[int] = 1 * 1024 * 1024
MAX_SUPPORTED_FINGERPRINTS: Final[int] = 1 << 20
MAX_PAYLOAD_BYTES: Final[int] = 12 * 1024 * 1024
HEADER: Final[struct.Struct] = struct.Struct("<4sHHIII32s")
MAX_CONTAINER_BYTES: Final[int] = 14 * 1024 * 1024
_FNV_PRIME: Final[int] = 0x100000001B3
_UINT64_MASK: Final[int] = (1 << 64) - 1
_MAX_MODEL_FLOAT: Final[float] = 1_000_000.0
_MODEL_CACHE: dict[tuple[str, int, int, int, int], "LinearNgramModel"] = {}
_MODEL_CACHE_LOCK: Final[threading.RLock] = threading.RLock()
_MODEL_DECODE_LOCK: Final[threading.RLock] = threading.RLock()
_CANONICAL_FINGERPRINT_PAYLOAD: Final[object] = object()
_P = ParamSpec("_P")
_T = TypeVar("_T")


class IntentModelFormatError(ValueError):
    """The KSLM artifact is malformed, incompatible, or untrusted."""


@dataclass(frozen=True)
class IntentModelInput:
    """All bounded evidence needed for one layout-correction decision."""

    original: str
    alternative: str
    source_group: int
    target_group: int
    trigger: CorrectionTrigger
    source_score: WordScore
    target_score: WordScore
    context_delta: float = 0.0
    context_group: int | None = None


@dataclass(frozen=True)
class LinearPrediction:
    """Calibrated model output plus its conservative operating threshold."""

    logit: float
    probability: float
    threshold: float
    coverage: float
    should_switch: bool
    model_version: str


@dataclass(frozen=True)
class IntentModelStatus:
    """Diagnostics-safe state returned by optional-model discovery."""

    available: bool
    path: Path | None
    version: str | None
    checksum: str | None
    error: str | None

    @property
    def summary(self) -> str:
        if self.available:
            version = self.version or "unknown"
            if self.checksum:
                return f"{version} · sha256:{self.checksum[:12]}"
            return version
        return f"недоступна: {self.error or 'файл модели не найден'}"

    def as_dict(self) -> dict[str, object]:
        return {
            "available": self.available,
            "path": str(self.path) if self.path is not None else None,
            "version": self.version,
            "checksum": self.checksum,
            "error": self.error,
            "summary": self.summary,
        }


@dataclass(frozen=True)
class FeatureVector:
    """Stable sparse vector and exact-name fingerprints used for coverage."""

    values: tuple[tuple[int, float], ...]
    character_fingerprints: frozenset[int]


@dataclass(frozen=True)
class PlattParameters:
    """Immutable sigmoid calibration for one physical layout direction."""

    scale: float
    bias: float


def normalize_token(token: str) -> str:
    """Bound CPU/memory use, then apply the model's Unicode normalization."""

    bounded = token[:RAW_TOKEN_LIMIT]
    return unicodedata.normalize("NFC", unicodedata.normalize("NFC", bounded).casefold())


def fnv1a64(value: str, seed: int = DEFAULT_FNV_SEED) -> int:
    """Return a platform-independent FNV-1a hash of UTF-8 feature text."""

    if isinstance(seed, bool) or not 0 <= seed <= _UINT64_MASK:
        raise ValueError("FNV seed must be an unsigned 64-bit integer")
    result = seed
    for byte in value.encode("utf-8"):
        result ^= byte
        result = (result * _FNV_PRIME) & _UINT64_MASK
    return result


def signed_feature_hash(
    feature: str,
    dimension: int,
    seed: int = DEFAULT_FNV_SEED,
) -> tuple[int, int]:
    """Map one feature to a bucket and a deterministic collision sign."""

    _validate_dimension(dimension)
    hashed = fnv1a64(feature, seed)
    sign = -1 if hashed & (1 << 63) else 1
    return hashed & (dimension - 1), sign


def stable_sigmoid(value: float) -> float:
    """Numerically stable sigmoid, including saturated finite inputs."""

    if math.isnan(value):
        raise ValueError("sigmoid input must not be NaN")
    if value >= 0.0:
        return 1.0 / (1.0 + math.exp(-value))
    exponential = math.exp(value)
    return exponential / (1.0 + exponential)


def extract_features(
    evidence: IntentModelInput,
    *,
    dimension: int,
    hash_seed: int = DEFAULT_FNV_SEED,
    membership_seed: int = DEFAULT_MEMBERSHIP_FNV_SEED,
    ngram_orders: Sequence[int] = NGRAM_ORDERS,
) -> FeatureVector:
    """Build pairwise character and bounded dense hashed features."""

    _validate_dimension(dimension)
    _validate_seed(hash_seed)
    _validate_membership_seed(membership_seed, hash_seed)
    orders = _validate_orders(ngram_orders)
    if evidence.trigger not in TRIGGERS:
        raise ValueError(f"unsupported correction trigger: {evidence.trigger!r}")

    values: dict[int, float] = {}
    character_fingerprints: set[int] = set()

    def add(name: str, value: float, *, character: bool = False) -> None:
        if value == 0.0:
            return
        bucket, sign = signed_feature_hash(name, dimension, hash_seed)
        values[bucket] = values.get(bucket, 0.0) + (value * sign)
        if character:
            character_fingerprints.add(fnv1a64(name, membership_seed))

    original = normalize_token(evidence.original)
    alternative = normalize_token(evidence.alternative)
    active_orders = sum(
        1
        for order in orders
        if len(f"^{original}$") >= order or len(f"^{alternative}$") >= order
    )
    order_scale = 1.0 / math.sqrt(active_orders) if active_orders else 1.0
    _add_character_features(
        original,
        evidence.source_group,
        -order_scale,
        orders,
        add,
    )
    _add_character_features(
        alternative,
        evidence.target_group,
        order_scale,
        orders,
        add,
    )

    direction = f"{_group(evidence.source_group)}>{_group(evidence.target_group)}"
    length_bucket = _length_bucket(max(len(original), len(alternative)))

    add("dense:length_delta", (len(alternative) - len(original)) / float(RAW_TOKEN_LIMIT))
    add(
        f"interaction:direction:{direction}:length:{length_bucket}",
        1.0,
    )
    add(f"interaction:trigger:{evidence.trigger}:length:{length_bucket}", 1.0)

    add(f"length:{length_bucket}", 1.0)
    add(f"direction:{direction}", 1.0)
    add(f"trigger:{evidence.trigger}", 1.0)

    compact = tuple(
        (bucket, value)
        for bucket, value in sorted(values.items())
        if value != 0.0
    )
    return FeatureVector(compact, frozenset(character_fingerprints))


class LinearNgramModel:
    """Immutable KSLM v4 model with direction-calibrated inference."""

    def __init__(
        self,
        *,
        dimension: int,
        weights: array[int],
        supported_fingerprints: array[int],
        weight_scale: float,
        bias: float,
        platt_calibration: Mapping[LayoutDirection, PlattParameters],
        threshold_logits: ThresholdLogits,
        veto_threshold: float,
        model_version: str,
        fnv_seed: int,
        membership_seed: int,
        ngram_orders: tuple[int, ...],
        payload_sha256: str,
        checksum: str,
        source_path: Path | None,
        metadata: Mapping[str, object],
        _fingerprint_payload_token: object | None = None,
    ) -> None:
        _validate_dimension(dimension)
        if weights.typecode != "h" or weights.itemsize != 2:
            raise ValueError("weights must be a native signed-int16 array")
        if len(weights) != dimension:
            raise ValueError("weight count must equal model dimension")
        if supported_fingerprints.typecode != "Q" or supported_fingerprints.itemsize != 8:
            raise ValueError(
                "supported fingerprints must be a native unsigned-int64 array"
            )
        if len(supported_fingerprints) > MAX_SUPPORTED_FINGERPRINTS:
            raise ValueError(
                "supported fingerprint count exceeds KSLM size limit"
            )
        normalized_fingerprints = (
            supported_fingerprints
            if _fingerprint_payload_token is _CANONICAL_FINGERPRINT_PAYLOAD
            else _normalize_supported_fingerprints(supported_fingerprints)
        )
        parsed_weight_scale = _model_float(weight_scale, "weight_scale")
        if parsed_weight_scale <= 0.0:
            raise ValueError("weight_scale must be positive")
        parsed_platt_calibration = _validate_platt_calibration(
            {direction: value for direction, value in platt_calibration.items()}
        )
        parsed_threshold_logits = _validate_threshold_logits_object(
            {trigger: value for trigger, value in threshold_logits.items()}
        )
        _validate_seed(fnv_seed)
        _validate_membership_seed(membership_seed, fnv_seed)
        parsed_orders = _validate_orders(ngram_orders)
        parsed_version = _validate_version(model_version)
        if (
            len(payload_sha256) != 64
            or any(
                character not in "0123456789abcdef"
                for character in payload_sha256
            )
        ):
            raise ValueError("payload_sha256 must be exact lowercase SHA-256")
        if (
            len(checksum) != 64
            or any(character not in "0123456789abcdef" for character in checksum)
        ):
            raise ValueError("checksum must be exact lowercase SHA-256")
        self.dimension = dimension
        weight_storage = weights.tobytes()
        fingerprint_storage = normalized_fingerprints.tobytes()
        self._weights = memoryview(weight_storage).cast("h")
        self._supported_fingerprints = memoryview(fingerprint_storage).cast("Q")
        self.weight_scale = parsed_weight_scale
        self.bias = _model_float(bias, "bias")
        self.platt_calibration = MappingProxyType(parsed_platt_calibration)
        self.threshold_logits = MappingProxyType(
            {
                trigger: MappingProxyType(dict(values))
                for trigger, values in parsed_threshold_logits.items()
            }
        )
        self.thresholds = MappingProxyType(
            {
                trigger: MappingProxyType(
                    {
                        direction: stable_sigmoid(logit)
                        for direction, logit in values.items()
                    }
                )
                for trigger, values in parsed_threshold_logits.items()
            }
        )
        self.veto_threshold = _model_float(veto_threshold, "veto_threshold")
        self.model_version = parsed_version
        self.fnv_seed = fnv_seed
        self.membership_seed = membership_seed
        self.ngram_orders = parsed_orders
        self.payload_sha256 = payload_sha256
        self.checksum = checksum
        self.source_path = source_path
        self._metadata = _validated_json_object(metadata, "metadata")
        self._frozen = True

    def __setattr__(self, name: str, value: object) -> None:
        if getattr(self, "_frozen", False):
            raise AttributeError("LinearNgramModel is immutable")
        object.__setattr__(self, name, value)

    @property
    def metadata(self) -> dict[str, object]:
        """Return a defensive JSON-safe copy of training diagnostics."""

        return _validated_json_object(self._metadata, "metadata")

    @classmethod
    def load(cls, path: Path | str) -> LinearNgramModel:
        source_path = Path(path)
        data = _read_bounded(source_path)
        # KSLM JSON and arrays cannot contain reference cycles.  Suspending the
        # cyclic collector keeps an unrelated, large live object graph from
        # turning one model decode into an unbounded generation scan.  The lock
        # makes the process-global GC state exact for concurrent/nested loads.
        with _MODEL_DECODE_LOCK:
            restore_gc = gc.isenabled()
            if restore_gc:
                gc.disable()
            try:
                return _decode_container(data, source_path)
            finally:
                if restore_gc:
                    gc.enable()

    @classmethod
    def try_load_default(cls) -> tuple[LinearNgramModel | None, IntentModelStatus]:
        packaged = Path(__file__).resolve().parent / "resources" / "models" / "layout_intent_v1.ksm"
        override = os.environ.get("KEYSWITCH_INTENT_MODEL_PATH", "").strip()
        candidates = ([Path(override).expanduser()] if override else []) + [packaged]
        unique: list[Path] = []
        for candidate in candidates:
            if candidate not in unique:
                unique.append(candidate)

        errors: list[str] = []
        for candidate in unique:
            try:
                model = _load_cached(candidate)
            except (OSError, IntentModelFormatError, ValueError) as error:
                errors.append(f"{candidate}: {error}")
                continue
            return model, IntentModelStatus(
                True,
                candidate,
                model.model_version,
                model.checksum,
                None,
            )
        return None, IntentModelStatus(
            False,
            unique[0],
            None,
            None,
            "; ".join(errors),
        )

    def predict(self, evidence: IntentModelInput) -> LinearPrediction:
        features = extract_features(
            evidence,
            dimension=self.dimension,
            hash_seed=self.fnv_seed,
            membership_seed=self.membership_seed,
            ngram_orders=self.ngram_orders,
        )
        logit = self.bias
        for bucket, value in features.values:
            logit += self._weights[bucket] * self.weight_scale * value
        direction = layout_direction(
            evidence.source_group,
            evidence.target_group,
        )
        calibration = self.platt_calibration[direction]
        calibrated = (calibration.scale * logit) + calibration.bias
        probability = stable_sigmoid(calibrated)
        threshold_logit = self.threshold_logits[evidence.trigger][direction]
        threshold = self.thresholds[evidence.trigger][direction]
        if features.character_fingerprints:
            supported = sum(
                self._is_supported(fingerprint)
                for fingerprint in features.character_fingerprints
            )
            coverage = supported / len(features.character_fingerprints)
        else:
            coverage = 0.0
        return LinearPrediction(
            logit,
            probability,
            threshold,
            coverage,
            calibrated >= threshold_logit,
            self.model_version,
        )

    def _is_supported(self, fingerprint: int) -> bool:
        index = bisect_left(self._supported_fingerprints, fingerprint)
        return (
            index < len(self._supported_fingerprints)
            and self._supported_fingerprints[index] == fingerprint
        )


def encode_model(
    *,
    model_version: str,
    dimension: int,
    weights: Sequence[float],
    supported_fingerprints: Collection[int],
    threshold_logits: ThresholdLogits,
    veto_threshold: float,
    bias: float = 0.0,
    platt_calibration: Mapping[LayoutDirection, PlattParameters] | None = None,
    fnv_seed: int = DEFAULT_FNV_SEED,
    membership_seed: int = DEFAULT_MEMBERSHIP_FNV_SEED,
    ngram_orders: Sequence[int] = NGRAM_ORDERS,
    metadata: Mapping[str, object] | None = None,
) -> bytes:
    """Encode and self-validate deterministic KSLM v4 bytes."""

    _validate_dimension(dimension)
    _validate_seed(fnv_seed)
    _validate_membership_seed(membership_seed, fnv_seed)
    orders = _validate_orders(ngram_orders)
    version = _validate_version(model_version)
    float_weights = _normalize_weights(weights, dimension)
    scale = max((abs(value) for value in float_weights), default=0.0) / 32767.0
    if scale == 0.0:
        scale = 1.0
    quantized = array(
        "h",
        (
            max(-32767, min(32767, round(value / scale)))
            for value in float_weights
        ),
    )
    weight_bytes = _int16_little_endian_bytes(quantized)
    fingerprints = _normalize_supported_fingerprints(supported_fingerprints)
    fingerprint_bytes = _uint64_little_endian_bytes(fingerprints)
    payload = weight_bytes + fingerprint_bytes

    threshold_values: dict[str, object] = {
        trigger: value for trigger, value in threshold_logits.items()
    }
    parsed_threshold_logits = _validate_threshold_logits_object(threshold_values)
    parsed_bias = _model_float(bias, "bias")
    calibration_input = platt_calibration or {
        direction: PlattParameters(1.0, 0.0)
        for direction in LAYOUT_DIRECTIONS
    }
    parsed_platt_calibration = _validate_platt_calibration(
        {direction: value for direction, value in calibration_input.items()}
    )
    parsed_veto = _model_float(veto_threshold, "veto_threshold")
    manifest: dict[str, object] = {
        "bias": parsed_bias,
        "dimension": dimension,
        "feature_version": FEATURE_VERSION,
        "fnv_seed": fnv_seed,
        "format": MAGIC.decode("ascii"),
        "hash_algorithm": HASH_ALGORITHM,
        "membership_algorithm": MEMBERSHIP_ALGORITHM,
        "membership_seed": membership_seed,
        "model_version": version,
        "ngram_orders": list(orders),
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
        "platt_calibration": {
            direction: {
                "bias": parsed_platt_calibration[direction].bias,
                "scale": parsed_platt_calibration[direction].scale,
            }
            for direction in LAYOUT_DIRECTIONS
        },
        "schema": SCHEMA_VERSION,
        "supported_fingerprint_count": len(fingerprints),
        "threshold_logits": parsed_threshold_logits,
        "veto_threshold": parsed_veto,
        "weight_scale": scale,
    }
    if metadata is not None:
        manifest["metadata"] = _validated_json_object(metadata, "metadata")
    manifest_bytes = _canonical_json(manifest)
    if len(manifest_bytes) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds KSLM size limit")
    header = HEADER.pack(
        MAGIC,
        SCHEMA_VERSION,
        0,
        len(manifest_bytes),
        len(payload),
        zlib.crc32(payload) & 0xFFFFFFFF,
        hashlib.sha256(manifest_bytes).digest(),
    )
    encoded = header + manifest_bytes + payload
    _decode_container(encoded, None)
    return encoded


def write_model(
    path: Path | str,
    *,
    model_version: str,
    dimension: int,
    weights: Sequence[float],
    supported_fingerprints: Collection[int],
    threshold_logits: ThresholdLogits,
    veto_threshold: float,
    bias: float = 0.0,
    platt_calibration: Mapping[LayoutDirection, PlattParameters] | None = None,
    fnv_seed: int = DEFAULT_FNV_SEED,
    membership_seed: int = DEFAULT_MEMBERSHIP_FNV_SEED,
    ngram_orders: Sequence[int] = NGRAM_ORDERS,
    metadata: Mapping[str, object] | None = None,
) -> LinearNgramModel:
    """Atomically write, validate, and reload a KSLM model artifact."""

    destination = Path(path)
    encoded = encode_model(
        model_version=model_version,
        dimension=dimension,
        weights=weights,
        supported_fingerprints=supported_fingerprints,
        threshold_logits=threshold_logits,
        veto_threshold=veto_threshold,
        bias=bias,
        platt_calibration=platt_calibration,
        fnv_seed=fnv_seed,
        membership_seed=membership_seed,
        ngram_orders=ngram_orders,
        metadata=metadata,
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            prefix=f".{destination.name}.",
            suffix=".tmp",
            dir=destination.parent,
            delete=False,
        ) as handle:
            temporary_path = Path(handle.name)
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary_path, 0o644)
        validated = LinearNgramModel.load(temporary_path)
        if validated.checksum != hashlib.sha256(encoded).hexdigest():
            raise IntentModelFormatError("written model checksum mismatch")
        os.replace(temporary_path, destination)
        temporary_path = None
        _fsync_directory(destination.parent)
        return LinearNgramModel.load(destination)
    finally:
        if temporary_path is not None:
            try:
                temporary_path.unlink()
            except OSError:
                pass


def _decode_container(data: bytes, source_path: Path | None) -> LinearNgramModel:
    if len(data) < HEADER.size:
        raise IntentModelFormatError("truncated KSLM header")
    magic, schema, flags, manifest_length, payload_length, payload_crc, manifest_digest = HEADER.unpack_from(data)
    if magic != MAGIC:
        raise IntentModelFormatError("invalid KSLM magic")
    if schema != SCHEMA_VERSION:
        raise IntentModelFormatError(f"unsupported KSLM schema: {schema}")
    if flags != 0:
        raise IntentModelFormatError("unsupported KSLM header flags")
    if not 2 <= manifest_length <= MAX_MANIFEST_BYTES:
        raise IntentModelFormatError("invalid KSLM manifest length")
    if not 0 < payload_length <= MAX_PAYLOAD_BYTES:
        raise IntentModelFormatError("invalid KSLM payload length")
    expected_length = HEADER.size + manifest_length + payload_length
    if len(data) != expected_length:
        raise IntentModelFormatError("KSLM file length does not match header")

    manifest_bytes = data[HEADER.size : HEADER.size + manifest_length]
    payload = data[HEADER.size + manifest_length :]
    if hashlib.sha256(manifest_bytes).digest() != manifest_digest:
        raise IntentModelFormatError("KSLM manifest checksum mismatch")
    if (zlib.crc32(payload) & 0xFFFFFFFF) != payload_crc:
        raise IntentModelFormatError("KSLM payload CRC32 mismatch")
    try:
        decoded_json = cast(object, json.loads(manifest_bytes.decode("utf-8")))
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise IntentModelFormatError("KSLM manifest is not valid UTF-8 JSON") from error
    manifest = _validated_json_object(_as_mapping(decoded_json, "manifest"), "manifest")
    canonical = _canonical_json(manifest)
    if canonical != manifest_bytes:
        raise IntentModelFormatError("KSLM manifest is not canonical JSON")

    required = {
        "bias",
        "dimension",
        "feature_version",
        "fnv_seed",
        "format",
        "hash_algorithm",
        "membership_algorithm",
        "membership_seed",
        "model_version",
        "ngram_orders",
        "payload_sha256",
        "platt_calibration",
        "schema",
        "supported_fingerprint_count",
        "threshold_logits",
        "veto_threshold",
        "weight_scale",
    }
    allowed = required | {"metadata"}
    if set(manifest) != required and set(manifest) != allowed:
        raise IntentModelFormatError("KSLM manifest fields do not match schema")
    manifest_schema = _manifest_int(manifest["schema"], "schema")
    if manifest["format"] != "KSLM" or manifest_schema != SCHEMA_VERSION:
        raise IntentModelFormatError("KSLM manifest format/schema mismatch")
    feature_version = _manifest_int(
        manifest["feature_version"], "feature_version"
    )
    if feature_version != FEATURE_VERSION:
        raise IntentModelFormatError("unsupported KSLM feature version")
    if manifest["hash_algorithm"] != HASH_ALGORITHM:
        raise IntentModelFormatError("unsupported KSLM hash algorithm")
    if manifest["membership_algorithm"] != MEMBERSHIP_ALGORITHM:
        raise IntentModelFormatError("unsupported KSLM membership algorithm")

    dimension = _manifest_int(manifest["dimension"], "dimension")
    _format_validation(_validate_dimension, dimension)
    fnv_seed = _manifest_int(manifest["fnv_seed"], "fnv_seed")
    _format_validation(_validate_seed, fnv_seed)
    membership_seed = _manifest_int(manifest["membership_seed"], "membership_seed")
    _format_validation(_validate_membership_seed, membership_seed, fnv_seed)
    fingerprint_count = _manifest_int(
        manifest["supported_fingerprint_count"],
        "supported_fingerprint_count",
    )
    if not 0 <= fingerprint_count <= MAX_SUPPORTED_FINGERPRINTS:
        raise IntentModelFormatError(
            "supported_fingerprint_count exceeds KSLM size limit"
        )
    orders_value = _as_sequence(manifest["ngram_orders"], "ngram_orders")
    orders = _format_validation(_validate_orders, orders_value)
    version = _format_validation(_validate_version, manifest["model_version"])
    weight_scale = _manifest_float(manifest["weight_scale"], "weight_scale")
    if weight_scale <= 0.0:
        raise IntentModelFormatError("weight_scale must be positive")
    bias = _manifest_float(manifest["bias"], "bias")
    platt_mapping = _as_mapping(
        manifest["platt_calibration"],
        "platt_calibration",
    )
    platt_calibration = _format_validation(
        _validate_platt_calibration,
        platt_mapping,
    )
    veto_threshold = _manifest_float(manifest["veto_threshold"], "veto_threshold")
    threshold_mapping = _as_mapping(
        manifest["threshold_logits"],
        "threshold_logits",
    )
    threshold_logits = _format_validation(
        _validate_threshold_logits_object,
        threshold_mapping,
    )
    metadata_value = manifest.get("metadata", {})
    metadata = _validated_json_object(
        _as_mapping(metadata_value, "metadata"),
        "metadata",
    )

    payload_sha = manifest["payload_sha256"]
    if (
        not isinstance(payload_sha, str)
        or len(payload_sha) != 64
        or any(character not in "0123456789abcdef" for character in payload_sha)
    ):
        raise IntentModelFormatError("payload_sha256 must be lowercase hexadecimal")
    if hashlib.sha256(payload).hexdigest() != payload_sha:
        raise IntentModelFormatError("KSLM payload SHA256 mismatch")

    expected_payload = (dimension * 2) + (fingerprint_count * 8)
    if payload_length != expected_payload:
        raise IntentModelFormatError("KSLM payload shape does not match dimension")
    weights = _decode_int16_little_endian(payload[: dimension * 2])
    if len(weights) != dimension:
        raise IntentModelFormatError("KSLM weight count does not match dimension")
    supported_fingerprints = _decode_uint64_little_endian(payload[dimension * 2 :])
    if len(supported_fingerprints) != fingerprint_count:
        raise IntentModelFormatError(
            "KSLM supported fingerprint count does not match manifest"
        )
    if any(
        left >= right
        for left, right in zip(
            supported_fingerprints,
            supported_fingerprints[1:],
        )
    ):
        raise IntentModelFormatError(
            "KSLM supported fingerprints must be sorted and unique"
        )

    checksum = hashlib.sha256(data).hexdigest()
    return LinearNgramModel(
        dimension=dimension,
        weights=weights,
        supported_fingerprints=supported_fingerprints,
        weight_scale=weight_scale,
        bias=bias,
        platt_calibration=platt_calibration,
        threshold_logits=threshold_logits,
        veto_threshold=veto_threshold,
        model_version=version,
        fnv_seed=fnv_seed,
        membership_seed=membership_seed,
        ngram_orders=orders,
        payload_sha256=payload_sha,
        checksum=checksum,
        source_path=source_path,
        metadata=metadata,
        _fingerprint_payload_token=_CANONICAL_FINGERPRINT_PAYLOAD,
    )


def _read_bounded(path: Path) -> bytes:
    maximum = MAX_CONTAINER_BYTES
    with path.open("rb") as handle:
        data = handle.read(maximum + 1)
    if len(data) > maximum:
        raise IntentModelFormatError("KSLM file exceeds size limit")
    return data


def _add_character_features(
    token: str,
    group: int,
    polarity: float,
    orders: tuple[int, ...],
    add_feature: _FeatureAdder,
) -> None:
    bounded = f"^{token}$"
    for order in orders:
        if len(bounded) < order:
            continue
        counts = Counter(
            bounded[index : index + order]
            for index in range(len(bounded) - order + 1)
        )
        norm = math.sqrt(sum(count * count for count in counts.values()))
        prefix = f"char:g{_group(group)}:n{order}:"
        for ngram, count in counts.items():
            add_feature(prefix + ngram, polarity * count / norm, character=True)


class _FeatureAdder(Protocol):
    def __call__(self, name: str, value: float, *, character: bool = False) -> None: ...


def _length_bucket(length: int) -> str:
    if length <= 4:
        return str(length)
    if length <= 7:
        return "5-7"
    if length <= 11:
        return "8-11"
    if length <= 19:
        return "12-19"
    return "20+"


def _group(group: int) -> int:
    return max(-1, min(63, group))


def layout_direction(source_group: int, target_group: int) -> LayoutDirection:
    if source_group == 0 and target_group == 1:
        return "0>1"
    if source_group == 1 and target_group == 0:
        return "1>0"
    raise ValueError("intent model requires an EN/RU layout direction")


def _validate_dimension(dimension: int) -> None:
    if (
        isinstance(dimension, bool)
        or dimension < 1
        or dimension > MAX_DIMENSION
        or dimension & (dimension - 1)
    ):
        raise ValueError("model dimension must be a power of two up to 2^21")


def _validate_seed(seed: int) -> None:
    if isinstance(seed, bool) or not 0 <= seed <= _UINT64_MASK:
        raise ValueError("FNV seed must be an unsigned 64-bit integer")


def _validate_membership_seed(seed: int, feature_hash_seed: int) -> None:
    _validate_seed(seed)
    if seed == feature_hash_seed:
        raise ValueError("membership_seed must differ from fnv_seed")


def _validate_orders(orders: Sequence[object]) -> tuple[int, ...]:
    parsed: list[int] = []
    for order in orders:
        if isinstance(order, bool) or not isinstance(order, int):
            raise ValueError("n-gram orders must be integers")
        parsed.append(order)
    result = tuple(parsed)
    if result != NGRAM_ORDERS:
        raise ValueError("KSLM v4 requires n-gram orders 1, 2, 3, 4, 5")
    return result


def _validate_version(value: object) -> str:
    if not isinstance(value, str) or not value or len(value) > 128 or not value.isprintable():
        raise ValueError("model_version must be a short printable string")
    return value


def _model_float(value: object, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    parsed = float(value)
    if not math.isfinite(parsed) or abs(parsed) > _MAX_MODEL_FLOAT:
        raise ValueError(f"{field} must be finite and bounded")
    return parsed


def _manifest_float(value: object, field: str) -> float:
    try:
        return _model_float(value, field)
    except ValueError as error:
        raise IntentModelFormatError(str(error)) from error


def _manifest_int(value: object, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise IntentModelFormatError(f"{field} must be an integer")
    return value


def _validate_threshold_logits_object(
    values: Mapping[str, object],
) -> dict[CorrectionTrigger, dict[LayoutDirection, float]]:
    if set(values) != set(TRIGGERS):
        raise ValueError("threshold_logits must contain every correction trigger")
    result: dict[CorrectionTrigger, dict[LayoutDirection, float]] = {}
    for trigger in TRIGGERS:
        raw = values[trigger]
        if not isinstance(raw, Mapping) or not all(
            isinstance(key, str) for key in raw
        ):
            raise ValueError(f"threshold_logits.{trigger} must be an object")
        if set(raw) != set(LAYOUT_DIRECTIONS):
            raise ValueError(
                f"threshold_logits.{trigger} must contain both directions"
            )
        result[trigger] = {
            direction: _model_float(
                raw[direction],
                f"threshold_logits.{trigger}.{direction}",
            )
            for direction in LAYOUT_DIRECTIONS
        }
    return result


def _validate_platt_calibration(
    values: Mapping[str, object],
) -> dict[LayoutDirection, PlattParameters]:
    if set(values) != set(LAYOUT_DIRECTIONS):
        raise ValueError(
            "platt_calibration must contain both EN/RU directions"
        )
    result: dict[LayoutDirection, PlattParameters] = {}
    for direction in LAYOUT_DIRECTIONS:
        raw = values[direction]
        if isinstance(raw, PlattParameters):
            scale_value: object = raw.scale
            bias_value: object = raw.bias
        elif isinstance(raw, Mapping):
            if set(raw) != {"scale", "bias"} or not all(
                isinstance(key, str) for key in raw
            ):
                raise ValueError(
                    f"platt_calibration.{direction} fields do not match schema"
                )
            scale_value = raw["scale"]
            bias_value = raw["bias"]
        else:
            raise ValueError(
                f"platt_calibration.{direction} must be an object"
            )
        scale = _model_float(
            scale_value,
            f"platt_calibration.{direction}.scale",
        )
        if scale <= 0.0:
            raise ValueError(
                f"platt_calibration.{direction}.scale must be positive"
            )
        result[direction] = PlattParameters(
            scale,
            _model_float(
                bias_value,
                f"platt_calibration.{direction}.bias",
            ),
        )
    return result


def _normalize_supported_fingerprints(values: Collection[object]) -> array[int]:
    if len(values) > MAX_SUPPORTED_FINGERPRINTS:
        raise ValueError("supported fingerprint count exceeds KSLM size limit")
    parsed: list[int] = []
    for value in values:
        if (
            isinstance(value, bool)
            or not isinstance(value, int)
            or not 0 <= value <= _UINT64_MASK
        ):
            raise ValueError("supported fingerprint must be an unsigned 64-bit integer")
        parsed.append(value)
        if len(parsed) > MAX_SUPPORTED_FINGERPRINTS:
            raise ValueError("supported fingerprint count exceeds KSLM size limit")
    parsed.sort()
    if any(left == right for left, right in zip(parsed, parsed[1:])):
        raise ValueError("supported fingerprints must be unique")
    return array("Q", parsed)


def _normalize_weights(values: Sequence[object], dimension: int) -> list[float]:
    """Read exactly dimension weights even from a dishonest Sequence."""

    if len(values) != dimension:
        raise ValueError("weight count must equal model dimension")
    parsed: list[float] = []
    for value in values:
        if len(parsed) >= dimension:
            raise ValueError("weight count must equal model dimension")
        parsed.append(_model_float(value, "weight"))
    if len(parsed) != dimension:
        raise ValueError("weight count must equal model dimension")
    return parsed


def _int16_little_endian_bytes(values: array[int]) -> bytes:
    if values.itemsize != 2:
        raise RuntimeError("platform int16 array type is unavailable")
    copy = array("h", values)
    if sys.byteorder != "little":
        copy.byteswap()
    return copy.tobytes()


def _decode_int16_little_endian(data: bytes) -> array[int]:
    values: array[int] = array("h")
    if values.itemsize != 2:
        raise IntentModelFormatError("platform int16 array type is unavailable")
    values.frombytes(data)
    if sys.byteorder != "little":
        values.byteswap()
    return values


def _uint64_little_endian_bytes(values: array[int]) -> bytes:
    if values.itemsize != 8:
        raise RuntimeError("platform uint64 array type is unavailable")
    copy = array("Q", values)
    if sys.byteorder != "little":
        copy.byteswap()
    return copy.tobytes()


def _decode_uint64_little_endian(data: bytes) -> array[int]:
    values: array[int] = array("Q")
    if values.itemsize != 8:
        raise IntentModelFormatError("platform uint64 array type is unavailable")
    if len(data) % 8:
        raise IntentModelFormatError("KSLM fingerprint payload is not uint64-aligned")
    values.frombytes(data)
    if sys.byteorder != "little":
        values.byteswap()
    return values


def _canonical_json(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    ).encode("utf-8")


def _as_mapping(value: object, field: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise IntentModelFormatError(f"{field} must be an object")
    raw = cast(dict[object, object], value)
    if any(not isinstance(key, str) for key in raw):
        raise IntentModelFormatError(f"{field} keys must be strings")
    return cast(dict[str, object], raw)


def _as_sequence(value: object, field: str) -> Sequence[object]:
    if not isinstance(value, list):
        raise IntentModelFormatError(f"{field} must be an array")
    return cast(list[object], value)


def _validated_json_object(value: Mapping[str, object], field: str) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise IntentModelFormatError(f"{field} keys must be strings")
        result[key] = _validated_json_value(item, f"{field}.{key}", 0)
    return result


def _validated_json_value(value: object, field: str, depth: int) -> object:
    if depth > 16:
        raise IntentModelFormatError(f"{field} exceeds JSON nesting limit")
    if value is None or isinstance(value, (str, bool)):
        return value
    if isinstance(value, int):
        if value < -(1 << 63) or value > _UINT64_MASK:
            raise IntentModelFormatError(f"{field} integer is out of range")
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise IntentModelFormatError(f"{field} must be finite")
        return value
    if isinstance(value, list):
        return [
            _validated_json_value(item, f"{field}[]", depth + 1)
            for item in cast(list[object], value)
        ]
    if isinstance(value, dict):
        mapping = cast(dict[object, object], value)
        if any(not isinstance(key, str) for key in mapping):
            raise IntentModelFormatError(f"{field} keys must be strings")
        return {
            cast(str, key): _validated_json_value(item, f"{field}.{key}", depth + 1)
            for key, item in mapping.items()
        }
    raise IntentModelFormatError(f"{field} contains a non-JSON value")


def _format_validation(function: Callable[_P, _T], *args: _P.args, **kwargs: _P.kwargs) -> _T:
    try:
        return function(*args, **kwargs)
    except ValueError as error:
        raise IntentModelFormatError(str(error)) from error


def clear_model_cache() -> None:
    """Clear successful artifact cache (primarily for controlled reload/tests)."""

    with _MODEL_CACHE_LOCK:
        _MODEL_CACHE.clear()


def _load_cached(path: Path) -> LinearNgramModel:
    resolved = path.expanduser().resolve(strict=False)
    before = resolved.stat()
    key = _stat_cache_key(resolved, before)
    with _MODEL_CACHE_LOCK:
        cached = _MODEL_CACHE.get(key)
    if cached is not None:
        return cached

    model = LinearNgramModel.load(resolved)
    after = resolved.stat()
    final_key = _stat_cache_key(resolved, after)
    if final_key != key:
        model = LinearNgramModel.load(resolved)
    with _MODEL_CACHE_LOCK:
        stale = [existing for existing in _MODEL_CACHE if existing[0] == str(resolved)]
        for existing in stale:
            del _MODEL_CACHE[existing]
        _MODEL_CACHE[final_key] = model
    return model


def _stat_cache_key(path: Path, status: os.stat_result) -> tuple[str, int, int, int, int]:
    return (
        str(path),
        status.st_dev,
        status.st_ino,
        status.st_size,
        status.st_mtime_ns,
    )


def _fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError:
        return
    try:
        os.fsync(descriptor)
    except OSError:
        pass
    finally:
        os.close(descriptor)

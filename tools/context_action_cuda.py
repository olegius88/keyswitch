"""CUDA featurizer of the context-v3 trainer: the evidence and action features of frames on the GPU.

The kernels in tools/context_action_kernels/ repeat, operation for operation, what the trainer runs
for one frame: `train_context_action_model.evidence` (the detector, the intent model, the short-word
rules, ortho, the typo check, identifiers) and `extract_action_features`. What they cannot repeat
exactly stays on the host: Hunspell (the kernels record the words they need, the CPU answers, the
kernels run again), the intent sigmoid (computed by Python's own `stable_sigmoid`; the decision
compares the logit and needs no exp), and every frame the kernels cannot take (characters outside
the exported table, over-long text, a value Python would reject), whose features the trainer's own
Python computes. Logarithms appear only of integers and come from tables Python computed; sums
follow CPython's `sum` and `math.fsum`; the build forbids contraction (`--fmad=false`). A feature
name is kept as two 64-bit hashes of its UTF-8 bytes; the second one catches collisions.

The kernels follow the Python sources that `context_action_pipeline.CUDA_MIRRORED_SOURCES` lists;
`tools/context_action_kernels/parity.json` holds their hashes as of the last parity check
(`python3 tools/context_action_cuda.py --refresh-parity`), and the trainer refuses this back end
while a hash differs.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import importlib
import json
import math
import pkgutil
import re
import time
import unicodedata
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast, get_args

import context_action_pipeline as pipeline  # sets up the accelerator site before numpy and CuPy load
import cupy as cp
import numpy as np
import numpy.typing as npt

import keyswitch.constants as constants_package
import train_context_action_model as trainer
from context_action_spans import SpanFrame
from context_physical_keys import _BY_CHARACTER
from keyswitch.constants.boundary import BOUNDARY_MISSING_LETTERS
from keyswitch.constants.file_formats import UINT64_MASK
from keyswitch.constants.keyboard import LAYOUT_GROUP_COUNT
from keyswitch.constants.model_protocol import TRAIN
from keyswitch.constants.models import (
    ACTION_FEATURE_FREQUENCY_CAP,
    ACTION_FEATURE_NGRAM_ORDERS,
    CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS,
    INTENT_NGRAM_ORDERS,
    LANGUAGE_MODEL_GRAM_SMOOTHING,
    LANGUAGE_MODEL_NGRAM_ORDERS,
    LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY,
)
from keyswitch.constants.training import (
    CONTEXT_ACTION_CUDA_BATCH_FRAMES,
    CONTEXT_ACTION_CUDA_BLOCK_THREADS,
    CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES,
    CONTEXT_ACTION_CUDA_CODE_POINTS,
    CONTEXT_ACTION_CUDA_CONTEXT_CHARACTERS,
    CONTEXT_ACTION_CUDA_HUNSPELL_QUESTIONS,
    CONTEXT_ACTION_CUDA_NAME_BYTES,
    CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY,
    CONTEXT_ACTION_CUDA_ORTHO_ORDER_CAPACITY,
    CONTEXT_ACTION_CUDA_RECORD_FLAGS,
    CONTEXT_ACTION_CUDA_RESERVED_FEATURES_PER_FRAME,
    CONTEXT_ACTION_CUDA_ROW_STATES,
    CONTEXT_ACTION_CUDA_SCORE_FIELDS,
    CONTEXT_ACTION_CUDA_SCORE_FLAGS,
    CONTEXT_ACTION_CUDA_SIMPLE_BLOCKS,
    CONTEXT_ACTION_CUDA_SMALL_TABLE_CAPACITY,
    CONTEXT_ACTION_CUDA_STACK_BYTES,
    CONTEXT_ACTION_CUDA_SURROGATES,
    CONTEXT_ACTION_CUDA_TERM_TABLES,
    CONTEXT_ACTION_CUDA_TOKEN_CHARACTERS,
    CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY,
    CONTEXT_ACTION_NAME_HASH_LENGTH_SHIFT,
    CONTEXT_ACTION_NAME_HASH_MULTIPLIER,
    CONTEXT_ACTION_NAME_HASH_SEED,
    CONTEXT_ACTION_NAME_HASH_SHIFT,
    CONTEXT_ACTION_REPORTED_DIFFERENCES,
    CONTEXT_OPTIMIZER_CACHE_DIGEST_CHARACTERS,
)
from keyswitch.constants.models import FNV1A64_OFFSET_BASIS, FNV1A64_PRIME
from keyswitch.context_action_features import (
    abbreviation_question, alone_question, capitals_question, latin_abbreviation_question, letter_question, start_question,
)
from keyswitch.context_model import AfterOrigin, ContextEvidence, _term_frequency
from keyswitch.detector import PROTECTED_TOKENS
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.intent_model import TRIGGERS, LayoutDirection, LinearNgramModel, stable_sigmoid
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import _RU_TO_US, _US_TO_RU
from keyswitch.ortho_model import BOS, EOS, SHAPES, OrthoModel, _Channel
from keyswitch.short_words import OPENING_WORDS, TRUSTED_SHORT_WORDS, TRUSTED_SINGLE_LETTER_WORDS

KERNELS = pipeline.ROOT / "tools/context_action_kernels"
CACHE = pipeline.ROOT / "build/context-action-cuda"
PARITY = KERNELS / "parity.json"
KERNEL_NAMES = ("row_records", "row_features")
COMPILER_OPTIONS = ("--fmad=false", "-std=c++17")
WORD_CHARACTER = re.compile(r"[^\W\d_]")
ORIGINS: tuple[AfterOrigin, ...] = get_args(AfterOrigin)
ROW_DONE, ROW_ASKING, ROW_CPU, ROW_RETRY = (CONTEXT_ACTION_CUDA_ROW_STATES.index(name) for name in ("done", "asking", "cpu", "retry"))
# Names of a record's columns of text, in the order the kernels take them.
FEATURE_COLUMNS = ("original", "alternative", "before", "after", "application", "role", "trigger", "tail", "boundary")
ORTHO_FEATURES = ("ident", "repeat", "parts_source", "parts_target", "dot_word", "extra_key", "source_word",
                  "name_unknown", "target_unknown", "source_plausibility", "target_plausibility")
SCRIPTS = ("en", "ru")
DIRECTIONS: tuple[LayoutDirection, ...] = get_args(LayoutDirection)
LOCALES = ("en_US", "ru_RU")

UInt32Array = npt.NDArray[np.uint32]
UInt64Array = npt.NDArray[np.uint64]
Int64Array = npt.NDArray[np.int64]
Int32Array = npt.NDArray[np.int32]
FloatArray = npt.NDArray[np.float64]
BoolArray = npt.NDArray[np.bool_]
Frame = trainer.ActionRow | SpanFrame


def bit(names: Sequence[str], name: str) -> int:
    return 1 << names.index(name)


P_KNOWN = bit(CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES, "known")
RF_PROBABILITY = bit(CONTEXT_ACTION_CUDA_RECORD_FLAGS, "probability")


# ---- the kernels' sources ----------------------------------------------------------------------

def _c_literal(value: object) -> str | None:
    if type(value) is bool:
        return "1" if value else "0"
    if type(value) is int:
        if np.iinfo(np.int64).min <= value <= np.iinfo(np.int64).max:
            return f"({value}LL)"
        return f"{value}ULL" if 0 <= value <= UINT64_MASK else None
    if type(value) is float:
        return f"({value.hex()})" if math.isfinite(value) else None
    return None


def constants_header() -> str:
    """Every number of keyswitch.constants as a KS_ macro (floats bit-exact in hex), the size of every
    tuple, flat integer tuples as device arrays, and the names the kernels index by."""
    lines = ["// Generated by tools/context_action_cuda.py from keyswitch.constants; do not edit.", "#pragma once"]
    defined: dict[str, str] = {}

    def define(name: str, text: str) -> None:
        if defined.get(name, text) != text:
            raise ValueError(f"two values for the kernel constant {name}")
        if name not in defined:
            defined[name] = text
            lines.append(f"#define {name} {text}")

    for info in sorted(pkgutil.iter_modules(constants_package.__path__), key=lambda module: module.name):
        module = importlib.import_module(f"{constants_package.__name__}.{info.name}")
        for name, value in sorted(vars(module).items()):
            if name.startswith("_") or not name.isupper():
                continue
            literal = _c_literal(value)
            if literal is not None:
                define("KS_" + name, literal)
            elif isinstance(value, tuple):
                define(f"KS_{name}_COUNT", f"({len(value)}LL)")
                if value and all(type(item) is int for item in value) and f"KS_{name}" not in defined:
                    defined[f"KS_{name}"] = "array"
                    lines.append(f"static __device__ const long long KS_{name}[] = {{{', '.join(f'{item}LL' for item in value)}}};")
    for prefix, names, bits in (("P_", CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES, True),
                                ("RF_", CONTEXT_ACTION_CUDA_RECORD_FLAGS, True),
                                ("SF_", CONTEXT_ACTION_CUDA_SCORE_FLAGS, True),
                                ("SCORE_", CONTEXT_ACTION_CUDA_SCORE_FIELDS, False),
                                ("TERM_", CONTEXT_ACTION_CUDA_TERM_TABLES, False),
                                ("ROW_", CONTEXT_ACTION_CUDA_ROW_STATES, False),
                                ("SHAPE_", SHAPES, False)):
        for index, name in enumerate(names):
            define(prefix + name.upper(), f"(1u << {index})" if bits else str(index))
        define(prefix + "COUNT", str(len(names)))
    define("ORTHO_BOS", f"{ord(BOS)}u")
    define("ORTHO_EOS", f"{ord(EOS)}u")
    return "\n".join(lines) + "\n"


def kernel_source() -> str:
    """featurizer.cu with its headers inlined, the generated constants first (NVRTC reads no files)."""
    included: set[str] = set()

    def expand(name: str) -> str:
        if name in included:
            return ""
        included.add(name)
        text = constants_header() if name == "ks_constants.h" else (KERNELS / name).read_text(encoding="utf-8")
        result = []
        for line in text.splitlines():
            match = re.match(r'\s*#include "([^"]+)"', line)
            if match is not None:
                result.append(expand(match.group(1)))
            elif line.strip() != "#pragma once":
                result.append(line)
        return "\n".join(result)

    return expand("featurizer.cu") + "\n"


def compile_kernels() -> cp.RawModule:
    module = cp.RawModule(code=kernel_source(), options=COMPILER_OPTIONS)
    for name in KERNEL_NAMES:
        module.get_function(name)
    return module


# ---- characters, strings and tables --------------------------------------------------------------

def simple_character(char: str) -> bool:
    """A character the kernels take: one code point that stays one under casefold and NFC."""
    if unicodedata.combining(char) or unicodedata.normalize("NFC", char) != char:
        return False
    folded = char.casefold()
    return (len(folded) == 1 and ord(folded) < CONTEXT_ACTION_CUDA_CODE_POINTS and not unicodedata.combining(folded)
            and unicodedata.normalize("NFC", folded) == folded and folded.casefold() == folded)


class Characters:
    """Per code point of the exported blocks: property bits, casefold and upper, Python's own answers."""

    def __init__(self) -> None:
        names = CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES
        self.props = np.zeros(CONTEXT_ACTION_CUDA_CODE_POINTS, dtype=np.uint32)
        self.fold = np.arange(CONTEXT_ACTION_CUDA_CODE_POINTS, dtype=np.uint32)
        self.upper = np.arange(CONTEXT_ACTION_CUDA_CODE_POINTS, dtype=np.uint32)
        low_surrogate, high_surrogate = CONTEXT_ACTION_CUDA_SURROGATES
        for low, high in CONTEXT_ACTION_CUDA_SIMPLE_BLOCKS:
            for code in range(low, high + 1):
                char = chr(code)
                if low_surrogate <= code <= high_surrogate or not simple_character(char):
                    continue
                name = unicodedata.name(char, "")
                answers = {"known": True, "alpha": char.isalpha(), "digit": char.isdigit(), "decimal": char.isdecimal(),
                           "space": char.isspace(), "upper": char.isupper(), "lower": char.islower(),
                           "title": unicodedata.category(char) == "Lt", "alnum": char.isalnum(),
                           "word": WORD_CHARACTER.fullmatch(char) is not None,
                           "cyrillic_name": "CYRILLIC" in name, "latin_name": "LATIN" in name}
                self.props[code] = sum(bit(names, key) for key, value in answers.items() if value)
                self.fold[code] = ord(char.casefold())
                upper = char.upper()
                self.upper[code] = ord(upper) if len(upper) == 1 and ord(upper) < CONTEXT_ACTION_CUDA_CODE_POINTS else code
        self.device = (cp.asarray(self.props), cp.asarray(self.fold), cp.asarray(self.upper))


def encode(strings: Sequence[str]) -> tuple[UInt32Array, Int64Array, Int32Array]:
    """UTF-32 pool, offsets and lengths (in code points) of the strings."""
    lengths = np.fromiter((len(text) for text in strings), dtype=np.int32, count=len(strings))
    offsets = np.zeros(len(strings), dtype=np.int64)
    if len(strings):
        offsets[1:] = np.cumsum(lengths[:-1], dtype=np.int64)
    pool = np.frombuffer("".join(strings).encode("utf-32-le"), dtype=np.uint32).copy()
    return pool, offsets, lengths


def fnv_code_points(pool: UInt32Array, offsets: Int64Array, lengths: Int32Array) -> UInt64Array:
    """FNV-1a 64 over the code points of each string, as table_find hashes them on the device."""
    hashes = np.full(len(lengths), FNV1A64_OFFSET_BASIS, dtype=np.uint64)
    prime = np.uint64(FNV1A64_PRIME)
    for position in range(int(lengths.max()) if len(lengths) else 0):
        active = np.flatnonzero(lengths > position)
        hashes[active] ^= pool[offsets[active] + position].astype(np.uint64)
        hashes[active] *= prime
    return hashes


class HashTableC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        (name, ctypes.c_uint64) for name in ("hashes", "offsets", "lengths", "values", "pool", "mask")]


class Table:
    """Open addressing over unique string keys; a key's value is its index in `keys`."""

    def __init__(self, keys: Sequence[str]) -> None:
        if len(set(keys)) != len(keys):
            raise ValueError("table keys must be unique")
        self.arrays: tuple[cp.ndarray, ...] = ()
        self.mask = 0
        if not keys:
            return
        pool, offsets, lengths = encode(keys)
        hashes = fnv_code_points(pool, offsets, lengths)
        capacity = 1 << max(LAYOUT_GROUP_COUNT * LAYOUT_GROUP_COUNT, (LAYOUT_GROUP_COUNT * len(keys) - 1).bit_length())
        mask = capacity - 1
        slot_hash = np.zeros(capacity, dtype=np.uint64)
        slot_offset = np.zeros(capacity, dtype=np.int64)
        slot_length = np.full(capacity, -1, dtype=np.int32)
        slot_value = np.full(capacity, -1, dtype=np.int32)
        occupied = np.zeros(capacity, dtype=bool)
        for key, home in enumerate((hashes & np.uint64(mask)).astype(np.int64).tolist()):
            slot = home
            while occupied[slot]:
                slot = (slot + 1) & mask
            occupied[slot] = True
            slot_hash[slot], slot_offset[slot] = hashes[key], offsets[key]
            slot_length[slot], slot_value[slot] = lengths[key], key
        self.mask = mask
        self.arrays = tuple(cp.asarray(array) for array in (slot_hash, slot_offset, slot_length, slot_value, pool))

    def descriptor(self) -> HashTableC:
        if not self.arrays:
            return HashTableC(0, 0, 0, 0, 0, 0)
        return HashTableC(*(array.data.ptr for array in self.arrays), self.mask)


class Tables:
    """Keeps the device arrays a struct points to alive as long as the struct."""

    def __init__(self) -> None:
        self.keep: list[object] = []

    def table(self, keys: Sequence[str]) -> HashTableC:
        table = Table(keys)
        self.keep.append(table)
        return table.descriptor()

    def array(self, values: object) -> int:
        device = cp.asarray(values)
        self.keep.append(device)
        return device.data.ptr

    def pointer(self, structure: ctypes.Structure) -> cp.ndarray:
        """The struct's bytes on the device."""
        raw = cp.asarray(np.frombuffer(bytes(structure), dtype=np.uint8))
        self.keep.append(raw)
        return raw


Groups = ctypes.c_uint64 * LAYOUT_GROUP_COUNT
Orders = len(LANGUAGE_MODEL_NGRAM_ORDERS)


class ModelC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("props", ctypes.c_uint64), ("fold", ctypes.c_uint64), ("upper", ctypes.c_uint64),
        ("freq", HashTableC * LAYOUT_GROUP_COUNT), ("freq_count", Groups), ("popularity", Groups),
        ("grams", (HashTableC * Orders) * LAYOUT_GROUP_COUNT), ("gram_log", (ctypes.c_uint64 * Orders) * LAYOUT_GROUP_COUNT),
        ("unseen_log", (ctypes.c_double * Orders) * LAYOUT_GROUP_COUNT),
        ("ngram_mean", ctypes.c_double * LAYOUT_GROUP_COUNT), ("ngram_deviation", ctypes.c_double * LAYOUT_GROUP_COUNT),
        ("bigram", HashTableC * LAYOUT_GROUP_COUNT), ("bigram_value", Groups),
        ("hunspell", HashTableC * LAYOUT_GROUP_COUNT), ("hunspell_value", Groups),
        ("orders", ctypes.c_int64 * Orders),
    ]


class Device(Tables):
    """The language models of the two locales and the character table, as the kernels read them.

    The profiles differ in morphology only, so one frequency table per locale serves both; Hunspell
    answers are added as the kernels ask for them.
    """

    def __init__(self, models: Mapping[int, LanguageModel], characters: Characters) -> None:
        super().__init__()
        self.characters = characters
        self.model = ModelC()
        self.model.props, self.model.fold, self.model.upper = (array.data.ptr for array in characters.device)
        self.model.orders[:] = LANGUAGE_MODEL_NGRAM_ORDERS
        for group, language in models.items():
            words = list(language.frequencies)
            self.model.freq[group] = self.table(words)
            self.model.freq_count[group] = self.array(np.array([language.frequencies[word] for word in words], dtype=np.int64))
            self.model.popularity[group] = self.array(np.array(
                [math.log1p(language.frequencies[word]) / math.log1p(language.maximum) for word in words], dtype=np.float64))
            for slot, order in enumerate(LANGUAGE_MODEL_NGRAM_ORDERS):
                counter = language._gram_counts[order]
                total = language._gram_totals[order]
                vocabulary = len(counter) + LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY
                grams = list(counter)
                self.model.grams[group][slot] = self.table(grams)
                self.model.gram_log[group][slot] = self.array(np.array(
                    [math.log((counter.get(gram, 0) + LANGUAGE_MODEL_GRAM_SMOOTHING)
                              / (total + LANGUAGE_MODEL_GRAM_SMOOTHING * vocabulary)) for gram in grams], dtype=np.float64))
                self.model.unseen_log[group][slot] = math.log(
                    LANGUAGE_MODEL_GRAM_SMOOTHING / (total + LANGUAGE_MODEL_GRAM_SMOOTHING * vocabulary))
            self.model.ngram_mean[group] = language._ngram_mean
            self.model.ngram_deviation[group] = language._ngram_deviation
            pairs = list(language.bigrams)
            self.model.bigram[group] = self.table([left + "\x00" + right for left, right in pairs])
            self.model.bigram_value[group] = self.array(np.array(
                [math.log1p(language.bigrams[pair]) / math.log1p(language.maximum_bigram) for pair in pairs], dtype=np.float64))
        self.hunspell: dict[int, dict[str, bool]] = {group: {} for group in range(LAYOUT_GROUP_COUNT)}
        self.add_hunspell({})

    def add_hunspell(self, answers: Mapping[int, Mapping[str, bool]]) -> None:
        for group in range(LAYOUT_GROUP_COUNT):
            self.hunspell[group].update(answers.get(group, {}))
            words = list(self.hunspell[group])
            self.model.hunspell[group] = self.table(words)
            self.model.hunspell_value[group] = self.array(
                np.array([self.hunspell[group][word] for word in words] or [False], dtype=np.int8))


class IntentC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("weights", ctypes.c_uint64), ("dimension_mask", ctypes.c_uint64), ("fnv_seed", ctypes.c_uint64),
                ("weight_scale", ctypes.c_double), ("bias", ctypes.c_double),
                ("platt_scale", ctypes.c_double * LAYOUT_GROUP_COUNT), ("platt_bias", ctypes.c_double * LAYOUT_GROUP_COUNT),
                ("threshold_logit", (ctypes.c_double * LAYOUT_GROUP_COUNT) * CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY),
                ("threshold", (ctypes.c_double * LAYOUT_GROUP_COUNT) * CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY)]


class DecideC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("translate", Groups), ("protected_tokens", HashTableC),
                ("trusted_short_table", HashTableC * LAYOUT_GROUP_COUNT), ("single_letter", HashTableC), ("intent", IntentC)]


def translation_tables() -> tuple[UInt32Array, ...]:
    """context_physical_keys: per group, the code point of each character's key in the other group (0 = none)."""
    tables = tuple(np.zeros(CONTEXT_ACTION_CUDA_CODE_POINTS, dtype=np.uint32) for _ in range(LAYOUT_GROUP_COUNT))
    for group in range(LAYOUT_GROUP_COUNT):
        for char, key in _BY_CHARACTER[group].items():
            tables[group][ord(char)] = ord(key.characters[1 - group])
    return tables


class DecideTables(Tables):
    def __init__(self, intent: LinearNgramModel, translate: Sequence[UInt32Array]) -> None:
        super().__init__()
        if len(TRIGGERS) > CONTEXT_ACTION_CUDA_TRIGGER_CAPACITY or tuple(intent.ngram_orders) != INTENT_NGRAM_ORDERS:
            raise ValueError("the intent model does not fit the kernels")
        self.c = DecideC()
        for group in range(LAYOUT_GROUP_COUNT):
            self.c.translate[group] = self.array(translate[group])
            self.c.trusted_short_table[group] = self.table(sorted(TRUSTED_SHORT_WORDS.get(group, frozenset())))
        self.c.protected_tokens = self.table(sorted(PROTECTED_TOKENS))
        self.c.single_letter = self.table(sorted(TRUSTED_SINGLE_LETTER_WORDS))
        model = self.c.intent
        model.weights = self.array(np.frombuffer(intent._weights, dtype=np.int16))
        model.dimension_mask, model.fnv_seed = intent.dimension - 1, intent.fnv_seed
        model.weight_scale, model.bias = intent.weight_scale, intent.bias
        for index, direction in enumerate(DIRECTIONS):
            model.platt_scale[index] = intent.platt_calibration[direction].scale
            model.platt_bias[index] = intent.platt_calibration[direction].bias
            for position, trigger in enumerate(TRIGGERS):
                model.threshold_logit[position][index] = intent.threshold_logits[trigger][direction]
                model.threshold[position][index] = intent.thresholds[trigger][direction]


class MissesC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("count", ctypes.c_uint64), ("capacity", ctypes.c_int), ("words", ctypes.c_uint64),
                ("lengths", ctypes.c_uint64), ("locales", ctypes.c_uint64)]


class Misses:
    """The Hunspell questions the kernels could not answer from the table."""

    def __init__(self) -> None:
        capacity = CONTEXT_ACTION_CUDA_HUNSPELL_QUESTIONS
        self.capacity = capacity
        self.count = cp.zeros(1, dtype=cp.int32)
        self.words = cp.zeros(capacity * CONTEXT_ACTION_CUDA_TOKEN_CHARACTERS, dtype=cp.uint32)
        self.lengths = cp.zeros(capacity, dtype=cp.int32)
        self.locales = cp.zeros(capacity, dtype=cp.int32)
        record = MissesC(self.count.data.ptr, capacity, self.words.data.ptr, self.lengths.data.ptr, self.locales.data.ptr)
        self.raw = cp.asarray(np.frombuffer(bytes(record), dtype=np.uint8))

    def take(self) -> dict[int, set[str]]:
        count = min(int(cp.asnumpy(self.count)[0]), self.capacity)
        words = cp.asnumpy(self.words[:count * CONTEXT_ACTION_CUDA_TOKEN_CHARACTERS]).reshape(count, CONTEXT_ACTION_CUDA_TOKEN_CHARACTERS)
        lengths, locales = cp.asnumpy(self.lengths[:count]), cp.asnumpy(self.locales[:count])
        result: dict[int, set[str]] = {group: set() for group in range(LAYOUT_GROUP_COUNT)}
        for row, length, locale in zip(words, lengths.tolist(), locales.tolist()):
            result[locale].add(row[:length].tobytes().decode("utf-32-le"))
        self.count.fill(0)
        return result


class ChannelC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("logprob", HashTableC), ("logprob_value", ctypes.c_uint64), ("backoff", HashTableC),
                ("backoff_value", ctypes.c_uint64), ("uniform", ctypes.c_double), ("present", ctypes.c_int64)]


Shapes = ctypes.c_double * len(SHAPES)
OrthoFeatures = ctypes.c_int64 * CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY


class OrthoC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("order", ctypes.c_int64), ("channels", ChannelC * LAYOUT_GROUP_COUNT), ("source_channels", ChannelC * LAYOUT_GROUP_COUNT),
                ("prose_shape", Shapes * LAYOUT_GROUP_COUNT), ("acronym_shape", Shapes * LAYOUT_GROUP_COUNT),
                ("thresholds", ctypes.c_double * LAYOUT_GROUP_COUNT), ("feature_count", ctypes.c_int64 * LAYOUT_GROUP_COUNT),
                ("feature_code", OrthoFeatures * LAYOUT_GROUP_COUNT),
                ("feature_weight", (ctypes.c_double * CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY) * LAYOUT_GROUP_COUNT),
                ("has_centres", ctypes.c_int64), ("centres", ctypes.c_double * LAYOUT_GROUP_COUNT),
                ("has_target_centres", ctypes.c_int64), ("target_centres", ctypes.c_double * LAYOUT_GROUP_COUNT),
                ("gated_count", ctypes.c_int64), ("gated_code", OrthoFeatures),
                ("compound_min", ctypes.c_int64 * LAYOUT_GROUP_COUNT), ("collapse", ctypes.c_int64), ("stretch", ctypes.c_int64),
                ("hyphens", ctypes.c_int64), ("unscored_mask", ctypes.c_int64),
                ("us_to_ru", ctypes.c_uint64), ("ru_to_us", ctypes.c_uint64), ("identifiers", HashTableC)]


class EvidenceTablesC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("ortho", OrthoC), ("opening", HashTableC * LAYOUT_GROUP_COUNT), ("alphabet", Groups),
                ("alphabet_n", ctypes.c_int64 * LAYOUT_GROUP_COUNT)]


class EvidenceTables(Tables):
    def __init__(self, ortho: OrthoModel) -> None:
        super().__init__()
        if (ortho.order > CONTEXT_ACTION_CUDA_ORTHO_ORDER_CAPACITY or len(ortho.gated) > CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY
                or any(len(weights) > CONTEXT_ACTION_CUDA_ORTHO_FEATURE_CAPACITY for weights in ortho.features.values())):
            raise ValueError("the ortho model does not fit the kernels")
        self.c = EvidenceTablesC()
        target = self.c.ortho
        target.order = ortho.order
        for index, script in enumerate(SCRIPTS):
            target.channels[index] = self.channel(ortho.channels[script])
            if script in ortho.source_channels:
                target.source_channels[index] = self.channel(ortho.source_channels[script])
            for position, shape in enumerate(SHAPES):
                target.prose_shape[index][position] = ortho.prose_shape[script][shape]
                target.acronym_shape[index][position] = ortho.acronym_shape[script][shape]
            target.thresholds[index] = ortho.thresholds[script]
            weights = ortho.features.get(script, {})
            target.feature_count[index] = len(weights)
            for position, (name, weight) in enumerate(weights.items()):
                target.feature_code[index][position] = ORTHO_FEATURES.index(name)
                target.feature_weight[index][position] = weight
            target.compound_min[index] = ortho.compound_min_letters.get(script, 0)
            target.centres[index] = ortho.centres.get(script, 0.0)
            target.target_centres[index] = ortho.target_centres.get(script, 0.0)
        target.has_centres = bool(ortho.centres)
        target.has_target_centres = bool(ortho.target_centres)
        target.gated_count = len(ortho.gated)
        for position, name in enumerate(ortho.gated):
            target.gated_code[position] = ORTHO_FEATURES.index(name)
        target.collapse = ortho.collapse or 0
        target.stretch = ortho.stretch or 0
        target.hyphens = int(bool(ortho.hyphens))
        target.unscored_mask = sum(1 << SHAPES.index(shape) for shape in ortho.unscored_shapes)
        for attribute, mapping in (("us_to_ru", _US_TO_RU), ("ru_to_us", _RU_TO_US)):
            table = np.zeros(CONTEXT_ACTION_CUDA_CODE_POINTS, dtype=np.uint32)
            for left, right in mapping.items():
                table[ord(left)] = ord(right)
            setattr(target, attribute, self.array(table))
        lexicon, _status = IdentifierLexicon.try_load()
        target.identifiers = self.table(sorted(lexicon.identifiers) if lexicon is not None else [])
        for group, locale in enumerate(LOCALES):
            self.c.opening[group] = self.table(sorted(OPENING_WORDS.get(group, frozenset())))
            letters = BOUNDARY_MISSING_LETTERS[locale]
            self.c.alphabet[group] = self.array(np.frombuffer(letters.encode("utf-32-le"), dtype=np.uint32).copy())
            self.c.alphabet_n[group] = len(letters)

    def channel(self, channel: _Channel) -> ChannelC:
        grams, contexts = list(channel.logprob), list(channel.backoff)
        return ChannelC(self.table(grams), self.array(np.array([channel.logprob[gram] for gram in grams] or [0.0], dtype=np.float64)),
                        self.table(contexts), self.array(np.array([channel.backoff[context] for context in contexts] or [0.0], dtype=np.float64)),
                        channel.uniform, 1)


SmallTable = ctypes.c_int64 * CONTEXT_ACTION_CUDA_SMALL_TABLE_CAPACITY


class FeatureTablesC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("term", HashTableC * len(CONTEXT_ACTION_CUDA_TERM_TABLES)), ("term_counts", ctypes.c_uint64 * len(CONTEXT_ACTION_CUDA_TERM_TABLES)),
                ("bucket_bounds", SmallTable), ("bucket_count", ctypes.c_int64), ("logfrequency", HashTableC),
                ("logfrequency_value", ctypes.c_uint64), ("ngram_orders", SmallTable), ("ngram_count", ctypes.c_int64)]


class FeatureTables(Tables):
    def __init__(self, frequencies: Iterable[int]) -> None:
        super().__init__()
        if max(map(len, (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, ACTION_FEATURE_NGRAM_ORDERS))) > CONTEXT_ACTION_CUDA_SMALL_TABLE_CAPACITY:
            raise ValueError("the feature tables do not fit the kernels")
        self.c = FeatureTablesC()
        tables = _term_frequency()
        for index, alphabet in enumerate(CONTEXT_ACTION_CUDA_TERM_TABLES):
            words = list(tables[alphabet])
            self.c.term[index] = self.table(words)
            self.c.term_counts[index] = self.array(np.array([tables[alphabet][word] for word in words] or [0], dtype=np.int64))
        self.c.bucket_bounds[:len(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS)] = CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS
        self.c.bucket_count = len(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS)
        cap = ACTION_FEATURE_FREQUENCY_CAP
        values = sorted({max(0, min(cap, frequency)) for frequency in frequencies} - {0})
        self.c.logfrequency = self.table([str(value) for value in values])
        self.c.logfrequency_value = self.array(np.array([math.log1p(value) / math.log1p(cap) for value in values] or [0.0], dtype=np.float64))
        self.c.ngram_orders[:len(ACTION_FEATURE_NGRAM_ORDERS)] = ACTION_FEATURE_NGRAM_ORDERS
        self.c.ngram_count = len(ACTION_FEATURE_NGRAM_ORDERS)


class WantedC(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("hashes", ctypes.c_uint64), ("slots", ctypes.c_uint64), ("count", ctypes.c_int),
                ("names", ctypes.c_uint64), ("lengths", ctypes.c_uint64)]


SCORE_COUNT = len(CONTEXT_ACTION_CUDA_SCORE_FIELDS)
RECORD = np.dtype([("source", "f8", SCORE_COUNT), ("target", "f8", SCORE_COUNT), ("source_frequency", "i8"),
                   ("target_frequency", "i8"), ("source_flags", "i8"), ("target_flags", "i8"), ("score_delta", "f8"),
                   ("probability", "f8"), ("threshold", "f8"), ("ortho_score", "f8"), ("ortho_threshold", "f8"),
                   ("flags", "i8"), ("source_group", "i8"), ("after_origin", "i8")])


def heads_question(original: str, before: str, after: str, trigger: str) -> bool:
    """Whether a frame may have a head's second copy of its features or a head's own evidence, which the kernels do
    not make.

    The lone-word, message-start, single-letter and abbreviation heads' classes also ask for the other reading to be
    letters; every frame they could hold takes the CPU path, which decides that as the features do.
    """

    return (capitals_question(original, before) or alone_question(original, original, before, after, trigger)
            or start_question(original, original, before, after) or letter_question(original, original, before)
            or abbreviation_question(original, original, before) or latin_abbreviation_question(original, original, before))


def record_of(evidence: ContextEvidence) -> npt.NDArray[np.void]:
    """One ContextEvidence as a record: the host path for frames whose evidence Python made (span frames)."""
    record = np.zeros(1, dtype=RECORD)
    for side, score in (("source", evidence.source_score), ("target", evidence.target_score)):
        if score is not None:
            record[side][0] = tuple(getattr(score, name) for name in CONTEXT_ACTION_CUDA_SCORE_FIELDS)
            record[side + "_frequency"] = score.frequency
            record[side + "_flags"] = sum(bit(CONTEXT_ACTION_CUDA_SCORE_FLAGS, name) for name, value in
                                          (("exact", score.exact), ("spell_known", score.spell_known), ("present", True)) if value)
    record["score_delta"] = evidence.score_delta
    flags = 0
    for name, value in (("probability", evidence.model_probability), ("threshold", evidence.model_threshold),
                        ("ortho_score", evidence.ortho_score), ("ortho_threshold", evidence.ortho_threshold)):
        if value is not None:
            record[name] = value
            flags |= bit(CONTEXT_ACTION_CUDA_RECORD_FLAGS, name)
    for name, present in (("baseline", evidence.baseline_convert), ("source_known", evidence.source_known),
                          ("target_known", evidence.target_known), ("source_identifier", evidence.source_identifier),
                          ("target_identifier", evidence.target_identifier), ("source_opening", evidence.source_opening),
                          ("target_opening", evidence.target_opening), ("inside", evidence.inside)):
        if present:
            flags |= bit(CONTEXT_ACTION_CUDA_RECORD_FLAGS, name)
    record["flags"] = flags
    record["source_group"] = evidence.source_group
    record["after_origin"] = ORIGINS.index(evidence.after_origin)
    return record


def name_hashes(name: str) -> tuple[int, int]:
    """The two 64-bit hashes Name.hash computes over the UTF-8 bytes of a feature name."""
    first, second = FNV1A64_OFFSET_BASIS, CONTEXT_ACTION_NAME_HASH_SEED
    data = name.encode("utf-8")
    for byte in data:
        first = ((first ^ byte) * FNV1A64_PRIME) & UINT64_MASK
        second = (second * CONTEXT_ACTION_NAME_HASH_MULTIPLIER + byte + 1) & UINT64_MASK
        second ^= second >> CONTEXT_ACTION_NAME_HASH_SHIFT
    return first, second ^ ((len(data) << CONTEXT_ACTION_NAME_HASH_LENGTH_SHIFT) & UINT64_MASK)


# ---- the featurizer ----------------------------------------------------------------------------

class Dictionary:
    """Feature names seen so far, by id; on the device sorted by the first hash, the second one checks collisions."""

    def __init__(self) -> None:
        self.names: list[str] = []
        self.index: dict[str, int] = {}
        self.first = cp.zeros(0, dtype=cp.uint64)
        self.second = cp.zeros(0, dtype=cp.uint64)
        self.ids = cp.zeros(0, dtype=cp.int64)

    def add(self, first: cp.ndarray, second: cp.ndarray, names: Sequence[str]) -> None:
        start = len(self.names)
        for offset, name in enumerate(names):
            if name in self.index:
                raise ValueError("a feature name arrived twice")
            self.index[name] = start + offset
        self.names.extend(names)
        hashes = cp.concatenate([self.first, first])
        checks = cp.concatenate([self.second, second])
        ids = cp.concatenate([self.ids, cp.arange(start, start + len(names), dtype=cp.int64)])
        order = cp.argsort(hashes)
        self.first, self.second, self.ids = hashes[order], checks[order], ids[order]
        if len(self.first) > 1 and bool(cp.any(self.first[1:] == self.first[:-1])):
            raise ValueError("two feature names share a 64-bit hash")

    def lookup(self, first: cp.ndarray) -> tuple[cp.ndarray, cp.ndarray]:
        if len(self.first) == 0:
            return cp.zeros(len(first), dtype=cp.bool_), cp.zeros(len(first), dtype=cp.int64)
        position = cp.minimum(cp.searchsorted(self.first, first), len(self.first) - 1)
        return self.first[position] == first, position


class Items:
    """The distinct frames of one split (both profiles), encoded for the kernels."""

    def __init__(self, rows: Sequence[Frame], characters: Characters, translate: Sequence[UInt32Array], split: str) -> None:
        count = len(rows)
        strings: list[list[str]] = [[] for _ in FEATURE_COLUMNS]
        groups = np.zeros(count, np.int32)
        triggers = np.zeros(count, np.int32)
        origins = np.zeros(count, np.int32)
        dropped = np.zeros(count, np.uint8)
        action_row = np.zeros(count, bool)
        fits = np.ones(count, bool)
        span_alternatives: list[tuple[int, str]] = []
        self.span_records: dict[int, npt.NDArray[np.void]] = {}
        trigger_index = {name: index for index, name in enumerate(TRIGGERS)}
        origin_index = {name: index for index, name in enumerate(ORIGINS)}
        for number, row in enumerate(rows):
            values: tuple[str, ...]
            if isinstance(row, trainer.ActionRow):
                field = row.field
                values = (row.original, "", field.before, field.after, field.application, field.role,
                          row.trigger, row.literal_tail, row.boundary_text)
                groups[number] = row.group
                triggers[number] = trigger_index.get(row.trigger, 0)
                origins[number] = origin_index.get(row.after_origin, 0)
                # The kernels leave out the kept-neighbour question, whose features are renamed
                # (KEPT_FEATURE_PREFIX), the heads' second copy of a token's features (CAPITALS_FEATURE_PREFIX,
                # ALONE_FEATURE_PREFIX, START_FEATURE_PREFIX, LETTER_FEATURE_PREFIX) and the heads' own evidence
                # (ABBREVIATION_FEATURE_PREFIX, LATIN_ABBREVIATION_FEATURE_PREFIX): those frames take the CPU path.
                fits[number] = (row.trigger in trigger_index and row.after_origin in origin_index
                                and row.after_origin != "kept_next_word"
                                and not heads_question(row.original, row.field.before, row.field.after, row.trigger)
                                and row.group in range(LAYOUT_GROUP_COUNT))
                dropped[number] = split == TRAIN and trainer.identifier_evidence_dropped(row.identifier)
                action_row[number] = True
            else:
                evidence = row.evidence
                field = evidence.field
                values = (evidence.original, "", field.before, field.after, field.application, field.role,
                          evidence.trigger, evidence.literal_tail, evidence.boundary_text)
                self.span_records[number] = record_of(evidence)
                span_alternatives.append((number, evidence.alternative))
                fits[number] = not heads_question(evidence.original, field.before, field.after, evidence.trigger)
            for column, value in enumerate(values):
                strings[column].append(value)
        host_columns = [encode(texts) for texts in strings]
        # The other reading of an ActionRow: its keys in the other layout (context_physical_keys.translated),
        # by table; a glyph without a key sends the frame to the CPU path, where Python raises as it would.
        pool, offsets, lengths = host_columns[0]
        rows_of = np.repeat(np.arange(count), lengths)
        safe = np.minimum(pool, CONTEXT_ACTION_CUDA_CODE_POINTS - 1)
        translated = np.where(groups[rows_of] == 0, translate[0][safe], translate[1][safe]).astype(np.uint32)
        missing = (translated == 0) | (pool >= CONTEXT_ACTION_CUDA_CODE_POINTS)
        missing_rows = np.zeros(count, np.int64)
        np.add.at(missing_rows, rows_of[missing], 1)
        fits &= ~(action_row & (missing_rows > 0))
        span_pool = np.frombuffer("".join(text for _, text in span_alternatives).encode("utf-32-le"), dtype=np.uint32)
        alternative_offsets, alternative_lengths = offsets.copy(), lengths.copy()
        cursor = len(translated)
        for number, text in span_alternatives:
            alternative_offsets[number], alternative_lengths[number] = cursor, len(text)
            cursor += len(text)
        host_columns[1] = (np.concatenate([translated, span_pool]), alternative_offsets, alternative_lengths)
        self.arrays: list[tuple[cp.ndarray, cp.ndarray, cp.ndarray]] = []
        for column, (pool, offsets, lengths) in enumerate(host_columns):
            unknown = (pool >= CONTEXT_ACTION_CUDA_CODE_POINTS) | (
                (characters.props[np.minimum(pool, CONTEXT_ACTION_CUDA_CODE_POINTS - 1)] & P_KNOWN) == 0)
            cumulative = np.concatenate([[0], np.cumsum(unknown, dtype=np.int64)])
            fits &= (cumulative[offsets + lengths] - cumulative[offsets]) == 0
            # The word and its other reading carry two characters of padding in the kernels.
            limit = CONTEXT_ACTION_CUDA_TOKEN_CHARACTERS - LAYOUT_GROUP_COUNT if column < LAYOUT_GROUP_COUNT else CONTEXT_ACTION_CUDA_CONTEXT_CHARACTERS
            fits &= lengths <= limit
            self.arrays.append((cp.asarray(pool if len(pool) else np.zeros(1, np.uint32)), cp.asarray(offsets), cp.asarray(lengths)))
        self.groups, self.triggers, self.origins = cp.asarray(groups), cp.asarray(triggers), cp.asarray(origins)
        self.dropped = cp.asarray(dropped)
        self.action_row, self.fits = action_row, fits

    def columns(self) -> tuple[cp.ndarray, ...]:
        return tuple(item for triple in self.arrays for item in triple)


class Chunks:
    """Feature entries of some frames, put into frame order at the end."""

    def __init__(self, count: int) -> None:
        self.count = count
        self.frames: list[Int64Array] = []
        self.counts: list[Int64Array] = []
        self.ids: list[Int64Array] = []
        self.values: list[FloatArray] = []

    def add(self, frames: Int64Array, counts: Int64Array, ids: Int64Array, values: FloatArray) -> None:
        self.frames.append(frames.astype(np.int64))
        self.counts.append(counts.astype(np.int64))
        self.ids.append(ids.astype(np.int64))
        self.values.append(values.astype(np.float64))

    def ordered(self) -> tuple[UInt32Array, FloatArray, Int64Array]:
        frames = np.concatenate([np.zeros(0, np.int64), *self.frames])
        counts = np.concatenate([np.zeros(0, np.int64), *self.counts])
        ids = np.concatenate([np.zeros(0, np.int64), *self.ids])
        values = np.concatenate([np.zeros(0, np.float64), *self.values])
        if len(frames) != self.count or len(np.unique(frames)) != self.count:
            raise ValueError("a frame was featurised twice or not at all")
        starts = np.cumsum(counts) - counts
        order = np.argsort(frames, kind="stable")
        row_counts = counts[order]
        index = pipeline.gather_index(starts[order], row_counts)
        return ids[index].astype(np.uint32), values[index], row_counts


class Featurizer:
    """Evidence and features of frames on the GPU; the frames it cannot take, by the trainer's Python."""

    def __init__(self, models: Mapping[int, LanguageModel], spelling: Mapping[int, LanguageModel], intent: LinearNgramModel,
                 ortho: OrthoModel, keep_importance: float) -> None:
        started = time.monotonic()
        self.characters = Characters()
        self.translate = translation_tables()
        self.device = Device(models, self.characters)
        self.decide = DecideTables(intent, self.translate)
        self.evidence_tables = EvidenceTables(ortho)
        self.feature_tables = FeatureTables(frequency for model in models.values() for frequency in model.frequencies.values())
        module = compile_kernels()
        self.records_kernel = module.get_function(KERNEL_NAMES[0])
        self.features_kernel = module.get_function(KERNEL_NAMES[1])
        self.misses = Misses()
        self.spelling = spelling
        self.keep_importance = keep_importance
        self.dictionary = Dictionary()
        self.decide_pointer = self.decide.pointer(self.decide.c)
        self.evidence_pointer = self.evidence_tables.pointer(self.evidence_tables.c)
        self.features_pointer = self.feature_tables.pointer(self.feature_tables.c)
        self.model_pointer = self.device.pointer(self.device.model)
        self.cache = CACHE / f"hunspell-{self._dictionaries_digest()}.json"
        self.hunspell_asked = 0
        self.cpu_frames = 0
        if self.cache.exists():
            stored = cast(dict[str, dict[str, bool]], json.loads(self.cache.read_text(encoding="utf-8")))
            self._add_answers({int(group): answers for group, answers in stored.items()})
        cp.cuda.runtime.deviceSetLimit(cp.cuda.runtime.cudaLimitStackSize, CONTEXT_ACTION_CUDA_STACK_BYTES)
        self.setup_seconds = time.monotonic() - started

    def _dictionaries_digest(self) -> str:
        digest = hashlib.sha256()
        for group in range(LAYOUT_GROUP_COUNT):
            source = Path(self.spelling[group].speller.source)
            for path in (source, source.with_suffix(".aff")):
                digest.update(path.read_bytes())
        return digest.hexdigest()[:CONTEXT_OPTIMIZER_CACHE_DIGEST_CHARACTERS]

    def _add_answers(self, answers: Mapping[int, Mapping[str, bool]]) -> None:
        self.device.add_hunspell(answers)
        self.model_pointer = self.device.pointer(self.device.model)

    def _answer(self, asked: Mapping[int, set[str]]) -> None:
        self.hunspell_asked += sum(len(words) for words in asked.values())
        self._add_answers({group: {word: self.spelling[group].speller.check(word) for word in sorted(words)}
                           for group, words in asked.items()})

    def save_cache(self) -> None:
        """Keep the Hunspell answers for the next fit with the same dictionaries."""
        CACHE.mkdir(parents=True, exist_ok=True)
        temporary = self.cache.with_suffix(".pending")
        temporary.write_text(json.dumps({str(group): answers for group, answers in self.device.hunspell.items()},
                                        ensure_ascii=False, sort_keys=True), encoding="utf-8")
        temporary.replace(self.cache)

    def featurize_split(self, entries: Mapping[str, Sequence[Frame]], split: str, spelling: Mapping[str, bool],
                        python_features: Mapping[str, Callable[[Frame], dict[str, float]]]) -> dict[str, pipeline.Columns]:
        """The features of each profile's frames, in its order; a frame both profiles hold is encoded once."""
        distinct: list[Frame] = []
        found: dict[Frame, int] = {}
        selection: dict[str, Int64Array] = {}
        for profile, frames in entries.items():
            chosen = np.empty(len(frames), np.int64)
            for position, row in enumerate(frames):
                number = found.get(row)
                if number is None:
                    number = found[row] = len(distinct)
                    distinct.append(row)
                chosen[position] = number
            selection[profile] = chosen
        items = Items(distinct, self.characters, self.translate, split)
        return {profile: self._profile(items, chosen, spelling[profile], python_features[profile], distinct)
                for profile, chosen in selection.items()}

    def _profile(self, items: Items, chosen: Int64Array, spelling: bool, python_features: Callable[[Frame], dict[str, float]],
                 distinct: Sequence[Frame]) -> pipeline.Columns:
        count = len(chosen)
        fits = items.fits[chosen]
        taken = np.flatnonzero(fits)            # positions of this profile's frames the GPU takes
        n = len(taken)
        rows = cp.asarray(chosen[taken].astype(np.int32))
        records = cp.zeros((max(n, 1), RECORD.itemsize), dtype=cp.uint8)
        states = cp.full(n, ROW_ASKING, dtype=cp.int32)
        spans = [position for position in range(n) if not items.action_row[chosen[taken[position]]]]
        if spans:
            host = np.concatenate([items.span_records[int(chosen[taken[position]])] for position in spans])
            where = cp.asarray(np.array(spans, np.int64))
            records[where] = cp.asarray(host.view(np.uint8).reshape(len(spans), RECORD.itemsize))
            states[where] = ROW_DONE
        blocks = ((n + CONTEXT_ACTION_CUDA_BLOCK_THREADS - 1) // CONTEXT_ACTION_CUDA_BLOCK_THREADS,)
        while n:
            self.records_kernel(blocks, (CONTEXT_ACTION_CUDA_BLOCK_THREADS,), (
                self.model_pointer, self.decide_pointer, self.evidence_pointer, self.misses.raw, *items.columns(),
                items.groups, items.triggers, items.origins, items.dropped, rows, np.int32(n), np.int32(int(spelling)), records, states))
            asked = self.misses.take()
            if not any(asked.values()):
                break
            self._answer(asked)
        if n:
            host = cp.asnumpy(records[:n]).view(RECORD).reshape(n)
            action = items.action_row[chosen[taken]]
            calibrated = np.flatnonzero(((host["flags"] & RF_PROBABILITY) != 0) & action)
            # The kernels leave the calibrated logit; the probability is Python's own stable_sigmoid of it.
            host["probability"][calibrated] = [stable_sigmoid(float(value)) for value in host["probability"][calibrated].tolist()]
            records = cp.asarray(host.view(np.uint8).reshape(n, RECORD.itemsize))
        state = cp.asnumpy(states).view(np.int32) if n else np.zeros(0, np.int32)
        chunks = Chunks(count)
        cpu = set(np.flatnonzero(~fits).tolist()) | set(taken[state == ROW_CPU].tolist())
        queue = [np.flatnonzero(state == ROW_DONE)]
        while queue:
            pending = queue.pop()
            if len(pending) > CONTEXT_ACTION_CUDA_BATCH_FRAMES:
                queue.extend(np.array_split(pending, (len(pending) + CONTEXT_ACTION_CUDA_BATCH_FRAMES - 1) // CONTEXT_ACTION_CUDA_BATCH_FRAMES))
                continue
            if not len(pending):
                continue
            done, entry_counts, entry_ids, entry_values, outcome = self._features(items, chosen[taken], pending, records)
            chunks.add(taken[done], entry_counts, entry_ids, entry_values)
            cpu |= set(taken[pending[outcome == ROW_CPU]].tolist())
            again = pending[outcome == ROW_RETRY]
            if len(again):
                queue.append(again)
            cp.get_default_memory_pool().free_all_blocks()
        for position in sorted(cpu):
            row_ids, row_values = self._from_names(python_features(distinct[int(chosen[position])]))
            chunks.add(np.array([position], np.int64), np.array([len(row_ids)], np.int64), row_ids, row_values)
        self.cpu_frames += len(cpu)
        ids, values, counts = chunks.ordered()
        rows_of = [distinct[int(number)] for number in chosen.tolist()]
        return pipeline.Columns.of(ids, values, counts, rows_of, self.keep_importance)

    def _features(self, items: Items, item_rows: Int64Array, pending: Int64Array, records: cp.ndarray,
                  ) -> tuple[Int64Array, Int64Array, Int64Array, FloatArray, Int32Array]:
        """Features of the frames `pending` (indices into records): those finished, their entries, every frame's state."""
        n = len(pending)
        rows = cp.asarray(item_rows[pending].astype(np.int32))
        sub_records = records[cp.asarray(pending)]
        capacity = max(CONTEXT_ACTION_CUDA_BATCH_FRAMES, n * CONTEXT_ACTION_CUDA_RESERVED_FEATURES_PER_FRAME)
        cursor = cp.zeros(1, dtype=cp.uint64)
        first, second = cp.zeros(capacity, dtype=cp.uint64), cp.zeros(capacity, dtype=cp.uint64)
        values = cp.zeros(capacity, dtype=cp.float64)
        row_start, row_n, row_state = cp.zeros(n, dtype=cp.int64), cp.zeros(n, dtype=cp.int32), cp.zeros(n, dtype=cp.int32)
        blocks = ((n + CONTEXT_ACTION_CUDA_BLOCK_THREADS - 1) // CONTEXT_ACTION_CUDA_BLOCK_THREADS,)
        self.features_kernel(blocks, (CONTEXT_ACTION_CUDA_BLOCK_THREADS,), (
            self.model_pointer, self.features_pointer, *items.columns(), rows, sub_records, cp.zeros(n, dtype=cp.int32),
            np.int32(n), np.uint64(0), cursor, np.int64(capacity), first, second, values, row_start, row_n, row_state))
        state = cp.asnumpy(row_state).view(np.int32)
        done = np.flatnonzero(state == ROW_DONE)
        counts = row_n[cp.asarray(done)].astype(cp.int64)
        index = gather_index_on_device(row_start[cp.asarray(done)], counts)
        entry_first, entry_second, entry_values = first[index], second[index], values[index]
        unique, positions = cp.unique(entry_first, return_index=True)
        known, _position = self.dictionary.lookup(unique)
        new = unique[~known]
        if len(new):
            names = self._names(items, item_rows, pending[done], sub_records[cp.asarray(done)], new)
            self.dictionary.add(new, entry_second[positions[~known]], names)
        known, position = self.dictionary.lookup(entry_first)
        if not bool(cp.all(known)) or not bool(cp.all(self.dictionary.second[position] == entry_second)):
            raise ValueError("a feature hash was not found or two names collided")
        return (pending[done], cp.asnumpy(counts).view(np.int64), cp.asnumpy(self.dictionary.ids[position]).view(np.int64),
                cp.asnumpy(entry_values).view(np.float64), state)

    def _from_names(self, features: Mapping[str, float]) -> tuple[Int64Array, FloatArray]:
        """A frame the CPU computed: its new names join the dictionary, with their hashes for later GPU frames."""
        new = [name for name in features if name not in self.dictionary.index]
        if new:
            hashes = [name_hashes(name) for name in new]
            self.dictionary.add(cp.asarray(np.array([first for first, _ in hashes], np.uint64)),
                                cp.asarray(np.array([second for _, second in hashes], np.uint64)), new)
        ordered = sorted(features.items())
        return (np.array([self.dictionary.index[name] for name, _ in ordered], np.int64),
                np.array([value for _, value in ordered], np.float64))

    def _names(self, items: Items, item_rows: Int64Array, positions: Int64Array, records: cp.ndarray, wanted: cp.ndarray) -> list[str]:
        """The strings of `wanted` (sorted first hashes), written back by the frames that made them."""
        count = len(wanted)
        slots = cp.arange(count, dtype=cp.int64)
        names = cp.zeros(count * CONTEXT_ACTION_CUDA_NAME_BYTES, dtype=cp.uint8)
        lengths = cp.full(count, -1, dtype=cp.int32)
        raw = cp.asarray(np.frombuffer(bytes(WantedC(wanted.data.ptr, slots.data.ptr, count, names.data.ptr, lengths.data.ptr)), dtype=np.uint8))
        n = len(positions)
        rows = cp.asarray(item_rows[positions].astype(np.int32))
        unused = cp.zeros(1, dtype=cp.uint64)
        self.features_kernel(((n + CONTEXT_ACTION_CUDA_BLOCK_THREADS - 1) // CONTEXT_ACTION_CUDA_BLOCK_THREADS,), (CONTEXT_ACTION_CUDA_BLOCK_THREADS,), (
            self.model_pointer, self.features_pointer, *items.columns(), rows, records, cp.zeros(n, dtype=cp.int32), np.int32(n), raw,
            cp.zeros(1, dtype=cp.uint64), np.int64(0), unused, unused, cp.zeros(1, dtype=cp.float64),
            cp.zeros(1, dtype=cp.int64), cp.zeros(1, dtype=cp.int32), cp.zeros(1, dtype=cp.int32)))
        written = cp.asnumpy(lengths).view(np.int32)
        if (written < 0).any():
            raise ValueError("a new feature name was not written back")
        data = cp.asnumpy(names).reshape(count, CONTEXT_ACTION_CUDA_NAME_BYTES)
        return [bytes(data[slot, :written[slot]]).decode("utf-8") for slot in range(count)]


def gather_index_on_device(starts: cp.ndarray, counts: cp.ndarray) -> cp.ndarray:
    """pipeline.gather_index on the device."""
    total = int(cp.asnumpy(counts.sum()).view(np.int64).sum()) if len(counts) else 0
    if not total:
        return cp.zeros(0, dtype=cp.int64)
    out_start = cp.cumsum(counts) - counts
    positions = cp.arange(0, total, dtype=cp.int64)
    rows = cp.searchsorted(out_start + counts, positions, side="right")
    return starts[rows] + (positions - out_start[rows])


def release_device_memory() -> None:
    cp.get_default_memory_pool().free_all_blocks()


def device_sort(keys: Int64Array) -> Int64Array:
    """A stable argsort of the keys on the device (cupy.argsort keeps equal keys in order)."""
    return cast(Int64Array, cp.asnumpy(cp.argsort(cp.asarray(keys))))


# ---- the parity check --------------------------------------------------------------------------

@dataclass(frozen=True)
class ParityResult:
    frames: int
    cpu_frames: int
    mismatches: list[str]


def check_parity(frames_by_split: Mapping[str, Sequence[Frame]], inputs: trainer.FitInputs) -> ParityResult:
    """Featurise the frames on the GPU and with the trainer's Python, for every profile, and compare."""
    profiles = inputs.profiles
    lexicons = inputs.lexical_models
    featurizer = Featurizer(lexicons[profiles[0]], pipeline.spelling_lexicons(inputs), inputs.intent, inputs.ortho,
                            float(cast(float, inputs.options["keep_importance"])))
    mismatches: list[str] = []
    total = 0
    for split, frames in frames_by_split.items():
        python = {profile: pipeline.python_features(inputs, profile, split) for profile in profiles}
        result = featurizer.featurize_split({profile: frames for profile in profiles}, split,
                                            {profile: lexicons[profile][0].speller.available for profile in profiles}, python)
        for profile in profiles:
            for frame, (actual, _label, _importance) in zip(frames, result[profile].rows(featurizer.dictionary.names)):
                expected = python[profile](frame)
                if actual != expected or any(repr(actual[name]) != repr(value) for name, value in expected.items()):
                    differing = sorted(set(actual.items()) ^ set(expected.items()))[:CONTEXT_ACTION_REPORTED_DIFFERENCES]
                    mismatches.append(f"{profile}/{split} {pipeline.frame_identifier(frame)}: {differing}")
            total += len(frames)
    featurizer.save_cache()
    return ParityResult(total, featurizer.cpu_frames, mismatches)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--refresh-parity", action="store_true",
                        help="compare the GPU with the Python features and, when every frame agrees, record the sources' hashes")
    parser.add_argument("--corpus", type=Path, required=True, help="a fitting corpus (its TEST split is never read)")
    parser.add_argument("--frames", type=int, default=None, help="at most this many frames per split")
    args = parser.parse_args(argv)
    inputs = trainer.load_inputs(args.corpus, trainer.recipe())
    with pipeline.bound_process_state(inputs):
        frames = pipeline.parity_frames(inputs, args.frames)
        result = check_parity(frames, inputs)
    print(f"parity: {result.frames} frames, {result.cpu_frames} on the CPU path, {len(result.mismatches)} differ", flush=True)
    for line in result.mismatches[:CONTEXT_ACTION_REPORTED_DIFFERENCES]:
        print("  " + line, flush=True)
    if result.mismatches:
        return 1
    if args.refresh_parity:
        pipeline.write_parity_record(PARITY)
        print(f"recorded {PARITY.relative_to(pipeline.ROOT)}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

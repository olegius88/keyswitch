"""Orthotactic plausibility over the physical key sequence.

The engine observes which keys were pressed, not which letters appeared. Because
the us/ru map is a bijection on the mapped keys, one key sequence has exactly two
readings, and a character model per language over that shared space turns the
question "was the layout wrong?" into a likelihood ratio that needs no dictionary
at all. That is what lets an unknown token such as ``htop`` be judged: the model
never asks whether ``htop`` is a word, only whether ``рещз`` is a worse Russian
sequence than ``htop`` is an English one.

The ratio alone is not enough, and the shape channel is why. The worst genuine
Russian sequences under the ratio are abbreviations - РСФСР, ЛДПР, РНК - whose
key renderings look thoroughly English. A person writing an abbreviation writes
it in capitals, so an all-lowercase token cannot be excused as one. Case is
therefore counted evidence here, not a veto.

This module only scores. It never decides, never injects and never overrides an
explicit user rule; the policy layer applies it.
"""

from __future__ import annotations

import json
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from typing import Final

ARTIFACT_PATH: Final[Path] = Path(__file__).parent / "resources" / "models" / "ortho_v1.json"
MAX_ARTIFACT_BYTES: Final[int] = 24 * 1024 * 1024
MAX_GRAMS: Final[int] = 4_000_000
MIN_ORDER: Final[int] = 2
MAX_ORDER: Final[int] = 8
SCRIPTS: Final[tuple[str, str]] = ("en", "ru")
SHAPES: Final[tuple[str, ...]] = ("lower", "initial", "inner", "upper")
BOS: Final[str] = "\x02"
EOS: Final[str] = "\x03"


@dataclass(frozen=True)
class OrthoEvidence:
    """One token as the engine saw it: the keys, the case shape, the layout."""

    keys: str
    shape: str
    source_script: str


@dataclass(frozen=True)
class OrthoScore:
    """Nats in favour of reading the keys in the other layout."""

    ratio: float
    shape_ratio: float
    total: float
    source_logprob: float
    target_logprob: float
    supported: bool


def shape_of(token: str, first_in_field: bool) -> str:
    """The case shape of a token as typed, in the vocabulary the model counts."""

    letters = [character for character in token if character.isalpha()]
    if len(letters) >= 2 and all(character.isupper() for character in letters):
        return "upper"
    if token[:1].isupper():
        return "initial" if first_in_field else "inner"
    return "lower"


class _Channel:
    """One language's character model in key space, ARPA-shaped for lookup."""

    def __init__(self, order: int, logprob: dict[str, float], backoff: dict[str, float],
                 uniform: float) -> None:
        self.order = order
        self.logprob = logprob
        self.backoff = backoff
        self.uniform = uniform

    def _conditional(self, gram: str) -> float:
        """log P(last | context), walking the backoff chain to the unigram."""

        accumulated = 0.0
        while True:
            found = self.logprob.get(gram)
            if found is not None:
                return accumulated + found
            if len(gram) == 1:
                return accumulated + self.uniform
            accumulated += self.backoff.get(gram[:-1], 0.0)
            gram = gram[1:]

    def score(self, keys: str) -> float:
        """log P(keys) with an explicit boundary, so length is scored honestly."""

        padded = BOS * (self.order - 1) + keys + EOS
        return sum(
            self._conditional(padded[index - self.order + 1:index + 1])
            for index in range(self.order - 1, len(padded))
        )


class OrthoModel:
    """Loaded orthotactic model. Construction validates; scoring never raises."""

    def __init__(self, order: int, channels: dict[str, _Channel],
                 prose_shape: dict[str, dict[str, float]],
                 acronym_shape: dict[str, dict[str, float]], version: str,
                 thresholds: dict[str, float], minimum_length: int) -> None:
        self.order = order
        self.channels = channels
        self.prose_shape = prose_shape
        self.acronym_shape = acronym_shape
        self.version = version
        self.thresholds = thresholds
        self.minimum_length = minimum_length

    @classmethod
    def load(cls, path: Path = ARTIFACT_PATH) -> OrthoModel:
        with path.open("rb") as handle:
            content = handle.read(MAX_ARTIFACT_BYTES + 1)
        if len(content) > MAX_ARTIFACT_BYTES:
            raise ValueError("oversized orthotactic artifact")
        payload: object = json.loads(content)
        if not isinstance(payload, dict) or payload.get("schema_version") != 1:
            raise ValueError("unsupported orthotactic artifact")
        order = payload.get("order")
        version = payload.get("version")
        scale = payload.get("scale")
        if (not isinstance(order, int) or not MIN_ORDER <= order <= MAX_ORDER
                or not isinstance(version, str) or not version.startswith("ortho-v1-")
                or not isinstance(scale, int) or not 1 <= scale <= 4096):
            raise ValueError("invalid orthotactic identity")
        models = payload.get("models")
        if not isinstance(models, dict) or set(models) != set(SCRIPTS):
            raise ValueError("orthotactic artifact must carry both scripts")
        channels: dict[str, _Channel] = {}
        for script in SCRIPTS:
            channels[script] = cls._channel(models[script], order, scale)
        raw_thresholds = payload.get("thresholds")
        minimum_length = payload.get("minimum_length")
        if (not isinstance(raw_thresholds, dict) or set(raw_thresholds) != set(SCRIPTS)
                or not isinstance(minimum_length, int) or not 1 <= minimum_length <= 32):
            raise ValueError("orthotactic artifact carries no usable decision threshold")
        thresholds: dict[str, float] = {}
        for script in SCRIPTS:
            value = raw_thresholds[script]
            if not isinstance(value, int):
                raise ValueError("invalid orthotactic threshold")
            thresholds[script] = value / scale
        return cls(order, channels, cls._shape(payload.get("prose_shape"), scale),
                   cls._shape(payload.get("acronym_shape"), scale), version,
                   thresholds, minimum_length)

    @staticmethod
    def _channel(raw: object, order: int, scale: int) -> _Channel:
        if not isinstance(raw, dict):
            raise ValueError("invalid orthotactic channel")
        grams, logprob, backoff, uniform = (raw.get("grams"), raw.get("logprob"),
                                            raw.get("backoff"), raw.get("uniform"))
        if (not isinstance(grams, str) or not isinstance(logprob, list)
                or not isinstance(backoff, list) or not isinstance(uniform, int)):
            raise ValueError("invalid orthotactic channel")
        names = grams.split("\n") if grams else []
        if len(names) != len(logprob) or len(names) > MAX_GRAMS:
            raise ValueError("orthotactic channel is inconsistent")
        table: dict[str, float] = {}
        for name, value in zip(names, logprob, strict=True):
            if not isinstance(value, int) or len(name) > order:
                raise ValueError("invalid orthotactic weight")
            table[name] = value / scale
        weights: dict[str, float] = {}
        for entry in backoff:
            if not isinstance(entry, list) or len(entry) != 2:
                raise ValueError("invalid orthotactic backoff")
            name, value = entry
            if not isinstance(name, str) or not isinstance(value, int):
                raise ValueError("invalid orthotactic backoff")
            weights[name] = value / scale
        return _Channel(order, table, weights, uniform / scale)

    @staticmethod
    def _shape(raw: object, scale: int) -> dict[str, dict[str, float]]:
        if not isinstance(raw, dict) or set(raw) != set(SCRIPTS):
            raise ValueError("orthotactic artifact must carry a shape channel")
        result: dict[str, dict[str, float]] = {}
        for script in SCRIPTS:
            table = raw[script]
            if not isinstance(table, dict) or set(table) != set(SHAPES):
                raise ValueError("invalid shape channel")
            values: dict[str, float] = {}
            for shape in SHAPES:
                value = table[shape]
                if not isinstance(value, int):
                    raise ValueError("invalid shape channel")
                values[shape] = value / scale
            result[script] = values
        return result

    @classmethod
    def try_load(cls, path: Path = ARTIFACT_PATH) -> tuple[OrthoModel | None, str]:
        try:
            model = cls.load(path)
        except (OSError, ValueError, TypeError) as error:
            return None, f"unavailable: {type(error).__name__}"
        return model, model.version

    def score(self, evidence: OrthoEvidence) -> OrthoScore:
        """Evidence, in nats, that the keys were meant for the other layout."""

        source = evidence.source_script
        target = "ru" if source == "en" else "en"
        if source not in SCRIPTS or not evidence.keys:
            return OrthoScore(0.0, 0.0, 0.0, 0.0, 0.0, False)
        keys = unicodedata.normalize("NFC", evidence.keys).casefold()
        source_logprob = self.channels[source].score(keys)
        target_logprob = self.channels[target].score(keys)
        ratio = target_logprob - source_logprob
        shape = evidence.shape if evidence.shape in SHAPES else "lower"
        # An abbreviation is written in capitals. A lowercase token therefore
        # cannot be excused as one, which is what removes the dominant class of
        # false conversions: РСФСР, ЛДПР, РНК all read as fluent English keys.
        source_acronym = self.acronym_shape[source][shape] - self.prose_shape[source][shape]
        shape_ratio = -source_acronym
        return OrthoScore(ratio, shape_ratio, ratio + shape_ratio,
                          source_logprob, target_logprob, True)


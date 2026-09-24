"""Bounded experimental action features shared by training and inference."""

from __future__ import annotations

import math
import re
import unicodedata
from typing import TYPE_CHECKING
from .constants.models import (
    ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS,
    ACTION_FEATURE_APPLICATION_NAME_CHARACTERS,
    ACTION_FEATURE_APPLICATION_TOKEN_COUNT,
    ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS,
    ACTION_FEATURE_CHARACTER_WEIGHT_CAP,
    ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS,
    ACTION_FEATURE_FREQUENCY_CAP,
    ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS,
    ACTION_FEATURE_NEIGHBOUR_WORD_COUNT,
    ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_NGRAM_ORDERS,
    ACTION_FEATURE_ORTHO_SCORE_BOUND,
    ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND,
    ACTION_FEATURE_SHORT_TEXT_CHARACTERS,
    ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_SCORE_BOUND,
    PLANNED_CONTEXT_AFTER_MAX_CHARACTERS,
    PLANNED_CONTEXT_WORD_MAX_CHARACTERS,
)

if TYPE_CHECKING:
    from .context_model import ContextEvidence
    from .language_model import WordScore


_WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)


def _bounded(value: float, lower: float = -1.0, upper: float = 1.0) -> float:
    if not math.isfinite(value):
        raise ValueError("nonfinite context action evidence")
    return max(lower, min(upper, value))


def _script(text: str) -> str:
    ru = any("а" <= char <= "я" or char == "ё" for char in text)
    en = any("a" <= char <= "z" for char in text)
    return "mixed" if ru and en else "ru" if ru else "en" if en else "none"


def _characters(features: dict[str, float], label: str, text: str, direction: str) -> None:
    if not text:
        return
    padded = "^" + text + "$"
    for order in ACTION_FEATURE_NGRAM_ORDERS:
        count = len(padded) - order + 1
        for index in range(max(0, count)):
            name = f"{label}:char:{direction}:{order}:{padded[index:index + order]}"
            features[name] = min(
                ACTION_FEATURE_CHARACTER_WEIGHT_CAP, features.get(name, 0.0) + 1.0 / math.sqrt(count)
            )


def _word_score(features: dict[str, float], label: str, score: WordScore | None, known: bool) -> None:
    features[f"{label}:known:{int(known)}"] = 1.0
    if score is None:
        features[f"{label}:score_missing"] = 1.0
        return
    if type(score.frequency) is not int or score.frequency < 0:
        raise ValueError("invalid context action frequency")
    features.update({
        f"{label}:value": _bounded(score.value, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND,
        f"{label}:gram_ratio": _bounded(score.gram_ratio, 0.0, 1.0),
        f"{label}:ngram_score": (
            _bounded(score.ngram_score, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND
        ),
        f"{label}:invalid_ratio": _bounded(score.invalid_ratio, 0.0, 1.0),
        f"{label}:raw_ngram_score": (
            _bounded(score.raw_ngram_score, -ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND, 0.0) / ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND
        ),
        f"{label}:logfrequency": (
            math.log1p(max(0, min(ACTION_FEATURE_FREQUENCY_CAP, score.frequency))) / math.log1p(ACTION_FEATURE_FREQUENCY_CAP)
        ),
        f"{label}:exact:{int(score.exact)}": 1.0,
        f"{label}:spell_known:{int(score.spell_known)}": 1.0,
    })


def extract_action_features(item: ContextEvidence) -> dict[str, float]:
    """Describe each reading separately; no token-specific conversion rules."""

    if type(item.source_group) is not int or item.source_group not in (0, 1):
        raise ValueError("invalid context action direction")
    origin = item.after_origin
    if origin not in ("none", "field", "planned_next_conversion"):
        raise ValueError("invalid right-context origin")
    if origin == "planned_next_conversion" and (
        not 0 < len(item.original) <= PLANNED_CONTEXT_WORD_MAX_CHARACTERS
        or item.trigger != "space" or item.boundary_text != " "
        or not item.field.after or len(item.field.after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
        or any(char.isspace() for char in item.field.after)
    ):
        raise ValueError("unreachable planned right context")
    direction = str(item.source_group)
    length = min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS, max(len(item.original), len(item.alternative)))
    features: dict[str, float] = {
        "bias": 1.0, f"direction:{direction}": 1.0,
        f"baseline:{int(item.baseline_convert)}": 1.0,
        f"length:{length}": 1.0,
        f"baseline:{int(item.baseline_convert)}:length:{length}": 1.0,
        f"known:{int(item.source_known)}:{int(item.target_known)}:length:{length}": 1.0,
        f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}": 1.0,
        f"role:{item.field.role[:ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS]}": 1.0,
        f"trigger:{item.trigger[:ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS]}": 1.0,
        "score_delta": _bounded(item.score_delta, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND,
        f"after_origin:{origin}": 1.0,
        f"after_origin:{origin}:direction:{direction}:length:{length}": 1.0,
        f"after_origin:{origin}:script:{_script(item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS].casefold())}"
        f":direction:{direction}:length:{length}": 1.0,
    }
    if item.source_identifier or item.target_identifier:
        # Only an identifier reading carries new information; without one these
        # combinations would merely duplicate the length, direction and
        # left-context evidence and double their learned weight.
        prefix = f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}"
        features[f"{prefix}:direction:{direction}"] = 1.0
        features[f"{prefix}:length:{length}"] = 1.0
        features[f"{prefix}:before:{_script(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold())}"] = 1.0
    source_case = "none"
    for label, raw, score, known, identifier in (
        ("source", item.original, item.source_score, item.source_known, item.source_identifier),
        ("target", item.alternative, item.target_score, item.target_known, item.target_identifier),
    ):
        raw = raw[:ACTION_FEATURE_WORD_MAX_CHARACTERS]
        text = unicodedata.normalize("NFC", raw.casefold())[:ACTION_FEATURE_WORD_MAX_CHARACTERS]
        _characters(features, label, text, direction)
        _word_score(features, label, score, known)
        features[f"{label}:identifier:{int(identifier)}"] = 1.0
        letters = "".join(char for char in raw if char.isalpha())
        case = "none" if not letters else "upper" if letters.isupper() else "lower" if letters.islower() else "title" if letters.istitle() else "mixed"
        if label == "source":
            source_case = case
        features[f"{label}:case:{case}"] = 1.0
        features[f"{label}:script:{_script(text)}"] = 1.0
        features[f"{label}:length"] = len(text) / ACTION_FEATURE_WORD_MAX_CHARACTERS
        features[f"{label}:digits"] = sum(char.isdigit() for char in raw) / max(1, len(raw))
        for mark in ",.;[]'’`-_/@\\=<>\"":
            features[f"{label}:mark:{ord(mark):04x}"] = raw.count(mark) / max(1, len(raw))
    for label, raw in (
        ("before", item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:]),
        ("after", item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS]),
    ):
        text = unicodedata.normalize("NFC", raw.casefold())
        script = _script(text)
        features[f"{label}:script:{script}:direction:{direction}"] = 1.0
        features[f"{label}:script:{script}:direction:{direction}:length:{length}"] = 1.0
        words = _WORDS.findall(text)
        neighbours = words[-ACTION_FEATURE_NEIGHBOUR_WORD_COUNT:] if label == "before" else words[:ACTION_FEATURE_NEIGHBOUR_WORD_COUNT]
        for word in neighbours:
            _characters(features, label, word[:ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS], direction)
        whitespace = raw[len(raw.rstrip()):] if label == "before" else raw[:len(raw) - len(raw.lstrip())]
        whitespace = whitespace[:ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS]
        features[f"{label}:space_count"] = len(whitespace) / ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS
        for char in sorted(set(whitespace)):
            features[f"{label}:space:{ord(char):04x}"] = whitespace.count(char) / ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS
    tail = item.literal_tail[:ACTION_FEATURE_SHORT_TEXT_CHARACTERS]
    features["tail:length"] = len(tail) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    features["tail:overflow"] = float(len(item.literal_tail) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)
    for char in sorted(set(tail)):
        features[f"tail:char:{ord(char):04x}"] = tail.count(char) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    boundary = item.boundary_text[:ACTION_FEATURE_SHORT_TEXT_CHARACTERS]
    features["boundary:length"] = len(boundary) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    features["boundary:overflow"] = float(len(item.boundary_text) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)
    features["boundary:isspace"] = float(boundary.isspace())
    for char in sorted(set(boundary)):
        features[f"boundary:char:{ord(char):04x}"] = boundary.count(char) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    application = item.field.application.casefold()[:ACTION_FEATURE_APPLICATION_NAME_CHARACTERS]
    for part in re.findall(r"[a-z0-9]+", application)[:ACTION_FEATURE_APPLICATION_TOKEN_COUNT]:
        features[f"app:{part}"] = 1.0
    for label, value in (("baseline:probability", item.model_probability), ("baseline:threshold", item.model_threshold)):
        if value is not None:
            features[label] = _bounded(value, 0.0, 1.0)
            features[label + ":present"] = 1.0
    if item.model_probability is not None and item.model_threshold is not None:
        features["baseline:margin"] = _bounded(item.model_probability - item.model_threshold)
    for label, value in (("ortho:score", item.ortho_score), ("ortho:threshold", item.ortho_threshold)):
        if value is not None:
            features[label] = _bounded(value, -ACTION_FEATURE_ORTHO_SCORE_BOUND, ACTION_FEATURE_ORTHO_SCORE_BOUND) / ACTION_FEATURE_ORTHO_SCORE_BOUND
            features[label + ":present"] = 1.0
    if item.ortho_score is not None and item.ortho_threshold is not None:
        margin = (
            _bounded(item.ortho_score - item.ortho_threshold, -ACTION_FEATURE_ORTHO_SCORE_BOUND, ACTION_FEATURE_ORTHO_SCORE_BOUND)
            / ACTION_FEATURE_ORTHO_SCORE_BOUND
        )
        features["ortho:margin"] = margin
        # The threshold belongs to the frozen orthographic model. Its side is
        # evidence with learned coefficients, alongside the continuous margin.
        side = "positive" if margin > 0 else "negative" if margin < 0 else "zero"
        support = f"direction:{direction}:source_known:{int(item.source_known)}"
        features[f"ortho:side:{side}:{support}"] = 1.0
        features[f"ortho:side:{side}:{support}:case:{source_case}"] = 1.0
        context_script = _script(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold())
        category = f"direction:{direction}:source_known:{int(item.source_known)}:context:{context_script}"
        # Orthographic evidence may differ by language and lexical support.
        # Both slopes are learned; neither licenses a conversion by itself.
        for sign, value in (("positive", max(0.0, margin)), ("negative", min(0.0, margin))):
            features[f"ortho:{sign}:direction:{direction}"] = value
            features[f"ortho:{sign}:{category}"] = value
    return {name: value for name, value in features.items() if value}

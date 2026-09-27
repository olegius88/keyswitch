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

An artifact of schema 2 adds a few token features to that score, each with a
weight in nats per direction: whether the Latin reading is a known command
(`identifiers.json`), whether the token repeats itself around a hyphen (`у-у`,
`мда-а`), whether both halves of a hyphenated reading are dictionary words
(`Я-то`), and whether the Latin reading is a dot and a dictionary word (`.dist`).
They are computed here, from the keys and the layout the policy already passes,
with the packaged identifier list and the engine's own dictionaries - the policy
hands over the detector's models, and a caller that does not is served by models
this module loads the same way - so the trainer and the runtime compute the same
thing (`token_features`). Schema 1 artifacts load and score exactly as before.

A schema 3 artifact adds one more feature, `extra_key`: the reading in the target language is no
word of its dictionary but becomes one without one of its letters - a stray or stuck key before or
inside a word (`фпривет` is `привет`). Only readings of at least CONTEXT_TYPO_MIN_CHARACTERS letters
are checked, as for the typo evidence of the context model.

A schema 4 artifact adds `stretch_runs`: a run of at least that many identical keys is a stretch
(`муууу`, `каеффф`) or a stuck key whose length in the word is unknown - one letter or two. Cutting it to
two inserts a double letter, and a double letter is evidence of a language (`ee`, `ff`, `ss` are common
English, `уу`, `аа` are rare Russian) that a stretch does not carry. The model therefore reads the token
both ways - with the run cut as `collapse_runs` cuts it and with every such run read once - and judges
each reading whole, its characters and its features: a stretch is no stray key, so `штттук` read once is
`штук`, not `inner` with one letter too many. It keeps the reading that argues less for the other
layout: when the spelling is ambiguous, the reading that protects the text wins. Schema 4 also weighs
`source_word`: the reading in the layout typed is a word of its dictionary. The engine never asks the
model about such a word, so the feature is 0 on a token as typed and fires only on a reading without
its stretch - `Ууувы` is `Увы`, whose keys `eds` read English enough to be converted otherwise.

A schema 5 artifact also reads a hyphen between two copies of one key as a drawn-out sound (`Ти-ише`),
lets compound evidence on both sides cancel (`В-к` / `D-r`), may require of a compound in a script a half of
some length (`compound_min_letters`: the English dictionary knows every letter and most two-letter strings, so
`r-ut` is no compound of English words), weighs whether the replacement is a word
(`target_unknown`, and `name_unknown` for a capitalised token) and how word-like the reading is in the
layout typed (`source_plausibility`: the source channel's log-probability per scored position, from a
centre the artifact carries). A feature the training left unfitted carries the weight 0. A schema 6
artifact also weighs how word-like the replacement is in its own language (`target_plausibility`, the
target channel's log-probability per scored position from its own centres: `ckicc` for `слшсс` is less
English than any replacement the corpus makes) and may count a feature only for a reading less plausible
than the centre of the layout typed (`gated_features`): a stray key explains noise, not a word of the layout
typed (`чегту` read as `xtune`).

A schema 2 artifact may also carry a second channel for a script, used only when that script is
the source of the evidence (`source_models`). Asking whether a Cyrillic token is Russian after all
may draw on a broad vocabulary of the web - slang on a borrowed stem, the name of a layout - which
the curated word lists lack; asking whether Latin keys were meant as Russian may not, because every
odd sequence such a vocabulary admits would then argue for turning English into Russian.

This module only scores. It never decides, never injects and never overrides an
explicit user rule; the policy layer applies it.
"""

from __future__ import annotations

import json
import unicodedata
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

from .constants.detection import ACRONYM_MIN_LETTERS
from .constants.models import CONTEXT_TYPO_MIN_CHARACTERS
from .constants.ortho import (
    ORTHO_ARTIFACT_MAX_BYTES as MAX_ARTIFACT_BYTES,
    ORTHO_BACKOFF_ENTRY_FIELDS,
    ORTHO_CHANNEL_MAX_GRAMS as MAX_GRAMS,
    ORTHO_COMPOUND_PARTS,
    ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION,
    ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS,
    ORTHO_MAX_ORDER as MAX_ORDER,
    ORTHO_MAX_SCALE,
    ORTHO_MIN_COLLAPSED_RUN,
    ORTHO_MIN_ORDER as MIN_ORDER,
    ORTHO_MIN_STRETCH_RUN,
    ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
    ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION,
)
from .identifier_lexicon import IdentifierLexicon
from .language_model import LanguageModel
from .layouts import LayoutPair
from .lexicon_supplement import supplement_words

# MAX_ARTIFACT_BYTES, MAX_GRAMS, MIN_ORDER and MAX_ORDER keep the names this module has always
# exported; their values live in the constants.
__all__ = [
    "ARTIFACT_PATH", "BOS", "CAPITALISED", "CONTINUOUS_FEATURE", "EOS", "FEATURES", "FEATURES_SCHEMA_2", "FEATURES_SCHEMA_3", "FEATURES_SCHEMA_4",
    "FEATURES_SCHEMA_5", "TARGET_FEATURE",
    "LOCALES", "MAX_ARTIFACT_BYTES", "MAX_GRAMS", "MAX_ORDER",
    "MIN_ORDER", "SCHEMA_FEATURES", "SCRIPTS", "SHAPES", "Identifier", "KnownWord", "OrthoEvidence", "OrthoModel",
    "OrthoScore", "collapse_runs", "drop_hyphen_stretches", "one_key_extra", "read_once", "readings", "reduplicated",
    "shape_of", "stretch_readings", "token_features",
]

ARTIFACT_PATH: Final[Path] = Path(__file__).parent / "resources" / "models" / "ortho_v1.json"
SCRIPTS: Final[tuple[str, str]] = ("en", "ru")
SHAPES: Final[tuple[str, ...]] = ("lower", "initial", "inner", "upper")
BOS: Final[str] = "\x02"
EOS: Final[str] = "\x03"
# The token features a schema-2 artifact weighs, in the order the trainer fits them.
FEATURES_SCHEMA_2: Final[tuple[str, ...]] = ("ident", "repeat", "parts_source", "parts_target", "dot_word")
FEATURES_SCHEMA_3: Final[tuple[str, ...]] = (*FEATURES_SCHEMA_2, "extra_key")
FEATURES_SCHEMA_4: Final[tuple[str, ...]] = (*FEATURES_SCHEMA_3, "source_word")
FEATURES_SCHEMA_5: Final[tuple[str, ...]] = (*FEATURES_SCHEMA_4, "name_unknown", "target_unknown", "source_plausibility")
FEATURES: Final[tuple[str, ...]] = (*FEATURES_SCHEMA_5, "target_plausibility")
# The features that are not indicators: how word-like the reading is in the layout typed - the source
# channel's log-probability per scored position - and how word-like the replacement is in its own
# language - the target channel's -, each measured from the centre the artifact carries.
CONTINUOUS_FEATURE: Final[str] = "source_plausibility"
TARGET_FEATURE: Final[str] = "target_plausibility"
# The features an artifact of each schema weighs.
SCHEMA_FEATURES: Final[dict[int, tuple[str, ...]]] = {
    ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION: FEATURES_SCHEMA_2,
    ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION: FEATURES_SCHEMA_3,
    ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION: FEATURES_SCHEMA_4,
    ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION: FEATURES_SCHEMA_5,
    ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION: FEATURES,
}
# The case shapes of a token written with a capital letter as a word, not as an abbreviation.
CAPITALISED: Final[frozenset[str]] = frozenset({"initial", "inner"})
# The dictionaries the engine loads for the two scripts (KeySwitchEngine: LanguageModel.load(locale,
# supplement_words(locale))).
LOCALES: Final[dict[str, str]] = {"en": "en_US", "ru": "ru_RU"}
_PAIR: Final[LayoutPair] = LayoutPair()
Identifier = Callable[[str], bool]
KnownWord = Callable[[str, str], bool]


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
    features: float = 0.0
    # The keys the features read: the token as typed, or with a schema 4 artifact the reading kept.
    reading: str = ""


def readings(keys: str, source_script: str) -> tuple[str, str]:
    """The token as typed and as it reads in the other layout; the keys are the Latin reading."""

    cyrillic = _PAIR.translate(keys, "us", "ru")
    return (keys, cyrillic) if source_script == "en" else (cyrillic, keys)


def reduplicated(text: str) -> bool:
    """A token that repeats itself around one hyphen: `у-у`, `мда-а`, `ну-ну`."""

    parts = text.split("-")
    if len(parts) != ORTHO_COMPOUND_PARTS or not all(parts):
        return False
    head, tail = parts
    return head.endswith(tail) or tail in head or head[-1] == tail[-1]


def collapse_runs(text: str, limit: int) -> str:
    """The text with every run of one character longer than `limit` cut to `limit`: `каеффф` → `каефф`."""

    result: list[str] = []
    run = 0
    for character in text:
        run = run + 1 if result and character == result[-1] else 1
        if run <= limit:
            result.append(character)
    return "".join(result)


def read_once(text: str, minimum: int) -> str:
    """The text with every run of at least `minimum` identical characters read once: `муууу` → `му`."""

    result: list[str] = []
    start = 0
    for index in range(1, len(text) + 1):
        if index == len(text) or text[index] != text[start]:
            length = index - start
            result.append(text[start] if length >= minimum else text[start:index])
            start = index
    return "".join(result)


def drop_hyphen_stretches(text: str) -> str:
    """The text without every hyphen that stands between two copies of one character, and without the
    second copy: a drawn-out sound (`ти-ише` → `тише`, `ура-а-а` → `ура`), not a boundary of parts."""

    result: list[str] = []
    index = 0
    while index < len(text):
        stretched = (text[index] == "-" and bool(result) and result[-1] != "-" and index + 1 < len(text)
                     and text[index + 1] == result[-1])
        if not stretched:
            result.append(text[index])
        index += 1 + int(stretched)
    return "".join(result)


def stretch_readings(keys: str, collapse: int | None, stretch: int | None, hyphens: bool = False) -> tuple[str, ...]:
    """The readings of a token the character models judge: as typed with a stretch cut to `collapse`
    and, with `stretch`, also with every run of at least `stretch` keys read once - with `hyphens`,
    and every hyphen stretch dropped (`drop_hyphen_stretches`)."""

    typed = keys if collapse is None else collapse_runs(keys, collapse)
    if stretch is None:
        return (typed,)
    once = read_once(drop_hyphen_stretches(keys) if hyphens else keys, stretch)
    once = once if collapse is None else collapse_runs(once, collapse)
    return (typed,) if once == typed else (typed, once)


def _compound_known(text: str, script: str, known: KnownWord, min_letters: int = 0) -> bool:
    parts = text.split("-")
    return (len(parts) == ORTHO_COMPOUND_PARTS and all(parts) and max(len(part) for part in parts) >= min_letters
            and all(known(script, part) for part in parts))


def one_key_extra(text: str, script: str, known: KnownWord) -> bool:
    """A reading that is no word of the dictionary but becomes one without one of its letters."""

    if len(text) < CONTEXT_TYPO_MIN_CHARACTERS or not text.isalpha() or known(script, text):
        return False
    return any(known(script, text[:index] + text[index + 1:]) for index in range(len(text)))


def token_features(keys: str, source_script: str, *, identifier: Identifier,
                   known: KnownWord, shape: str = "lower", symmetric: bool = False,
                   compound_min_letters: Mapping[str, int] | None = None) -> dict[str, int]:
    """The feature values of one token, from its keys, the layout it was typed in and its case shape.

    `identifier` answers whether a Latin string is a known command, `known` whether a word is in
    the dictionary of a script. The trainer passes the frozen answers, the runtime the packaged
    list and the engine's dictionaries. With `symmetric` (schema 5) a token whose readings on both
    sides are compounds of dictionary words (`В-к` is `D-r`) gives no compound evidence either way.
    `compound_min_letters` (schema 5) names, per script, how long the longer half of a compound of that
    script's words must be: two halves that short are words of its dictionary by chance.
    """

    target_script = "ru" if source_script == "en" else "en"
    source, target = readings(keys, source_script)
    unknown = target.isalpha() and not known(target_script, target)
    minimum = compound_min_letters or {}
    parts_source = _compound_known(source, source_script, known, minimum.get(source_script, 0))
    parts_target = _compound_known(target, target_script, known, minimum.get(target_script, 0))
    if symmetric and parts_source and parts_target:
        parts_source = parts_target = False
    return {
        "ident": int(identifier(keys)),
        "repeat": int(reduplicated(source) or reduplicated(target)),
        "parts_source": int(parts_source),
        "parts_target": int(parts_target),
        "dot_word": int(source_script == "ru" and len(keys) > 1 and keys.startswith(".")
                        and known("en", keys[1:])),
        "extra_key": int(one_key_extra(target, target_script, known)),
        "source_word": int(known(source_script, source)),
        "name_unknown": int(unknown and shape in CAPITALISED),
        "target_unknown": int(unknown),
    }


def _packaged_identifier() -> Identifier:
    lexicon, _status = IdentifierLexicon.try_load()
    return lexicon.contains if lexicon is not None else (lambda _token: False)


def _engine_dictionaries() -> KnownWord:
    """`known` as the engine answers it, for a caller that does not pass the engine's models.

    `LanguageModel.load(locale, supplement_words(locale))` returns the engine's cached models, but
    finding them normalises the whole supplement, which takes about a second; the context policy
    therefore passes the detector's models with every score, and only tools reach this.
    """

    models = {script: LanguageModel.load(locale, supplement_words(locale)) for script, locale in LOCALES.items()}
    return lambda script, word: models[script].score(word).known


def shape_of(token: str, first_in_field: bool) -> str:
    """The case shape of a token as typed, in the vocabulary the model counts."""

    letters = [character for character in token if character.isalpha()]
    if len(letters) >= ACRONYM_MIN_LETTERS and all(character.isupper() for character in letters):
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
                 thresholds: dict[str, float], minimum_length: int,
                 features: dict[str, dict[str, float]] | None = None, *,
                 identifier: Identifier | None = None, known: KnownWord | None = None,
                 collapse: int | None = None, unscored_shapes: frozenset[str] = frozenset(),
                 source_channels: dict[str, _Channel] | None = None, stretch: int | None = None,
                 hyphens: bool = False, centres: dict[str, float] | None = None,
                 compound_min_letters: dict[str, int] | None = None, target_centres: dict[str, float] | None = None,
                 gated: tuple[str, ...] = ()) -> None:
        self.order = order
        self.channels = channels
        # The channel a script is scored with when the token was typed in it, if it differs.
        self.source_channels = source_channels or {}
        self.prose_shape = prose_shape
        self.acronym_shape = acronym_shape
        self.version = version
        self.thresholds = thresholds
        self.minimum_length = minimum_length
        self.features = features or {}
        # A schema 2 artifact may judge a stretched token by its word (`collapse`) and may declare
        # case shapes it does not score at all (`unscored_shapes`), as it does not score tokens
        # shorter than `minimum_length`.
        self.collapse = collapse
        self.unscored_shapes = unscored_shapes
        # A schema 4 artifact also reads every run of at least `stretch` keys once (`read_once`); a
        # schema 5 artifact also drops hyphen stretches in that reading (`drop_hyphen_stretches`).
        self.stretch = stretch
        self.hyphens = hyphens
        # A schema 5 artifact measures `source_plausibility` from these centres (per source script), and may
        # require of a compound in a script a half of some length (`compound_min_letters`).
        self.centres = centres or {}
        self.compound_min_letters = compound_min_letters or {}
        # A schema 6 artifact also measures `target_plausibility` from these centres (per source script), and may
        # count a feature only for a reading less plausible than the centre of the layout typed (`gated`).
        self.target_centres = target_centres or {}
        self.gated = gated
        self._identifier = identifier
        self._known = known

    @classmethod
    def load(cls, path: Path = ARTIFACT_PATH, *, identifier: Identifier | None = None,
             known: KnownWord | None = None) -> OrthoModel:
        with path.open("rb") as handle:
            content = handle.read(MAX_ARTIFACT_BYTES + 1)
        if len(content) > MAX_ARTIFACT_BYTES:
            raise ValueError("oversized orthotactic artifact")
        payload: object = json.loads(content)
        if not isinstance(payload, dict) or payload.get("schema_version") not in (1, *SCHEMA_FEATURES):
            raise ValueError("unsupported orthotactic artifact")
        order = payload.get("order")
        version = payload.get("version")
        scale = payload.get("scale")
        if (not isinstance(order, int) or not MIN_ORDER <= order <= MAX_ORDER
                or not isinstance(version, str) or not version.startswith("ortho-v1-")
                or not isinstance(scale, int) or not 1 <= scale <= ORTHO_MAX_SCALE):
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
                or not isinstance(minimum_length, int)
                or not 1 <= minimum_length <= ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS):
            raise ValueError("orthotactic artifact carries no usable decision threshold")
        thresholds: dict[str, float] = {}
        for script in SCRIPTS:
            value = raw_thresholds[script]
            if not isinstance(value, int):
                raise ValueError("invalid orthotactic threshold")
            thresholds[script] = value / scale
        names = SCHEMA_FEATURES.get(payload["schema_version"])
        featured = names is not None
        features = cls._features(payload.get("features"), scale, names) if names is not None else None
        if not featured and any(name in payload for name in ("features", "collapse_runs", "unscored_shapes",
                                                              "source_models")):
            raise ValueError("a schema 1 orthotactic artifact carries no features")
        raw_sources = payload.get("source_models", {})
        if not isinstance(raw_sources, dict) or not set(raw_sources) <= set(SCRIPTS):
            raise ValueError("invalid orthotactic source channels")
        sources = {script: cls._channel(raw, order, scale) for script, raw in raw_sources.items()}
        collapse = payload.get("collapse_runs")
        if collapse is not None and (type(collapse) is not int or collapse < ORTHO_MIN_COLLAPSED_RUN):
            raise ValueError("invalid orthotactic run collapse")
        unscored = payload.get("unscored_shapes", [])
        if not isinstance(unscored, list) or not set(unscored) <= set(SHAPES):
            raise ValueError("invalid orthotactic shape scope")
        stretch = payload.get("stretch_runs")
        if (payload["schema_version"] >= ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION) != (stretch is not None):
            raise ValueError("only a schema 4 or 5 orthotactic artifact reads stretches, and it must")
        if stretch is not None and (type(stretch) is not int or stretch < ORTHO_MIN_STRETCH_RUN):
            raise ValueError("invalid orthotactic stretch run")
        return cls(order, channels, cls._shape(payload.get("prose_shape"), scale),
                   cls._shape(payload.get("acronym_shape"), scale), version,
                   thresholds, minimum_length, features, identifier=identifier, known=known,
                   collapse=collapse, unscored_shapes=frozenset(unscored), source_channels=sources,
                   stretch=stretch, hyphens=payload["schema_version"] >= ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
                   centres=cls._centres(payload, scale), compound_min_letters=cls._compound_minimum(payload),
                   target_centres=cls._centres(payload, scale, "target_centres", ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION),
                   gated=cls._gated(payload))

    @staticmethod
    def _features(raw: object, scale: int, names: tuple[str, ...]) -> dict[str, dict[str, float]]:
        if not isinstance(raw, dict) or set(raw) != set(SCRIPTS):
            raise ValueError("orthotactic artifact must weigh its features for both scripts")
        result: dict[str, dict[str, float]] = {}
        for script in SCRIPTS:
            table = raw[script]
            if not isinstance(table, dict) or set(table) != set(names):
                raise ValueError("invalid orthotactic features")
            weights: dict[str, float] = {}
            for name in names:
                value = table[name]
                if not isinstance(value, int):
                    raise ValueError("invalid orthotactic feature weight")
                weights[name] = value / scale
            result[script] = weights
        return result

    @staticmethod
    def _compound_minimum(payload: dict[str, object]) -> dict[str, int]:
        """The length a schema 5 artifact requires of the longer half of a compound, per script."""

        raw = payload.get("compound_min_letters", {})
        if (not isinstance(raw, dict) or not set(raw) <= set(SCRIPTS)
                or not all(type(value) is int and value >= 1 for value in raw.values())
                or (raw and cast(int, payload["schema_version"]) < ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION)):
            raise ValueError("invalid orthotactic compound minimum")
        return {script: raw[script] for script in sorted(raw)}

    @staticmethod
    def _gated(payload: dict[str, object]) -> tuple[str, ...]:
        """Features a schema 6 artifact counts only for a token less plausible than the centre of its layout."""

        raw = payload.get("gated_features", [])
        if (not isinstance(raw, list) or not set(raw) <= set(FEATURES_SCHEMA_4)
                or (raw and payload["schema_version"] != ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION)):
            raise ValueError("invalid orthotactic gated features")
        return tuple(sorted(raw))

    @staticmethod
    def _centres(payload: dict[str, object], scale: int, field: str = "feature_centres",
                 since: int = ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION) -> dict[str, float]:
        """Centres of a plausibility (`feature_centres` of the source channel from schema 5, `target_centres` of the
        target channel from schema 6): required from that schema on, absent before."""

        raw = payload.get(field)
        if (cast(int, payload["schema_version"]) >= since) != (raw is not None):
            raise ValueError(f"an orthotactic artifact of schema {since} or later carries {field}, and no other")
        if raw is None:
            return {}
        if not isinstance(raw, dict) or set(raw) != set(SCRIPTS) or not all(type(value) is int for value in raw.values()):
            raise ValueError("invalid orthotactic feature centres")
        return {script: raw[script] / scale for script in SCRIPTS}

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
            if not isinstance(entry, list) or len(entry) != ORTHO_BACKOFF_ENTRY_FIELDS:
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

    def feature_values(self, keys: str, source_script: str, known: KnownWord | None = None,
                       shape: str = "lower") -> dict[str, float]:
        """The token features, from the packaged identifier list and the dictionaries passed in.

        Without `known` the model uses the dictionaries it was loaded with, or loads the engine's.
        """

        if self._identifier is None:
            self._identifier = _packaged_identifier()
        if known is None:
            if self._known is None:
                self._known = _engine_dictionaries()
            known = self._known
        values: dict[str, float] = dict(token_features(keys, source_script, identifier=self._identifier, known=known,
                                                        shape=shape, symmetric=self.hyphens,
                                                        compound_min_letters=self.compound_min_letters))
        return values

    @classmethod
    def try_load(cls, path: Path = ARTIFACT_PATH) -> tuple[OrthoModel | None, str]:
        try:
            model = cls.load(path)
        except (OSError, ValueError, TypeError) as error:
            return None, f"unavailable: {type(error).__name__}"
        return model, model.version

    def score(self, evidence: OrthoEvidence, *, known: KnownWord | None = None) -> OrthoScore:
        """Evidence, in nats, that the keys were meant for the other layout.

        `known` answers the dictionary questions of the features (the context policy passes the
        detector's models); a schema 1 artifact asks none.
        """

        source = evidence.source_script
        target = "ru" if source == "en" else "en"
        shape = evidence.shape if evidence.shape in SHAPES else "lower"
        if source not in SCRIPTS or not evidence.keys or shape in self.unscored_shapes:
            return OrthoScore(0.0, 0.0, 0.0, 0.0, 0.0, False)
        keys = unicodedata.normalize("NFC", evidence.keys).casefold()
        source_channel = self.source_channels.get(source, self.channels[source])
        target_channel = self.channels[target]
        # An abbreviation is written in capitals. A lowercase token therefore
        # cannot be excused as one, which is what removes the dominant class of
        # false conversions: РСФСР, ЛДПР, РНК all read as fluent English keys.
        source_acronym = self.acronym_shape[source][shape] - self.prose_shape[source][shape]
        shape_ratio = -source_acronym
        # Up to schema 3 the one reading is the token as typed, a stretch cut as `collapse` cuts it,
        # and the features read the keys as typed. A schema 4 artifact reads a stretch both ways and
        # judges each reading whole - characters and features - keeping the one that argues less for
        # the other layout: when the spelling is ambiguous, the reading that protects the text wins.
        judged = [self._judged(keys if self.stretch is None else letters, source_channel.score(letters),
                               target_channel.score(letters), shape_ratio, source, known, shape)
                  for letters in stretch_readings(keys, self.collapse, self.stretch, self.hyphens)]
        return min(judged, key=lambda scored: scored.total)

    def _judged(self, featured: str, source_logprob: float, target_logprob: float,
                shape_ratio: float, source: str, known: KnownWord | None, shape: str) -> OrthoScore:
        """One reading scored: its character ratio, the case shape and the features of `featured`."""

        ratio = target_logprob - source_logprob
        features = 0.0
        if self.features:
            values = self.feature_values(featured, source, known, shape)
            if self.centres:
                values[CONTINUOUS_FEATURE] = source_logprob / (len(featured) + 1) - self.centres[source]
                # A gated feature is evidence only for a reading that is noise in the layout typed.
                for name in self.gated:
                    if values[CONTINUOUS_FEATURE] >= 0:
                        values[name] = 0
            if self.target_centres:
                values[TARGET_FEATURE] = target_logprob / (len(featured) + 1) - self.target_centres[source]
            features = sum(weight * values[name] for name, weight in self.features[source].items())
        return OrthoScore(ratio, shape_ratio, ratio + shape_ratio + features,
                          source_logprob, target_logprob, True, features, featured)


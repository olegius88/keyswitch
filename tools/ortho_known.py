#!/usr/bin/env python3
"""Freeze whether the runtime's dictionaries know each token the engine can serve the ortho model.

`ContextPolicy._orthotactic` never asks this model about a token the dictionary of the layout in
use knows (`baseline.source_score.known`), so such a token is not part of the population the model
is measured on. "Known" is whatever `LanguageModel.score(...).known` answers for the models the
engine loads (`LanguageModel.load(locale, supplement_words(locale))`): the frozen frequency list,
the packaged supplement, or Hunspell.

Hunspell cannot be called during a replay of the trainer: dictionaries differ between machines.
So the verdict is computed once, here, with the reference word lists and dictionaries whose digests
Layout Intent v1 records (`model/intent_v1/sources`, `model/intent_v1/config.json`), and checked in;
the trainer reads the answer, and only this tool needs a speller. Run it with
``KEYSWITCH_MODEL_PATH=model/intent_v1/sources KEYSWITCH_HUNSPELL_PATH=model/intent_v1/sources/hunspell``;
it refuses any other dictionary.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import sys
from collections.abc import Sequence
from itertools import groupby
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_corpus  # noqa: E402

from keyswitch.constants.file_formats import FROZEN_CORPUS_RECEIPT_JSON_INDENT  # noqa: E402
from keyswitch.constants.models import CONTEXT_TYPO_MIN_CHARACTERS  # noqa: E402
from keyswitch.language_model import LanguageModel  # noqa: E402
from keyswitch.lexicon_supplement import SUPPLEMENT_ROOT, supplement_words  # noqa: E402
from keyswitch.constants.ortho import ORTHO_MIN_COLLAPSED_RUN, ORTHO_MIN_STRETCH_RUN  # noqa: E402
from keyswitch.ortho_model import readings, stretch_readings  # noqa: E402
from keyswitch.value_provenance import CONSTANTS_PACKAGE, pin_values  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
EVIDENCE: Final[Path] = ortho_corpus.DIRECTORY / "known-evidence.json.gz"
RECEIPT: Final[Path] = ortho_corpus.DIRECTORY / "known-receipt.json"
INTENT_CONFIG: Final[Path] = ROOT / "model/intent_v1/config.json"
LOCALES: Final[dict[str, str]] = {"en": "en_US", "ru": "ru_RU"}
CODE: Final[tuple[str, ...]] = ("tools/ortho_known.py",)


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def recorded_dictionaries() -> dict[str, dict[str, object]]:
    """The Hunspell digests Layout Intent v1 was certified against."""

    payload = cast(dict[str, object], json.loads(INTENT_CONFIG.read_bytes()))
    external = cast(dict[str, object], payload["external_evaluation"])
    recorded = cast(dict[str, object], external["hunspell"])
    return {locale: cast(dict[str, object], recorded[locale]) for locale in sorted(LOCALES.values())}


def supplement_paths() -> tuple[Path, ...]:
    return tuple(path for locale in sorted(LOCALES.values())
                 if (path := SUPPLEMENT_ROOT / f"lexicon-supplement-{locale}.json").is_file())


def reference_models() -> dict[str, LanguageModel]:
    """The engine's models, refused unless they read exactly the reference word lists and dictionaries."""

    models = {script: LanguageModel.load(locale, supplement_words(locale)) for script, locale in LOCALES.items()}
    recorded = recorded_dictionaries()
    for script, model in models.items():
        path, expected = ortho_corpus.LEXICONS[script]
        frequency_source = model.source.split(";")[0]
        if not Path(frequency_source).is_file() or checksum(Path(frequency_source)) != expected:
            raise ValueError(f"the {script} word list is not the reference {path}; set KEYSWITCH_MODEL_PATH")
        dictionary = recorded[LOCALES[script]]
        speller_source = getattr(model.speller, "source", "")
        if not model.speller.available or not speller_source:
            raise ValueError(f"no {script} speller: refusing to freeze a partial verdict")
        dic = Path(speller_source)
        if checksum(dic) != dictionary["dictionary_sha256"] or checksum(dic.with_suffix(".aff")) != dictionary["affix_sha256"]:
            raise ValueError(f"the {script} Hunspell dictionary is not the reference; set KEYSWITCH_HUNSPELL_PATH")
    return models


def queried(keys: str, direction: str) -> dict[str, set[str]]:
    """Every word the population filter and the token features ask a dictionary about for one
    served token: the token as typed, both halves of a hyphenated reading in its own script, the
    word after a leading dot of the Latin reading, and for `extra_key` the reading in the other
    script with each of its variants without one letter (`ortho_model.token_features`), the reading in
    the other script itself (is the replacement a word) - for the token as typed and for every reading
    of its runs a schema 4 or 5 artifact may judge (any run of at least two keys read once, with and
    without hyphen stretches, `ortho_model.stretch_readings`), whatever `stretch_runs` it carries."""

    longest = max((len(list(run)) for _key, run in groupby(keys)), default=0)
    variants = {keys, *(reading for minimum in range(ORTHO_MIN_STRETCH_RUN, max(longest, ORTHO_MIN_STRETCH_RUN) + 1)
                        for hyphens in (False, True)
                        for reading in stretch_readings(keys, ORTHO_MIN_COLLAPSED_RUN, minimum, hyphens))}
    result: dict[str, set[str]] = {script: set() for script in LOCALES}
    for variant in sorted(variants):
        for script, words in _asked(variant, direction).items():
            result[script].update(words)
    return result


def _asked(keys: str, direction: str) -> dict[str, set[str]]:
    result: dict[str, set[str]] = {script: set() for script in LOCALES}
    other = ortho_corpus.other(direction)
    source, target = readings(keys, direction)
    result[direction].add(ortho_corpus.visible(keys, direction))
    for text, script in ((source, direction), (target, other)):
        if "-" in text:
            result[script].update(part for part in text.split("-") if part)
    if direction == "ru" and len(keys) > 1 and keys.startswith("."):
        result["en"].add(keys[1:])
    if target.isalpha():
        # Whether the replacement is a word (`target_unknown`, `name_unknown`).
        result[other].add(target)
    if len(target) >= CONTEXT_TYPO_MIN_CHARACTERS and target.isalpha():
        result[other].update(target[:index] + target[index + 1:] for index in range(len(target)))
    return result


def forms() -> dict[str, set[str]]:
    """Every word asked about for every token the engine can serve the model, per script."""

    result: dict[str, set[str]] = {script: set() for script in LOCALES}
    for row in ortho_corpus.load_tokens():
        if row["split"] == ortho_corpus.LEXICON:
            continue
        keys = str(row["keys"])
        # The synthetic stray-key variant of the word, typed in the other layout, is asked about too
        # (`train_ortho_model.synthetic_population`); its draw does not depend on the split.
        synthetic = ortho_corpus.extra_key_variant(str(row["script"]), keys)
        typed = [(keys, direction) for direction in LOCALES]
        if synthetic is not None:
            typed.append((synthetic, ortho_corpus.other(str(row["script"]))))
        for text, direction in typed:
            for served in ortho_corpus.served(text, direction):
                if set(served) <= ortho_corpus.KEYS:
                    for script, words in queried(served, direction).items():
                        result[script].update(words)
    return result


def build() -> dict[str, list[str]]:
    models = reference_models()
    return {script: sorted(word for word in words if models[script].score(word).known)
            for script, words in sorted(forms().items())}


def write(payload: dict[str, list[str]], path: Path) -> str:
    # Only the known forms are stored: the answer for anything absent is "not known".
    raw = (json.dumps(payload, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def load(path: Path = EVIDENCE) -> dict[str, frozenset[str]]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        payload = json.load(handle)
    return {str(script): frozenset(map(str, words)) for script, words in payload.items()}


def provenance() -> dict[str, str]:
    code = tuple(ROOT / name for name in CODE)
    # The Hunspell digests are recorded in the receipt itself; the intent configuration they come
    # from changes with every intent generation, so its bytes are not pinned.
    paths = (*code, ortho_corpus.TOKENS, *supplement_paths())
    pins = {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}
    pins[CONSTANTS_PACKAGE] = pin_values(code, source_root=ROOT / "src").sha256
    return pins


def receipt(payload: dict[str, list[str]], evidence_sha: str) -> dict[str, object]:
    return {
        "schema_version": 1,
        "description": (
            "Whether the dictionary of each layout knows each token the engine can serve the "
            "orthotactic model, each half of a hyphenated reading and the word after a leading dot, "
            "frozen so a replay needs no speller. Absent means not known."
        ),
        "evidence_sha256": evidence_sha,
        "tokens_sha256": checksum(ortho_corpus.TOKENS),
        "hunspell": recorded_dictionaries(),
        "word_lists": {script: path for script, (path, _expected) in sorted(ortho_corpus.LEXICONS.items())},
        "known_by_script": {script: len(words) for script, words in sorted(payload.items())},
        "scope": (
            "LanguageModel.load(locale, supplement_words(locale)).score(token).known with the "
            "reference word lists and Hunspell dictionaries of Layout Intent v1, which is what the "
            "runtime means by `known`; a machine whose speller differs licenses more or fewer "
            "conversions than this evidence measured"
        ),
        "provenance": provenance(),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    arguments = parser.parse_args(argv)
    payload = build()
    if arguments.freeze and not EVIDENCE.exists():
        evidence_sha = write(payload, EVIDENCE)
    else:
        if not EVIDENCE.exists():
            raise ValueError("dictionary evidence is missing; run once with --freeze")
        evidence_sha = checksum(EVIDENCE)
        check = EVIDENCE.with_name(".known-check.gz")
        reproduced = write(payload, check) == evidence_sha
        check.unlink()
        if not reproduced:
            raise ValueError("dictionary evidence is not reproducible from these dictionaries")
    content = (json.dumps(receipt(payload, evidence_sha), sort_keys=True, ensure_ascii=False,
                          indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("dictionary receipt is missing or changed")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

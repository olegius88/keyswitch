"""Exact runtime inputs for historical prefix/boundary engine regressions.

These pins describe the current replay, not a new fit or an independent test.
The evaluator must compare provenance before and after replay and patch its
engine loader with ``packaged_intent`` to exclude environment model overrides.
"""
from __future__ import annotations

from collections.abc import Sequence
from functools import lru_cache
import hashlib
import json
from pathlib import Path
from typing import cast

from keyswitch.intent_model import IntentModelStatus, LinearNgramModel

INTENT_PATH = "src/keyswitch/resources/models/layout_intent_v1.ksm"
CONFIG_PATH = "model/intent_v1/config.json"
REQUIRED_PATHS = (
    "src/keyswitch/__init__.py", "src/keyswitch/engine.py",
    INTENT_PATH,
    "src/keyswitch/resources/models/context_policy_v1.json",
    "src/keyswitch/resources/models/prefix_policy_v1.json",
    "src/keyswitch/resources/models/boundary-v2.json",
    "src/keyswitch/resources/models/ortho_v1.json",
    "src/keyswitch/resources/protected_tokens.txt",
    CONFIG_PATH,
    "tools/auxiliary_runtime_evidence.py", "tools/context_corpus.py",
    "tools/context_evidence.py", "tools/context_frames.py",
    "tests/test_input_integrity.py", "tests/test_engine_behaviour.py",
    "tests/test_windows_backend.py", "tests/test_x11_backend.py",
)


def _inside(root: Path, path: Path) -> Path:
    candidate = path if path.is_absolute() else root / path
    resolved = candidate.resolve(strict=True)
    if not resolved.is_relative_to(root) or not candidate.is_relative_to(root):
        raise ValueError("runtime input escapes repository: " + str(path))
    if ".." in candidate.relative_to(root).parts:
        raise ValueError("noncanonical runtime input: " + str(path))
    if not resolved.is_file():
        raise ValueError("runtime input is not a file: " + str(path))
    return candidate


def _checksum(path: Path) -> str:
    before = path.stat()
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    after = path.stat()
    if (before.st_mtime_ns, before.st_size, before.st_ino) != (after.st_mtime_ns, after.st_size, after.st_ino):
        raise ValueError("runtime input changed while hashing: " + str(path))
    return digest.hexdigest()


def _object(value: object, name: str) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("invalid lexical configuration: " + name)
    return cast(dict[str, object], value)


def _lexical_paths(root: Path) -> dict[Path, str]:
    config = _object(json.loads(_inside(root, Path(CONFIG_PATH)).read_bytes()), "config")
    languages = _object(_object(config.get("sources"), "sources").get("languages"), "languages")
    hunspell = _object(_object(config.get("external_evaluation"), "external_evaluation").get("hunspell"), "hunspell")
    if set(languages) != {"en_US", "ru_RU"} or set(hunspell) != {"en_US", "ru_RU"}:
        raise ValueError("reference lexical profiles must contain en_US and ru_RU")
    result: dict[Path, str] = {}
    for locale in ("en_US", "ru_RU"):
        language = _object(languages[locale], "languages." + locale)
        spelling = _object(hunspell[locale], "hunspell." + locale)
        name = language.get("path")
        if not isinstance(name, str) or not name or Path(name).is_absolute():
            raise ValueError("reference lexicon path must be repository relative")
        specifications = (
            (Path(name), language.get("sha256")),
            (Path("model/intent_v1/sources/hunspell") / (locale + ".dic"), spelling.get("dictionary_sha256")),
            (Path("model/intent_v1/sources/hunspell") / (locale + ".aff"), spelling.get("affix_sha256")),
        )
        for path, expected in specifications:
            if not isinstance(expected, str) or len(expected) != 64 or any(char not in "0123456789abcdef" for char in expected):
                raise ValueError("invalid reference lexical checksum")
            result[_inside(root, path)] = expected
    if len(result) != 6:
        raise ValueError("reference lexical inputs must be six distinct files")
    return result


def runtime_provenance(root: Path, extras: Sequence[Path]) -> dict[str, str]:
    """Bind replay dependencies, models and validated reference lexical bytes."""
    root = root.resolve(strict=True)
    paths = {_inside(root, Path(name)) for name in REQUIRED_PATHS}
    for directory, pattern in ((root / "src/keyswitch", "*.py"),
                               (root / "src/keyswitch/resources/models", "*")):
        for path in directory.rglob("*"):
            if path.is_symlink() and path.is_dir():
                raise ValueError("runtime directories must not be symlinks: " + str(path))
            if path.match(pattern) and (path.is_file() or path.is_symlink()):
                paths.add(_inside(root, path))
    optional = root / "src/keyswitch/resources/protected_tokens.json"
    if optional.exists() or optional.is_symlink():
        paths.add(_inside(root, optional))
    lexical = _lexical_paths(root)
    paths.update(lexical)
    paths.update(_inside(root, path) for path in extras)
    result = {path.relative_to(root).as_posix(): _checksum(path) for path in sorted(paths)}
    for path, expected in lexical.items():
        if result[path.relative_to(root).as_posix()] != expected:
            raise ValueError("reference lexical checksum mismatch: " + str(path))
    return result


@lru_cache(maxsize=1)
def _packaged_intent(path: Path, modified_ns: int, size: int) -> tuple[LinearNgramModel, IntentModelStatus]:
    # Stat fields invalidate decoding; the replay provenance binds exact bytes.
    model = LinearNgramModel.load(path)
    return model, IntentModelStatus(True, path, model.model_version, model.checksum, None)


def packaged_intent(root: Path) -> tuple[LinearNgramModel, IntentModelStatus]:
    """Load the packaged KSLM explicitly, without environment-based discovery."""
    root = root.resolve(strict=True)
    path = _inside(root, Path(INTENT_PATH)).resolve(strict=True)
    stat = path.stat()
    return _packaged_intent(path, stat.st_mtime_ns, stat.st_size)

"""Packaged identifier lexicon: executable names known to the distribution index.

A command name typed in the wrong layout looks like gibberish in both readings
to the natural-language lexicons and n-gram models, so this list supplies the
same kind of lexical evidence for identifiers that dictionaries supply for
words. It is a resource, not a label: it never decides anything by itself.
"""

from __future__ import annotations

import hashlib
import json
import re
from functools import lru_cache
from pathlib import Path
from typing import Final

RESOURCE_PATH: Final = Path(__file__).parent / "resources" / "identifiers.json"
MAX_RESOURCE_BYTES: Final = 4 * 1024 * 1024
MAX_IDENTIFIERS: Final = 200000
IDENTIFIER: Final = re.compile(r"[a-z][a-z0-9]{2,63}\Z")
LOADED_LEXICON_CACHE_SIZE: Final = 4
VERSION_HASH_CHARACTERS: Final = 12


class IdentifierLexicon:
    def __init__(self, identifiers: frozenset[str], name: str, version: str) -> None:
        self.identifiers = identifiers
        self.name = name
        self.version = version

    @staticmethod
    def normalize(token: str) -> str:
        return token.casefold()

    def contains(self, token: str) -> bool:
        """Exact, case-insensitive membership; no prefixes, no fuzzy matching."""

        return self.normalize(token) in self.identifiers

    @classmethod
    def load(cls, path: Path = RESOURCE_PATH) -> IdentifierLexicon:
        return cls._load_cached(path.resolve())

    @staticmethod
    @lru_cache(maxsize=LOADED_LEXICON_CACHE_SIZE)
    def _load_cached(path: Path) -> IdentifierLexicon:
        with path.open("rb") as handle:
            raw = handle.read(MAX_RESOURCE_BYTES + 1)
        if len(raw) > MAX_RESOURCE_BYTES:
            raise ValueError("identifier lexicon is too large")
        payload: object = json.loads(raw)
        if (not isinstance(payload, dict) or payload.get("schema_version") != 1
                or not isinstance(payload.get("name"), str) or not isinstance(payload.get("identifiers"), list)):
            raise ValueError("invalid identifier lexicon")
        entries: list[object] = payload["identifiers"]
        if not 0 < len(entries) <= MAX_IDENTIFIERS or entries != sorted(set(entries), key=str):
            raise ValueError("identifier lexicon must be a sorted set")
        identifiers: set[str] = set()
        for entry in entries:
            if not isinstance(entry, str) or not IDENTIFIER.fullmatch(entry):
                raise ValueError("invalid identifier entry")
            identifiers.add(entry)
        return IdentifierLexicon(frozenset(identifiers), str(payload["name"]),
                                 "identifiers-" + hashlib.sha256(raw).hexdigest()[:VERSION_HASH_CHARACTERS])

    @classmethod
    def try_load(cls, path: Path = RESOURCE_PATH) -> tuple[IdentifierLexicon | None, str]:
        try:
            lexicon = cls.load(path)
        except (OSError, ValueError) as error:
            return None, str(error)
        return lexicon, lexicon.version

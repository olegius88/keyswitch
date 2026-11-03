"""The user's own words, which the typo correction leaves as typed (KeySwitchEngine._typo_plan).

A word becomes the user's own when they type it and leave it as it is PERSONAL_WORD_MIN_COUNT times, or at once when
they take back a typo correction of it: `комить`, `иишка` and `митап` are words of their language for them, whatever a
word list says. The file beside the history keeps no text: each word is stored as a salted SHA-256 (the salt is drawn
once per installation and kept in the same file), so the file lists which hashes are the user's words and how often
each word not yet theirs was left as typed. Both lists are bounded; the oldest entry goes first.
"""

from __future__ import annotations

import hashlib
import json
import secrets
import threading
from collections.abc import Callable
from pathlib import Path

from .config import set_aside_unreadable
from .constants.file_formats import USER_DATA_JSON_INDENT
from .constants.typo import (
    PERSONAL_WORD_COUNTS_MAX,
    PERSONAL_WORD_HASH_HEX_CHARACTERS,
    PERSONAL_WORD_MIN_COUNT,
    PERSONAL_WORD_SALT_BYTES,
    PERSONAL_WORDS_MAX,
    PERSONAL_WORDS_SCHEMA_VERSION,
)
from .history import data_dir


class PersonalWords:
    """The user's own words as salted hashes, in `personal-words.json`.

    `persist` says whether the file may be written now: with the history off (general.keep_history) nothing typed is
    stored, and the words stay the user's for the session only.
    """

    def __init__(self, path: Path | None = None, *, persist: Callable[[], bool] = lambda: True) -> None:
        self.path = path or data_dir() / "personal-words.json"
        self._persist = persist
        self._lock = threading.RLock()
        self._salt = ""
        # Insertion-ordered: the first entry is the one used longest ago, and the first to go past the bound.
        self._personal: dict[str, None] = {}
        self._counts: dict[str, int] = {}
        # Where an unreadable file was moved by the last load, if anywhere.
        self.unreadable: Path | None = None
        self.load()

    def load(self) -> None:
        with self._lock:
            self._salt, self._personal, self._counts = "", {}, {}
            if not self.path.is_file():
                return
            try:
                payload: object = json.loads(self.path.read_text(encoding="utf-8-sig"))
            except (OSError, ValueError):
                payload = None
            if (not isinstance(payload, dict) or payload.get("schema_version") != PERSONAL_WORDS_SCHEMA_VERSION
                    or not isinstance(payload.get("salt"), str) or not isinstance(payload.get("personal"), list)
                    or not isinstance(payload.get("counts"), dict)):
                # The next word would save over the file: keep the user's words aside.
                self.unreadable = set_aside_unreadable(self.path)
                return
            self._salt = str(payload["salt"])
            personal: list[object] = payload["personal"]
            counts: dict[object, object] = payload["counts"]
            self._personal = {key: None for key in personal if isinstance(key, str)}
            self._counts = {key: count for key, count in counts.items()
                            if isinstance(key, str) and type(count) is int and count > 0}

    def _key(self, word: str) -> str:
        if not self._salt:
            self._salt = secrets.token_hex(PERSONAL_WORD_SALT_BYTES)
        digest = hashlib.sha256((self._salt + word.casefold()).encode("utf-8")).hexdigest()
        return digest[:PERSONAL_WORD_HASH_HEX_CHARACTERS]

    def __contains__(self, word: object) -> bool:
        with self._lock:
            return isinstance(word, str) and bool(self._salt) and self._key(word) in self._personal

    def __len__(self) -> int:
        with self._lock:
            return len(self._personal)

    def add(self, word: str) -> None:
        """Make the word the user's own at once (a typo correction of it was taken back)."""

        with self._lock:
            key = self._key(word)
            self._counts.pop(key, None)
            self._remember(key)
            self.save()

    def keep(self, word: str) -> bool:
        """Count the word left as typed once more; whether it is the user's own now."""

        with self._lock:
            key = self._key(word)
            if key in self._personal:
                return True
            count = self._counts.pop(key, 0) + 1
            if count >= PERSONAL_WORD_MIN_COUNT:
                self._remember(key)
            else:
                self._counts[key] = count
                while len(self._counts) > PERSONAL_WORD_COUNTS_MAX:
                    del self._counts[next(iter(self._counts))]
            self.save()
            return count >= PERSONAL_WORD_MIN_COUNT

    def _remember(self, key: str) -> None:
        self._personal.pop(key, None)
        self._personal[key] = None
        while len(self._personal) > PERSONAL_WORDS_MAX:
            del self._personal[next(iter(self._personal))]

    def clear(self) -> None:
        """Forget every word, with a new salt: hashes kept elsewhere match nothing any more."""

        with self._lock:
            self._salt, self._personal, self._counts = "", {}, {}
            self.save()

    def save(self) -> None:
        with self._lock:
            if not self._persist():
                return
            data = {
                "schema_version": PERSONAL_WORDS_SCHEMA_VERSION,
                "salt": self._salt,
                "personal": list(self._personal),
                "counts": self._counts,
            }
            try:
                self.path.parent.mkdir(parents=True, exist_ok=True)
                temporary = self.path.with_suffix(".json.tmp")
                temporary.write_text(json.dumps(data, indent=USER_DATA_JSON_INDENT) + "\n", encoding="utf-8")
                temporary.replace(self.path)
            except OSError:
                # The words stay the user's for this session; the next change tries again.
                return

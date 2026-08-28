"""Local, explicit learning from manual conversions and rejected corrections."""

from __future__ import annotations

import json
import threading
from pathlib import Path
from typing import TypedDict

from .history import data_dir


class _LearningData(TypedDict):
    schema_version: int
    rules: dict[str, object]
    rejections: dict[str, object]


def _string_keyed_dict(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict):
        return None
    result: dict[str, object] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            return None
        result[key] = item
    return result


class LearningStore:
    """Persist only words on which the user explicitly acted.

    A manual conversion increments a rule counter. An undo of an automatic
    correction records a hard rejection for that source/target direction.
    Ordinary typed text is never written here.
    """

    SCHEMA_VERSION = 2

    def __init__(self, path: Path | None = None) -> None:
        self.path = path or data_dir() / "learning.json"
        self._lock = threading.RLock()
        self._data: _LearningData = {
            "schema_version": self.SCHEMA_VERSION,
            "rules": {},
            "rejections": {},
        }
        self.load()

    @staticmethod
    def _key(source_group: int, word: str) -> str:
        # Keep punctuation that corresponds to letters in another layout.
        # `,fpf` (база) and `fpf` (аза) are different physical sequences and
        # must never share a learned rule.
        return f"{source_group}:{word.strip().casefold()}"

    def load(self) -> None:
        with self._lock:
            if not self.path.is_file():
                return
            try:
                payload = json.loads(self.path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                return
            if not isinstance(payload, dict):
                return
            rules = _string_keyed_dict(payload.get("rules", {}))
            rejections = _string_keyed_dict(payload.get("rejections", {}))
            if rules is not None and rejections is not None:
                self._data = {
                    "schema_version": self.SCHEMA_VERSION,
                    "rules": rules,
                    "rejections": rejections,
                }

    def save(self) -> None:
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix(".json.tmp")
            temporary.write_text(
                json.dumps(self._data, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            temporary.replace(self.path)

    def record_manual(
        self, source_group: int, word: str, target_group: int
    ) -> int:
        key = self._key(source_group, word)
        if not key.partition(":")[2] or source_group == target_group:
            return 0
        with self._lock:
            rules = self._data["rules"]
            current = rules.get(key, {})
            if not isinstance(current, dict) or current.get("target_group") != target_group:
                current = {"target_group": target_group, "confirmations": 0}
            confirmations = min(999, int(current.get("confirmations", 0)) + 1)
            rules[key] = {
                "target_group": target_group,
                "confirmations": confirmations,
            }
            rejections = self._data["rejections"]
            rejected = rejections.get(key, [])
            if isinstance(rejected, list) and target_group in rejected:
                remaining = [item for item in rejected if item != target_group]
                if remaining:
                    rejections[key] = remaining
                else:
                    rejections.pop(key, None)
            self.save()
            return confirmations

    def confirm_manual(
        self,
        source_group: int,
        word: str,
        target_group: int,
        confirmations_required: int,
    ) -> int:
        """Immediately confirm a rule offered after a manual conversion."""

        key = self._key(source_group, word)
        if not key.partition(":")[2] or source_group == target_group:
            return 0
        required = max(1, min(999, confirmations_required))
        with self._lock:
            rules = self._data["rules"]
            current = rules.get(key, {})
            confirmations = 0
            if (
                isinstance(current, dict)
                and current.get("target_group") == target_group
            ):
                confirmations = int(current.get("confirmations", 0))
            confirmed = max(required, confirmations)
            rules[key] = {
                "target_group": target_group,
                "confirmations": confirmed,
            }
            rejections = self._data["rejections"]
            rejected = rejections.get(key, [])
            if isinstance(rejected, list) and target_group in rejected:
                remaining = [item for item in rejected if item != target_group]
                if remaining:
                    rejections[key] = remaining
                else:
                    rejections.pop(key, None)
            self.save()
            return confirmed

    def reject(self, source_group: int, word: str, target_group: int) -> None:
        key = self._key(source_group, word)
        if not key.partition(":")[2] or source_group == target_group:
            return
        with self._lock:
            rejections = self._data["rejections"]
            rejected = rejections.get(key, [])
            if not isinstance(rejected, list):
                rejected = []
            rejections[key] = sorted({int(item) for item in rejected} | {target_group})
            rules = self._data["rules"]
            current = rules.get(key)
            if isinstance(current, dict) and current.get("target_group") == target_group:
                rules.pop(key, None)
            self.save()

    def forced_target(
        self, source_group: int, word: str, confirmations_required: int = 2
    ) -> int | None:
        key = self._key(source_group, word)
        with self._lock:
            rules = self._data["rules"]
            rule = rules.get(key)
            if not isinstance(rule, dict):
                return None
            if int(rule.get("confirmations", 0)) < max(1, confirmations_required):
                return None
            try:
                return int(rule["target_group"])
            except (KeyError, TypeError, ValueError):
                return None

    def rejected_targets(self, source_group: int, word: str) -> set[int]:
        key = self._key(source_group, word)
        with self._lock:
            rejections = self._data["rejections"]
            values = rejections.get(key, [])
            if not isinstance(values, list):
                return set()
            result: set[int] = set()
            for value in values:
                try:
                    result.add(int(value))
                except (TypeError, ValueError):
                    continue
            return result

    def counts(self) -> tuple[int, int]:
        with self._lock:
            rules = self._data["rules"]
            rejections = self._data["rejections"]
            return len(rules), sum(
                len(values) for values in rejections.values() if isinstance(values, list)
            )

    def clear(self) -> None:
        with self._lock:
            self._data = {
                "schema_version": self.SCHEMA_VERSION,
                "rules": {},
                "rejections": {},
            }
            self.save()

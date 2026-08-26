"""Privacy-conscious correction history (only corrected word pairs)."""

from __future__ import annotations

import json
import os
import threading
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable


def data_dir() -> Path:
    override = os.environ.get("KEYSWITCH_DATA_DIR")
    if override:
        return Path(override)
    base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    return base / "keyswitch"


@dataclass(frozen=True)
class HistoryEntry:
    timestamp: str
    original: str
    replacement: str
    application: str
    confidence: float

    @classmethod
    def create(
        cls, original: str, replacement: str, application: str, confidence: float
    ) -> "HistoryEntry":
        return cls(
            datetime.now(timezone.utc).isoformat(timespec="seconds"),
            original,
            replacement,
            application,
            round(confidence, 2),
        )


class HistoryStore:
    def __init__(self, path: Path | None = None, limit: int = 200) -> None:
        self.path = path or data_dir() / "history.jsonl"
        self.limit = max(1, limit)
        self._lock = threading.RLock()
        self._callbacks: list[Callable[[], None]] = []

    def append(self, entry: HistoryEntry) -> None:
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self.path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(asdict(entry), ensure_ascii=False) + "\n")
            entries = self.read(self.limit + 1)
            if len(entries) > self.limit:
                self._rewrite(entries[-self.limit :])
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback()

    def read(self, limit: int | None = None) -> list[HistoryEntry]:
        with self._lock:
            if not self.path.exists():
                return []
            result: list[HistoryEntry] = []
            try:
                for line in self.path.read_text(encoding="utf-8").splitlines():
                    try:
                        payload = json.loads(line)
                        result.append(HistoryEntry(**payload))
                    except (ValueError, TypeError):
                        continue
            except OSError:
                return []
            return result[-limit:] if limit is not None else result

    def _rewrite(self, entries: list[HistoryEntry]) -> None:
        self.path.write_text(
            "".join(json.dumps(asdict(item), ensure_ascii=False) + "\n" for item in entries),
            encoding="utf-8",
        )

    def clear(self) -> None:
        with self._lock:
            if self.path.exists():
                self.path.write_text("", encoding="utf-8")
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback()

    def subscribe(self, callback: Callable[[], None]) -> None:
        with self._lock:
            self._callbacks.append(callback)

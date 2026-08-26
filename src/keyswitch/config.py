"""Persistent, observable application settings."""

from __future__ import annotations

import copy
import json
import os
import threading
from pathlib import Path
from typing import Any, Callable


DEFAULTS: dict[str, Any] = {
    "schema_version": 1,
    "enabled": True,
    "general": {
        "start_hidden": False,
        "close_to_tray": True,
        "autostart": False,
        "notifications": True,
        "sound": False,
        "keep_history": True,
    },
    "detection": {
        "layouts": ["us", "ru"],
        "language_models": ["en_US", "ru_RU"],
        "minimum_length": 3,
        "confidence": 2.0,
        "correct_on_space": True,
        "correct_on_enter": True,
        "correct_on_tab": True,
        "correct_on_punctuation": True,
        "aggressive": False,
    },
    "hotkeys": {
        "toggle": "Ctrl+Alt+P",
        "convert_last": "Pause",
        "undo": "Ctrl+Alt+Z",
    },
    "exclusions": {
        "applications": ["keepassxc", "1password", "bitwarden"],
        "words": [],
    },
    "appearance": {
        "theme": "system",
        "show_indicator": True,
    },
    "history": {"limit": 200},
}


def _deep_merge(defaults: dict[str, Any], loaded: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(defaults)
    for key, value in loaded.items():
        if isinstance(value, dict) and isinstance(result.get(key), dict):
            result[key] = _deep_merge(result[key], value)
        else:
            result[key] = value
    return result


def config_dir() -> Path:
    override = os.environ.get("KEYSWITCH_CONFIG_DIR")
    if override:
        return Path(override)
    base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return base / "keyswitch"


class SettingsStore:
    """Thread-safe JSON settings with dotted-path access and change callbacks."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = path or config_dir() / "config.json"
        self._lock = threading.RLock()
        self._callbacks: list[Callable[[str, Any], None]] = []
        self._data = copy.deepcopy(DEFAULTS)
        self.load()

    def load(self) -> None:
        with self._lock:
            if not self.path.exists():
                return
            try:
                loaded = json.loads(self.path.read_text(encoding="utf-8"))
                if isinstance(loaded, dict):
                    self._data = _deep_merge(DEFAULTS, loaded)
            except (OSError, ValueError):
                # Keep safe defaults. The diagnostics page reports the path so
                # the user can repair a malformed file without data deletion.
                self._data = copy.deepcopy(DEFAULTS)

    def save(self) -> None:
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix(".json.tmp")
            temporary.write_text(
                json.dumps(self._data, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            temporary.replace(self.path)

    def get(self, dotted_path: str, default: Any = None) -> Any:
        with self._lock:
            value: Any = self._data
            for part in dotted_path.split("."):
                if not isinstance(value, dict) or part not in value:
                    return default
                value = value[part]
            return copy.deepcopy(value)

    def set(self, dotted_path: str, value: Any, *, persist: bool = True) -> None:
        with self._lock:
            target = self._data
            parts = dotted_path.split(".")
            for part in parts[:-1]:
                node = target.get(part)
                if not isinstance(node, dict):
                    node = {}
                    target[part] = node
                target = node
            if target.get(parts[-1]) == value:
                return
            target[parts[-1]] = copy.deepcopy(value)
            if persist:
                self.save()
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback(dotted_path, copy.deepcopy(value))

    def subscribe(self, callback: Callable[[str, Any], None]) -> Callable[[], None]:
        with self._lock:
            self._callbacks.append(callback)

        def unsubscribe() -> None:
            with self._lock:
                if callback in self._callbacks:
                    self._callbacks.remove(callback)

        return unsubscribe

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            return copy.deepcopy(self._data)

    def reset(self) -> None:
        with self._lock:
            self._data = copy.deepcopy(DEFAULTS)
            self.save()
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback("*", self.snapshot())

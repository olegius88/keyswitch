"""Persistent, observable application settings."""

from __future__ import annotations

import copy
import json
import os
import sys
import threading
from pathlib import Path
from typing import Callable, TypeVar, cast, overload


SettingsData = dict[str, object]
SettingCallback = Callable[[str, object], None]
_T = TypeVar("_T")


DEFAULTS: SettingsData = {
    "schema_version": 3,
    "enabled": True,
    "general": {
        "start_hidden": True,
        "close_to_tray": True,
        "autostart": True,
        "notifications": True,
        "sound": False,
        "keep_history": True,
    },
    "detection": {
        "layouts": ["us", "ru"],
        "language_models": ["en_US", "ru_RU"],
        "minimum_length": 3,
        "confidence": 2.0,
        "correct_on_pause": True,
        "correct_on_space": True,
        "correct_on_enter": True,
        "correct_on_tab": True,
        "correct_on_punctuation": True,
        "respect_manual_layout": True,
        "aggressive": False,
        "context_aware": True,
        "protect_code": True,
        "learning": True,
        "learning_confirmations": 2,
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
        "indicator_style": "letters",
    },
    "history": {"limit": 200},
}


def _string_keyed_mapping(value: object) -> SettingsData | None:
    """Return a JSON object with string keys, rejecting malformed mappings."""

    if not isinstance(value, dict):
        return None
    result: SettingsData = {}
    for key, item in value.items():
        if not isinstance(key, str):
            return None
        result[key] = item
    return result


def _deep_merge(defaults: SettingsData, loaded: SettingsData) -> SettingsData:
    result = copy.deepcopy(defaults)
    for key, value in loaded.items():
        loaded_mapping = _string_keyed_mapping(value)
        default_mapping = _string_keyed_mapping(result.get(key))
        if loaded_mapping is not None and default_mapping is not None:
            result[key] = _deep_merge(default_mapping, loaded_mapping)
        else:
            result[key] = copy.deepcopy(value)
    return result


def config_dir() -> Path:
    override = os.environ.get("KEYSWITCH_CONFIG_DIR")
    if override:
        return Path(override)
    if _running_on_windows():
        roaming = os.environ.get("APPDATA")
        base = Path(roaming) if roaming else Path.home() / "AppData" / "Roaming"
        return base / "KeySwitch"
    xdg_config = os.environ.get("XDG_CONFIG_HOME")
    base = Path(xdg_config) if xdg_config else Path.home() / ".config"
    return base / "keyswitch"


def _running_on_windows() -> bool:
    return sys.platform == "win32"


class SettingsStore:
    """Thread-safe JSON settings with dotted-path access and change callbacks."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = path or config_dir() / "config.json"
        self._lock = threading.RLock()
        self._callbacks: list[SettingCallback] = []
        self._data: SettingsData = copy.deepcopy(DEFAULTS)
        self.load()

    def load(self) -> None:
        with self._lock:
            if not self.path.exists():
                return
            try:
                loaded: object = json.loads(self.path.read_text(encoding="utf-8"))
                loaded_mapping = _string_keyed_mapping(loaded)
                if loaded_mapping is not None:
                    self._data = _deep_merge(DEFAULTS, loaded_mapping)
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

    @overload
    def get(self, dotted_path: str) -> object | None: ...

    @overload
    def get(self, dotted_path: str, default: _T) -> _T: ...

    def get(self, dotted_path: str, default: object = None) -> object:
        with self._lock:
            value: object = self._data
            for part in dotted_path.split("."):
                mapping = _string_keyed_mapping(value)
                if mapping is None or part not in mapping:
                    return default
                value = mapping[part]
            # The overload ties the result to the caller-provided default. The
            # persisted settings schema is seeded from DEFAULTS and merged by
            # path, so this cast is the single typed boundary for JSON data.
            return copy.deepcopy(value)

    def set(self, dotted_path: str, value: object, *, persist: bool = True) -> None:
        with self._lock:
            target = self._data
            parts = dotted_path.split(".")
            for part in parts[:-1]:
                node = target.get(part)
                if not isinstance(node, dict):
                    node = {}
                    target[part] = node
                target = cast(SettingsData, node)
            if target.get(parts[-1]) == value:
                return
            target[parts[-1]] = copy.deepcopy(value)
            if persist:
                self.save()
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback(dotted_path, copy.deepcopy(value))

    def subscribe(self, callback: SettingCallback) -> Callable[[], None]:
        with self._lock:
            self._callbacks.append(callback)

        def unsubscribe() -> None:
            with self._lock:
                if callback in self._callbacks:
                    self._callbacks.remove(callback)

        return unsubscribe

    def snapshot(self) -> SettingsData:
        with self._lock:
            return copy.deepcopy(self._data)

    def reset(self) -> None:
        with self._lock:
            self._data = copy.deepcopy(DEFAULTS)
            self.save()
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback("*", self.snapshot())

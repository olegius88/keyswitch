"""Persistent, observable application settings."""

from __future__ import annotations

import copy
import json
import logging
import os
import sys
import threading
import time
from pathlib import Path
from typing import Callable, TypeVar, cast, overload
from .constants.file_formats import UNREADABLE_FILE_SUFFIX, UNREADABLE_FILE_TIME_FORMAT, USER_DATA_JSON_INDENT
from .constants.settings_defaults import DEFAULT_SETTINGS, EARLY_SWITCH_DEFAULT_SCHEMA_VERSION


LOGGER = logging.getLogger(__name__)


SettingsData = dict[str, object]
SettingCallback = Callable[[str, object], None]
_T = TypeVar("_T")


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


def set_aside_unreadable(path: Path) -> Path | None:
    """Move a user file that could not be read out of the way of the next save.

    The application then runs on defaults, and saving them must not destroy what
    the user had: the bytes stay next to the file under a dated name.
    """

    target = path.with_name(path.name + UNREADABLE_FILE_SUFFIX + time.strftime(UNREADABLE_FILE_TIME_FORMAT))
    try:
        path.replace(target)
    except OSError:
        LOGGER.warning("%s could not be read or moved aside", path)
        return None
    LOGGER.warning("%s could not be read; kept as %s, defaults in use", path, target.name)
    return target


class SettingsStore:
    """Thread-safe JSON settings with dotted-path access and change callbacks."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = path or config_dir() / "config.json"
        self._lock = threading.RLock()
        self._callbacks: list[SettingCallback] = []
        self._data: SettingsData = copy.deepcopy(DEFAULT_SETTINGS)
        # Where an unreadable settings file was moved by the last load, if anywhere.
        self.unreadable: Path | None = None
        self.load()

    def load(self) -> None:
        with self._lock:
            if not self.path.exists():
                return
            try:
                # utf-8-sig: Notepad saves UTF-8 with a byte order mark, which json rejects.
                loaded: object = json.loads(self.path.read_text(encoding="utf-8-sig"))
            except (OSError, ValueError):
                loaded = None
            loaded_mapping = _string_keyed_mapping(loaded)
            if loaded_mapping is None:
                # Keep safe defaults, and keep the user's file: the next change
                # of any setting would otherwise save the defaults over it.
                self._data = copy.deepcopy(DEFAULT_SETTINGS)
                self.unreadable = set_aside_unreadable(self.path)
                return
            self._data = _deep_merge(DEFAULT_SETTINGS, loaded_mapping)
            self._adopt_new_defaults(loaded_mapping)

    def _adopt_new_defaults(self, loaded: SettingsData) -> None:
        """Let a file written by an older schema take the defaults that changed since.

        The persisted file is seeded from the shipped defaults, so a value equal to the
        old default is most likely the seed and not a choice. Schema 7 turned the early
        layout switch on; a schema-6 file with it off adopts that once, and the schema
        version written back marks the adoption done, so turning it off again holds.
        """

        version = loaded.get("schema_version")
        if not isinstance(version, int) or isinstance(version, bool) or version >= EARLY_SWITCH_DEFAULT_SCHEMA_VERSION:
            return
        detection = _string_keyed_mapping(loaded.get("detection"))
        current = self._data.get("detection")
        if detection is not None and detection.get("early_switch") is False and isinstance(current, dict):
            current["early_switch"] = True
        self._data["schema_version"] = DEFAULT_SETTINGS["schema_version"]

    def save(self) -> None:
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix(".json.tmp")
            temporary.write_text(
                json.dumps(self._data, ensure_ascii=False, indent=USER_DATA_JSON_INDENT) + "\n",
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

    def default(self, dotted_path: str, fallback: object = None) -> object:
        """Return the shipped value for a path without touching stored data."""

        value: object = DEFAULT_SETTINGS
        for part in dotted_path.split("."):
            mapping = _string_keyed_mapping(value)
            if mapping is None or part not in mapping:
                return fallback
            value = mapping[part]
        return copy.deepcopy(value)

    def is_default(self, dotted_path: str) -> bool:
        missing = object()
        return self.get(dotted_path, missing) == self.default(dotted_path, missing)

    def restore_default(self, dotted_path: str) -> bool:
        """Reset a single path; unknown paths are left untouched."""

        missing = object()
        value = self.default(dotted_path, missing)
        if value is missing:
            return False
        self.set(dotted_path, value)
        return True

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
            self._data = copy.deepcopy(DEFAULT_SETTINGS)
            self.save()
            callbacks = tuple(self._callbacks)
        for callback in callbacks:
            callback("*", self.snapshot())

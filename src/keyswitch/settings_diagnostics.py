"""Versioned, privacy-aware configuration deltas for the technical journal."""

from __future__ import annotations

import hashlib
import json
from typing import cast

from .config import DEFAULTS, SettingsData, SettingsStore


_MISSING = object()
_DEFAULTS_SHA256 = hashlib.sha256(
    json.dumps(DEFAULTS, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
).hexdigest()


def _loggable_value(value: object) -> object:
    if isinstance(value, (bool, int, float)) or value is None:
        return value
    if isinstance(value, str):
        return value if len(value) <= 80 else value[:77] + "..."
    if isinstance(value, (list, tuple, set, dict)):
        return {"type": type(value).__name__, "items": len(value)}
    return "<unsupported>"


def _overrides(defaults: SettingsData, current: SettingsData, prefix: str = "") -> SettingsData:
    result: SettingsData = {}
    # Walk known defaults, never arbitrary names from user configuration.
    for name, default in defaults.items():
        path = f"{prefix}.{name}" if prefix else name
        if path == "schema_version":
            continue
        value = current.get(name, _MISSING)
        if isinstance(default, dict) and isinstance(value, dict):
            result.update(_overrides(cast(SettingsData, default), cast(SettingsData, value), path))
        elif value != default:
            result[path] = {"missing": True} if value is _MISSING else _loggable_value(value)
    return result


def settings_snapshot(settings: SettingsStore) -> dict[str, object]:
    """One atomic snapshot; omitted known paths use the shipped defaults."""
    return {
        "format": "overrides-v1",
        "defaults_schema": DEFAULTS["schema_version"],
        "defaults_sha256": _DEFAULTS_SHA256,
        "overrides": _overrides(DEFAULTS, settings.snapshot()),
    }


def setting_change(settings: SettingsStore, path: str, value: object) -> dict[str, object] | None:
    """Represent reset without repeating its default; never disclose unknown keys."""
    default = settings.default(path, _MISSING)
    if path == "*" or isinstance(default, dict):
        return {"path": path, "operation": "snapshot", "settings": settings_snapshot(settings)}
    if default is _MISSING or path == "schema_version":
        return None
    if value == default:
        return {"path": path, "operation": "reset"}
    return {"path": path, "operation": "set", "value": _loggable_value(value)}

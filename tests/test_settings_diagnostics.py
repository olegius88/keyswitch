"""Only non-default configuration is serialized into diagnostic snapshots."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from keyswitch.config import DEFAULTS, SettingsStore
from keyswitch.settings_diagnostics import setting_change, settings_snapshot


class SettingsDiagnosticsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.store = SettingsStore(Path(self.temporary.name) / "config.json")

    def test_defaults_have_empty_overrides_and_versioned_public_baseline(self) -> None:
        result = settings_snapshot(self.store)
        self.assertEqual(result["format"], "overrides-v1")
        self.assertEqual(result["overrides"], {})
        self.assertEqual(result["defaults_schema"], DEFAULTS["schema_version"])
        expected = hashlib.sha256(json.dumps(DEFAULTS, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        self.assertEqual(result["defaults_sha256"], expected)

    def test_all_known_sections_are_compared_and_private_collections_are_summarized(self) -> None:
        changes = {"enabled": False, "detection.context_read_field": True,
                   "general.sound": True, "appearance.theme": "dark",
                   "hotkeys.convert_last": "F12", "updates.check_automatically": False,
                   "history.limit": 50, "exclusions.words": ["private-token"]}
        for path, value in changes.items():
            self.store.set(path, value, persist=False)
        snapshot = settings_snapshot(self.store)
        self.assertEqual(snapshot["overrides"], {
            **changes, "exclusions.words": {"type": "list", "items": 1},
        })
        self.assertNotIn("private-token", json.dumps(snapshot))

    def test_unknown_keys_and_schema_metadata_do_not_become_user_settings(self) -> None:
        self.store.set("future.token", "private-token", persist=False)
        self.store.set("detection.future_token", "private-token", persist=False)
        self.store.set("schema_version", 0, persist=False)
        self.assertEqual(settings_snapshot(self.store)["overrides"], {})
        for path in ("future.token", "detection.future_token", "schema_version"):
            self.assertIsNone(setting_change(self.store, path, "private-token"))

    def test_change_and_reset_never_repeat_a_default_value(self) -> None:
        self.store.set("detection.minimum_length", 5, persist=False)
        self.assertEqual(setting_change(self.store, "detection.minimum_length", 5), {
            "path": "detection.minimum_length", "operation": "set", "value": 5,
        })
        self.store.restore_default("detection.minimum_length")
        self.assertEqual(setting_change(self.store, "detection.minimum_length", 3), {
            "path": "detection.minimum_length", "operation": "reset",
        })
        self.assertEqual(settings_snapshot(self.store)["overrides"], {})

    def test_group_changes_and_reload_are_complete_replacement_snapshots(self) -> None:
        self.store.set("detection.minimum_length", 5, persist=False)
        for path in ("detection", "*"):
            self.assertEqual(setting_change(self.store, path, {}), {
                "path": path, "operation": "snapshot", "settings": settings_snapshot(self.store),
            })

    def test_missing_known_values_are_distinguished_from_null(self) -> None:
        self.store.set("hotkeys", {}, persist=False)
        self.assertEqual(settings_snapshot(self.store)["overrides"], {
            "hotkeys.toggle": {"missing": True}, "hotkeys.convert_last": {"missing": True},
            "hotkeys.undo": {"missing": True},
        })
        self.store.set("hotkeys", None, persist=False)
        self.assertEqual(settings_snapshot(self.store)["overrides"], {"hotkeys": None})

    def test_invalid_known_values_are_bounded_without_serializing_unknown_objects(self) -> None:
        class PrivateValue:
            def __str__(self) -> str:
                raise AssertionError("Do not format arbitrary settings values")

        for value, expected in (
            (None, None), ("x" * 100, "x" * 77 + "..."),
            (("private",), {"type": "tuple", "items": 1}),
            ({"private"}, {"type": "set", "items": 1}),
            ({"private": "value"}, {"type": "dict", "items": 1}),
            (PrivateValue(), "<unsupported>"),
        ):
            with self.subTest(value_type=type(value).__name__):
                self.store.set("general.sound", value, persist=False)
                self.assertEqual(setting_change(self.store, "general.sound", value), {
                    "path": "general.sound", "operation": "set", "value": expected,
                })
                self.assertEqual(settings_snapshot(self.store)["overrides"], {"general.sound": expected})


if __name__ == "__main__":
    unittest.main()

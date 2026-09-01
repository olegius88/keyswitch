"""Tests for the stable sealed intent-model release entry point."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import train_intent_model as trainer  # noqa: E402
import train_intent_model_release as release  # noqa: E402

class IntentModelReleaseTests(unittest.TestCase):
    def test_trainer_normalizes_tuple_metadata_without_changing_json(self) -> None:
        normalized = trainer.json_native_mapping(
            {"label": "v7", "nested": {"values": ("en", "ru")}}
        )
        self.assertEqual(
            normalized,
            {"label": "v7", "nested": {"values": ["en", "ru"]}},
        )

    def test_trainer_rejects_non_object_normalization(self) -> None:
        with patch.object(json, "loads", return_value=[]):
            with self.assertRaisesRegex(
                RuntimeError,
                "normalization was not an object",
            ):
                trainer.json_native_mapping({"label": "v7"})

    def test_trainer_rejects_changed_canonical_json(self) -> None:
        with patch.object(
            trainer,
            "_canonical_json_bytes",
            side_effect=(b'{"label":"v7"}\n', b'{"label":"changed"}\n'),
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "normalization changed bytes",
            ):
                trainer.json_native_mapping({"label": "v7"})

    def test_main_delegates(self) -> None:
        with patch.object(trainer, "main", return_value=7):
            self.assertEqual(release.main(), 7)

    def test_main_propagates_delegate_failure(self) -> None:
        with patch.object(trainer, "main", side_effect=RuntimeError("boom")):
            with self.assertRaisesRegex(RuntimeError, "boom"):
                release.main()


if __name__ == "__main__":
    unittest.main()

"""The engine's captured questions join the action model's TRAIN as a bounded, deterministic sample.

`captured_curriculum` reads the files the manifest pins, leaves out the inside-word files, takes
the convert share of each file's budget from its convert questions and the rest from its keep
questions, gives words no lexicon or identifier index knows their own share of the keeps and
weighs each row by the file's manifest weight times the recipe's scale.
"""

from __future__ import annotations

import hashlib
import json
import lzma
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from fixture_values.corpora import (
    CAPTURED_FIXTURE_BUDGET,
    CAPTURED_FIXTURE_CONVERT_SHARE,
    CAPTURED_FIXTURE_FILE_WEIGHT,
    CAPTURED_FIXTURE_KEEP_UNKNOWN_SHARE,
    CAPTURED_FIXTURE_MINIMUM_WORD_CHARACTERS,
    CAPTURED_FIXTURE_WEIGHT_SCALE,
)
from keyswitch.layouts import LayoutPair
from train_context_action_model import captured_curriculum
from train_context_model import CAPTURED_COLUMNS

# Words the reference lexicons know, typed in their own layout or in the other one.
KNOWN_RUSSIAN = ("привет", "работа", "сегодня", "вечером", "дом", "окно")
# Russian-looking words no lexicon holds: the keep answers a uniform sample rarely draws.
UNKNOWN_RUSSIAN = ("шупшуп", "фукшук", "дупшуп")


def question(original: str, alternative: str, group: int, label: str, *, before: str = "мы ",
             after: str = "", origin: str = "none") -> list[object]:
    values: dict[str, object] = {
        "count": 1, "original": original, "alternative": alternative, "source_group": group,
        "trigger": "space", "baseline_convert": False, "source_known": False, "target_known": False,
        "score_delta": 0.0, "literal_tail": "", "boundary_text": " ", "after_origin": origin,
        "source_typo": False, "target_typo": False, "source_opening": False, "target_opening": False,
        "inside": False, "before": before, "after": after, "role": "text", "label": label,
    }
    return [values[name] for name in CAPTURED_COLUMNS]


def convert(word: str) -> list[object]:
    """A Russian word typed in the English layout."""

    return question(LayoutPair().translate(word, "ru", "us"), word, 0, "convert")


class CapturedCurriculumTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)

    def manifest(self, files: dict[str, list[list[object]]]) -> Path:
        sources = []
        for name, rows in files.items():
            path = self.directory / name
            with lzma.open(path, "wt", encoding="utf-8") as handle:
                for row in rows:
                    handle.write(json.dumps(row, ensure_ascii=False) + "\n")
            sources.append({"file": name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                            "weight": CAPTURED_FIXTURE_FILE_WEIGHT})
        manifest = self.directory / "manifest.json"
        manifest.write_text(json.dumps({"schema_version": 1, "columns": list(CAPTURED_COLUMNS), "sources": sources}),
                            encoding="utf-8")
        return manifest

    def options(self, manifest: Path, convert_share: float | None = CAPTURED_FIXTURE_CONVERT_SHARE) -> dict[str, object]:
        return {
            "manifest": str(manifest), "maximum_rows_per_source": CAPTURED_FIXTURE_BUDGET,
            "convert_share": convert_share, "keep_unknown_share": CAPTURED_FIXTURE_KEEP_UNKNOWN_SHARE,
            "weight_scale": CAPTURED_FIXTURE_WEIGHT_SCALE, "excluded_file_markers": ["inside"],
            "minimum_word_characters": CAPTURED_FIXTURE_MINIMUM_WORD_CHARACTERS, "unknown_keep_groups": [1],
        }

    def typing_rows(self) -> list[list[object]]:
        rows = [convert(word) for word in KNOWN_RUSSIAN]
        rows += [question(word, word, 1, "keep") for word in KNOWN_RUSSIAN + UNKNOWN_RUSSIAN]
        return rows

    def test_a_file_is_sampled_by_its_shares_and_weighed_by_its_scaled_weight(self) -> None:
        manifest = self.manifest({"typing.jsonl.xz": self.typing_rows(), "typing-inside.jsonl.xz": self.typing_rows()})
        rows, report = captured_curriculum(self.options(manifest))
        self.assertEqual(report["typing-inside.jsonl.xz"], {"excluded": True})
        converts = int(CAPTURED_FIXTURE_BUDGET * CAPTURED_FIXTURE_CONVERT_SHARE)
        self.assertEqual(sum(row.action == "convert" for row in rows), converts)
        self.assertEqual(sum(row.action == "keep" for row in rows), CAPTURED_FIXTURE_BUDGET - converts)
        self.assertEqual({row.sample_weight for row in rows}, {CAPTURED_FIXTURE_FILE_WEIGHT * CAPTURED_FIXTURE_WEIGHT_SCALE})
        self.assertEqual({row.category for row in rows}, {"captured_typing"})

    def test_words_no_lexicon_knows_have_their_share_of_the_keeps(self) -> None:
        manifest = self.manifest({"typing.jsonl.xz": self.typing_rows()})
        rows, report = captured_curriculum(self.options(manifest))
        keeps = [row.original for row in rows if row.action == "keep"]
        unknown = int(len(keeps) * CAPTURED_FIXTURE_KEEP_UNKNOWN_SHARE)
        self.assertEqual(sum(word in UNKNOWN_RUSSIAN for word in keeps), unknown)
        entry = report["typing.jsonl.xz"]
        assert isinstance(entry, dict)
        self.assertEqual((entry["unknown_keep_available"], entry["unknown_keep_chosen"]), (len(UNKNOWN_RUSSIAN), unknown))

    def test_only_the_layouts_the_recipe_names_have_a_share_of_unknown_keeps(self) -> None:
        latin = [question(word, LayoutPair().translate(word, "us", "ru"), 0, "keep") for word in ("zx", "qw")]
        manifest = self.manifest({"typing.jsonl.xz": [*self.typing_rows(), *latin]})
        _, report = captured_curriculum(self.options(manifest))
        entry = report["typing.jsonl.xz"]
        assert isinstance(entry, dict)
        self.assertEqual(entry["unknown_keep_available"], len(UNKNOWN_RUSSIAN))
        _, report = captured_curriculum({**self.options(manifest), "unknown_keep_groups": [0, 1]})
        entry = report["typing.jsonl.xz"]
        assert isinstance(entry, dict)
        self.assertEqual(entry["unknown_keep_available"], len(UNKNOWN_RUSSIAN) + len(latin))

    def test_without_a_share_a_file_keeps_its_own_proportion(self) -> None:
        typing = self.typing_rows()
        manifest = self.manifest({"typing.jsonl.xz": typing})
        rows, _ = captured_curriculum(self.options(manifest, convert_share=None))
        converts = int(CAPTURED_FIXTURE_BUDGET * len(KNOWN_RUSSIAN) / len(typing))
        self.assertEqual(sum(row.action == "convert" for row in rows), converts)

    def test_the_sample_is_the_same_on_every_run(self) -> None:
        manifest = self.manifest({"typing.jsonl.xz": self.typing_rows()})
        first, _ = captured_curriculum(self.options(manifest))
        second, _ = captured_curriculum(self.options(manifest))
        self.assertEqual(first, second)

    def test_a_lone_letter_is_left_out(self) -> None:
        # `b` is `и` typed in the English layout nearly every time in these files; taught so, a
        # model converted a capital letter in English prose.
        manifest = self.manifest({"typing.jsonl.xz": [convert("и"), question("b", "и", 0, "keep"), *self.typing_rows()]})
        rows, report = captured_curriculum(self.options(manifest))
        self.assertNotIn("b", {row.original for row in rows})
        entry = report["typing.jsonl.xz"]
        assert isinstance(entry, dict)
        self.assertEqual(entry["skipped_short"], len(("и", "b")))

    def test_planned_questions_follow_what_the_engine_can_ask(self) -> None:
        manifest = self.manifest({"typing.jsonl.xz": [
            # The engine plans a next word only once one was typed.
            question("ns", "ты", 0, "convert", origin="planned_next_conversion", after=" "),
            # A word longer than the planned context allows is asked with the field origin.
            question("ghbdtn", "привет", 0, "convert", origin="planned_next_conversion", after="мир"),
            # A short token with no word on either side takes the deferred action.
            question("ns", "ты", 0, "convert", before=""),
        ]})
        rows, report = captured_curriculum(self.options(manifest, convert_share=1.0))
        self.assertEqual(sorted((row.original, row.action, row.after_origin) for row in rows),
                         [("ghbdtn", "convert", "field"), ("ns", "wait", "none")])
        entry = report["typing.jsonl.xz"]
        assert isinstance(entry, dict)
        self.assertEqual(entry["chosen"], {"skipped_planned_without_word": 1, "convert": 1, "wait": 1})


if __name__ == "__main__":
    unittest.main()

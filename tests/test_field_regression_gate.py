"""The field regression gate (tools/field_regression_gate.py) on synthetic batteries: the owner's own words never
appear in a test, and the gate prints counts alone."""

from __future__ import annotations

import contextlib
import datetime
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import field_regression_gate as gate  # noqa: E402
from keyswitch.constants.release import FIELD_GATE_ERROR_EXIT_CODE, FIELD_GATE_RECORD_FILE_MODE  # noqa: E402

TIME = "2026-10-15 10:00:00,000"
# A moment after every run the tests make: its record gets a name of its own.
LATER_RUN = datetime.datetime.fromisoformat("2030-01-01T00:00:00+00:00")
LATER = "2026-10-15 10:00:01,000"


def row(index: int, shown: str, *, typed: str = "ghbdtn", other: str = "привет", intended: str = "привет",
        owner: bool = False, time: str = TIME, stream: int = 0, waits: bool = False) -> gate.Row:
    return gate.Row(stream, index, time, typed, other, intended, shown, owner, "0.0.0", waits)


def verdict(name: str, typed: str = "ghbdtn", *, by: str = "llm", code: str = "garbage-in-prompt",
            reason: str = "a reason", date: str = "2026-10-16") -> dict[str, object]:
    return {"verdict": name, "typed": typed, "by": by, "date": date, "reason_code": code, "reason": reason}


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *arguments], cwd=root, check=True,
                          capture_output=True, text=True).stdout.strip()


class TargetTests(unittest.TestCase):
    def test_the_reading_the_log_implies_holds_without_a_verdict(self) -> None:
        self.assertEqual(gate.target(row(0, "ghbdtn"), None), "привет")

    def test_a_verdict_reads_the_word_in_either_layout_it_was_typed(self) -> None:
        self.assertEqual(gate.target(row(0, "x"), verdict(gate.KEEP)), "ghbdtn")
        self.assertEqual(gate.target(row(0, "x"), verdict(gate.CONVERT)), "привет")
        # The same item typed in the other layout by another tree: the verdict names the same text.
        swapped = row(0, "x", typed="привет", other="ghbdtn")
        self.assertEqual(gate.target(swapped, verdict(gate.KEEP)), "ghbdtn")
        self.assertEqual(gate.target(swapped, verdict(gate.CONVERT)), "привет")
        self.assertIsNone(gate.target(row(0, "x"), verdict("ambiguous")))
        with self.assertRaisesRegex(gate.GateError, "neither reading"):
            gate.target(row(0, "x"), verdict(gate.KEEP, "zzz"))

    def test_only_the_owner_reads_a_word_the_owner_labelled_otherwise(self) -> None:
        owned = row(0, "x", owner=True)
        # An llm verdict against the owner's own label, or taking the word out, counts for nothing: the owner's stands.
        self.assertTrue(gate.owner_overruled(owned, verdict(gate.KEEP)))
        self.assertEqual(gate.target(owned, verdict(gate.KEEP)), "привет")
        self.assertTrue(gate.owner_overruled(owned, verdict("ambiguous")))
        self.assertEqual(gate.target(owned, verdict("ambiguous")), "привет")
        self.assertEqual(gate.target(owned, verdict(gate.KEEP, by=gate.OWNER)), "ghbdtn")
        self.assertIsNone(gate.target(owned, verdict("ambiguous", by=gate.OWNER)))
        # A verdict that agrees with the owner's own label needs nobody.
        self.assertEqual(gate.target(owned, verdict(gate.CONVERT)), "привет")
        self.assertFalse(gate.owner_overruled(owned, verdict(gate.CONVERT)))
        with self.assertRaisesRegex(gate.GateError, "neither reading"):
            gate.owner_overruled(owned, verdict(gate.KEEP, "zzz"))

    def test_a_word_the_owner_labelled_and_an_llm_verdict_overrules_is_listed_for_the_owner(self) -> None:
        released = {"pc": {(0, 0): row(0, "привет", owner=True)}}
        result = gate.compare(released, released, {("pc", 0, TIME): verdict(gate.KEEP)})
        self.assertEqual((result.rows, result.excluded, result.released_right, len(result.needs_owner)), (1, 0, 1, 1))
        self.assertEqual(result.needs_owner[0]["implied"], "привет")
        self.assertFalse(result.verdicts_used)


class VerdictTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "verdicts.json"

    def read(self, value: dict[str, object]) -> dict[tuple[str, int, str], dict[str, object]]:
        self.path.write_text(json.dumps({f"pc|0|{TIME}|ghbdtn": value}), encoding="utf-8")
        return gate.read_verdicts(self.path)

    def test_a_verdict_carries_its_author_date_and_reason(self) -> None:
        self.assertEqual(list(self.read(verdict(gate.KEEP))), [("pc", 0, TIME)])
        # The review before the gate recorded no reason per word.
        self.read(verdict(gate.KEEP, code=gate.REVIEW_BEFORE_GATE, reason=""))
        for broken in ({**verdict(gate.KEEP), "reason": ""}, {**verdict(gate.KEEP), "by": "someone"},
                       {**verdict(gate.KEEP), "date": ""}, {**verdict(gate.KEEP), "reason_code": "because"},
                       {**verdict(gate.KEEP), "verdict": "maybe"}):
            with self.subTest(verdict=broken), self.assertRaisesRegex(gate.GateError, "lacks"):
                self.read(broken)


class CompareTests(unittest.TestCase):
    def test_a_word_the_release_got_right_and_the_candidate_breaks_is_counted_and_listed(self) -> None:
        released = {"pc": {(0, 0): row(0, "привет"), (0, 1): row(1, "ghbdtn", time=LATER), (1, 0): row(0, "привет", stream=1)}}
        candidate = {"pc": {(0, 0): row(0, "ghbdtn"), (0, 1): row(1, "привет", time=LATER), (1, 0): row(0, "привет", stream=1)}}
        result = gate.compare(released, candidate, {})
        self.assertEqual(result.counts(), {"rows": len(released["pc"]), "excluded": 0, "released_right": len("ab"),
                                           "candidate_right": len("ab"), "fixed": 1, "broken": 1, "verdicts_used": 0,
                                           "new_verdicts_used": 0, "needs_owner": 0})
        self.assertEqual(result.broken[0]["candidate"], "ghbdtn")
        # The first word of its stream the trees typed differently; the next one may only follow from it.
        self.assertTrue(result.broken[0]["first_divergence"])
        self.assertFalse(result.fixed[0]["first_divergence"])

    def test_a_verdict_that_decides_a_word_is_listed_and_dated(self) -> None:
        released = {"pc": {(0, 0): row(0, "ghbdtn"), (0, 1): row(1, "ghbdtn", time=LATER)}}
        verdicts = {("pc", 0, TIME): verdict(gate.KEEP), ("pc", 0, LATER): verdict("ambiguous", date="2026-10-01")}
        result = gate.compare(released, released, verdicts, since="2026-10-15")
        self.assertEqual((result.rows, result.excluded, result.released_right), (1, 1, 1))
        self.assertEqual([word["verdict"] for word in result.verdicts_used], [gate.KEEP])
        self.assertEqual(result.new_verdicts_used, 1)
        self.assertEqual(gate.compare(released, released, verdicts).new_verdicts_used, 0)

    def test_a_word_asked_about_at_a_pause_and_typed_on_is_right_as_typed_or_in_the_layout_it_was_finished_in(self) -> None:
        # `Ф` of `AmoCRM_v2`: converted into the finished word's layout, or left; `сд` of `сделала` converted into `cl`.
        released = {"pc": {(0, 0): row(0, "Ф", typed="Ф", other="A", intended="A", waits=True),
                           (0, 1): row(1, "сд", typed="сд", other="cl", intended="сд", waits=True, time=LATER)}}
        candidate = {"pc": {(0, 0): row(0, "A", typed="Ф", other="A", intended="A", waits=True),
                            (0, 1): row(1, "cl", typed="сд", other="cl", intended="сд", waits=True, time=LATER)}}
        result = gate.compare(released, candidate, {})
        self.assertEqual((result.released_right, result.candidate_right, len(result.broken), len(result.fixed)),
                         (len(released["pc"]), 1, 1, 0))
        self.assertEqual((result.broken[0]["candidate"], result.broken[0]["waits"]), ("cl", True))
        # A verdict against a waiting word is listed when it decides it.
        verdicts = {("pc", 0, TIME): verdict(gate.KEEP, "Ф")}
        self.assertEqual(gate.compare(candidate, candidate, verdicts).verdicts_used[0]["verdict"], gate.KEEP)

    def test_trees_that_typed_different_words_stop_the_gate(self) -> None:
        with self.assertRaisesRegex(gate.GateError, "different words"):
            gate.compare({"pc": {(0, 0): row(0, "x")}}, {"pc": {(0, 1): row(1, "x")}}, {})


class FilesTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def test_a_report_with_one_word_twice_or_without_rows_stops_the_gate(self) -> None:
        report = self.root / "pc.json"
        report.write_text(json.dumps({"rows": [[0, 0, TIME, "a", "ф", "a", "a", False, "0"]] * len("ab")}), encoding="utf-8")
        with self.assertRaisesRegex(gate.GateError, "twice"):
            gate.read_rows(report)
        report.write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(gate.GateError, "no replay report"):
            gate.read_rows(report)
        # A word that waits carries a tenth field; a report of an older replay script has nine.
        report.write_text(json.dumps({"rows": [[0, 0, TIME, "a", "ф", "a", "a", False, "0", True],
                                               [0, 1, TIME, "a", "ф", "a", "a", False, "0"]]}), encoding="utf-8")
        self.assertEqual([word.waits for word in gate.read_rows(report).values()], [True, False])

    def test_the_manifest_names_private_files_relative_to_the_repository(self) -> None:
        with self.assertRaisesRegex(gate.GateError, "not on this machine"):
            gate.load_manifest(self.root / "missing.json", self.root)
        manifest = self.root / "manifest.json"
        manifest.write_text(json.dumps({"batteries": {}, "verdicts": "v", "replay": "r", "records": "o"}), encoding="utf-8")
        with self.assertRaisesRegex(gate.GateError, "no battery"):
            gate.load_manifest(manifest, self.root)
        manifest.write_text(json.dumps({"batteries": {"pc": "b/pc.jsonl"}, "verdicts": "v", "replay": "r", "records": "o",
                                        "environment": {"FIELD_ENTER": 1}}), encoding="utf-8")
        loaded = gate.load_manifest(manifest, self.root)
        self.assertEqual((loaded.batteries, loaded.environment), ({"pc": self.root / "b/pc.jsonl"}, {"FIELD_ENTER": "1"}))

    def test_a_replay_sees_its_own_tree_and_none_of_the_variables_that_swap_models(self) -> None:
        manifest = gate.Manifest({}, self.root, self.root, self.root, {"FIELD_ENTER": "1"})
        with patch.dict(os.environ, {"FIELD_CONTEXT_ARTIFACT": "other.json", "FIELD_STYLE": "x", "DISPLAY": ":1"}):
            environment = gate.replay_environment(self.root / "tree", manifest)
        self.assertEqual({name for name in environment if name.startswith(gate.REPLAY_VARIABLE_PREFIX)}, {"FIELD_ENTER"})
        self.assertNotIn("DISPLAY", environment)
        self.assertEqual(environment["PYTHONPATH"].split(os.pathsep)[0], str(self.root / "tree" / "src"))


class GateRunTests(unittest.TestCase):
    """A whole run on a repository of one release, with a replay that writes synthetic reports."""

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / "repository"
        self.root.mkdir()
        git(self.root, "init", "-q")
        (self.root / ".gitignore").write_text(".t/\n", encoding="utf-8")
        (self.root / "engine.txt").write_text("released", encoding="utf-8")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "release")
        git(self.root, "tag", "v0.0.1")
        private = self.root / ".t" / "field"
        private.mkdir(parents=True)
        (private / "battery-pc.jsonl").write_text("{}\n", encoding="utf-8")
        (private / "replay.py").write_text("# replay\n", encoding="utf-8")
        (private / "verdicts.json").write_text("{}", encoding="utf-8")
        self.manifest = gate.Manifest({"pc": private / "battery-pc.jsonl"}, private / "verdicts.json", private / "replay.py",
                                      private / "records", {"FIELD_ENTER": "1"})
        self.candidate = Path(directory.name) / "candidate"
        self.candidate.mkdir()
        self.replays: list[Path] = []

    def replay(self, candidate_shown: str) -> Callable[[Path, Path, Path, gate.Manifest], None]:
        def run(tree: Path, battery: Path, output: Path, manifest: gate.Manifest) -> None:
            self.replays.append(tree)
            shown = candidate_shown if tree == self.candidate else "привет"
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps({"rows": [[0, 0, TIME, "ghbdtn", "привет", "привет", shown, False, "0"]]}),
                              encoding="utf-8")
        return run

    def test_the_release_is_exported_once_and_its_replays_are_kept(self) -> None:
        comparison, record = gate.run_gate(self.manifest, self.candidate, "v0.0.1", root=self.root, run=self.replay("привет"))
        self.assertEqual((comparison.rows, len(comparison.broken)), (1, 0))
        tree = self.manifest.records / "trees" / "v0.0.1"
        self.assertEqual((tree / "engine.txt").read_text(encoding="utf-8"), "released")
        # Only the owner reads a record: it names the owner's words.
        self.assertEqual(record.stat().st_mode & FIELD_GATE_RECORD_FILE_MODE, FIELD_GATE_RECORD_FILE_MODE)
        self.assertFalse(record.stat().st_mode & (stat.S_IRWXG | stat.S_IRWXO))
        # A second run replays the candidate alone: the release's replays are kept.
        later = gate.run_gate(self.manifest, self.candidate, "v0.0.1", root=self.root, run=self.replay("ghbdtn"),
                              now=lambda: LATER_RUN)
        self.assertEqual(len(later[0].broken), 1)
        self.assertEqual(self.replays, [tree, self.candidate, self.candidate])
        # A directory in the export's place that is no export stops the gate.
        (tree / gate.TREE_MARKER).unlink()
        with self.assertRaisesRegex(gate.GateError, "no export"):
            gate.release_tree("v0.0.1", self.manifest.records, self.root)

    def test_records_where_git_does_not_ignore_them_stop_the_gate(self) -> None:
        exposed = gate.Manifest(self.manifest.batteries, self.manifest.verdicts, self.manifest.replay, self.root / "records", {})
        with self.assertRaisesRegex(gate.GateError, "not ignored"):
            gate.run_gate(exposed, self.candidate, "v0.0.1", root=self.root, run=self.replay("привет"))

    def test_main_prints_counts_alone_and_fails_closed(self) -> None:
        run_gate = gate.run_gate
        for shown, status in (("привет", 0), ("ghbdtn", 1)):
            output = io.StringIO()
            with (patch.object(gate, "load_manifest", return_value=self.manifest),
                  patch.object(gate, "run_gate", side_effect=lambda manifest, candidate, tag, shown=shown: run_gate(
                      manifest, candidate, tag, root=self.root, run=self.replay(shown),
                      now=lambda: LATER_RUN + datetime.timedelta(seconds=status))),
                  contextlib.redirect_stdout(output)):
                self.assertEqual(gate.main(["--candidate-root", str(self.candidate), "--released-ref", "v0.0.1"]), status)
            self.assertNotIn("привет", output.getvalue())
            self.assertNotIn("ghbdtn", output.getvalue())
        output = io.StringIO()
        with (patch.object(gate, "load_manifest", side_effect=ValueError("a word: привет")), contextlib.redirect_stdout(output)):
            self.assertEqual(gate.main([]), FIELD_GATE_ERROR_EXIT_CODE)
        self.assertEqual(output.getvalue().strip(), f"{gate.SUMMARY_PREFIX} could not run: ValueError")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (TargetTests, VerdictTests, CompareTests, FilesTests, GateRunTests):
        suite.addTests(loader.loadTestsFromTestCase(case))
    return suite


if __name__ == "__main__":
    unittest.main()

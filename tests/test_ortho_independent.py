"""Independent orthotactic tests built from text the development never saw (tools/ortho_independent.py).

Nothing here reads a frozen independent set or scores a model on one: every path is a temporary fixture.
"""
from __future__ import annotations

import gzip
import hashlib
import io
import json
import runpy
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stdout
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import ortho_corpus as corpus  # noqa: E402
import ortho_independent as independent  # noqa: E402
import wikisource_text  # noqa: E402
import ortho_known as known_tool  # noqa: E402
import train_ortho_model as trainer  # noqa: E402

from keyswitch.constants.model_protocol import TEST, TRAIN  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION,
    ORTHO_MIN_STRETCH_RUN,
    ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS,
)
from keyswitch.constants.units import PERCENT_SCALE  # noqa: E402
from keyswitch.ortho_model import OrthoModel, read_once  # noqa: E402
from fixture_values.ortho import (  # noqa: E402
    INDEPENDENT_FIXTURE_SAMPLE_MAX,
    INDEPENDENT_FIXTURE_SAMPLE_MIN,
    INDEPENDENT_FIXTURE_SAMPLE_PERCENT,
    INDEPENDENT_FIXTURE_SAMPLE_SENTENCES,
    POPULATION_FIXTURE_MINIMUM_LENGTH,
    POPULATION_FIXTURE_MINIMUM_RECALL,
    POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES,
)

# Besides words: a token longer than the corpus keeps, one mixing alphabets, one with a key outside the pair.
ENGLISH = "Zorblax met Hello there. " + "x" * (ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS + 1) + " abcабв café\n"
RUSSIAN = "Квазимодо сказал привет.\n"
PAGE = """{{Отексте
|АВТОР=[[Автор]]
|НАЗВАНИЕ=Рассказ {{lang|fr|titre}}
}}
__TOC__
<div class=text>
== I ==
<!-- правка -->Жили-были '''Гаврила''' и ''Марфа''.<ref>Сноска.</ref> — Ура-а-а! — кричал он. [[Москва (город)|Москва]] и [[Тверь]].
* Пункт&nbsp;списка [http://example.org ссылка] <span>тег</span>.
{| class="wikitable"
| ячейка
|}
</div>

== Примечания ==
Примечание не текст.
[[Категория:Рассказы]]
"""


class Score:
    def __init__(self, known: bool) -> None:
        self.known = known


class Dictionary:
    def __init__(self, words: set[str]) -> None:
        self.words = words

    def score(self, word: str) -> Score:
        return Score(word in self.words)


def pages_file(root: Path) -> Path:
    path = root / "pages.json"
    path.write_text(json.dumps({"Рассказ (Автор)": {"revid": 1, "timestamp": "t", "content": PAGE},
                                "Прочитанный (Автор)": {"revid": 1, "timestamp": "t", "content": "Прочитанное слово."}},
                               ensure_ascii=False), encoding="utf-8")
    return path


class Workspace(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "en.txt").write_text(ENGLISH, encoding="utf-8")
        (self.root / "ru.txt").write_text(RUSSIAN, encoding="utf-8")
        pages = pages_file(self.root)

        def pinned(locale: str, name: str, reader: str = independent.LINES) -> independent.Source:
            return independent.Source(locale, name, f"https://example.org/{name}", "2026-09-26",
                                      hashlib.sha256((self.root / name).read_bytes()).hexdigest(), "CC0", reader)

        sources = (pinned("eng", "en.txt"), pinned("rus", "ru.txt"), pinned("rus", pages.name, independent.WIKISOURCE))
        frozen_dir = self.root / "independent"
        stack = ExitStack()
        self.addCleanup(stack.close)
        for name, value in (("SOURCES", sources), ("DIRECTORY", frozen_dir), ("TOKENS", frozen_dir / "tokens.jsonl.gz"),
                            ("EVIDENCE", frozen_dir / "known-evidence.json.gz"), ("RECEIPT", frozen_dir / "receipt.json"),
                            ("HISTORY", ()), ("DEVELOPMENT_SETS", ()), ("EXCLUDED_PAGES", ("Прочитанный (Автор)",))):
            stack.enter_context(patch.object(independent, name, value))

    def rows(self) -> list[dict[str, object]]:
        return independent.rows_from(self.root)


class BuildTests(Workspace):
    def test_rows_follow_the_corpus_rules(self) -> None:
        rows = {(row["script"], row["keys"]): row for row in self.rows()}
        self.assertEqual(rows[("en", "zorblax")]["shapes"], {"initial": 1})
        self.assertEqual(rows[("ru", corpus.to_keys("квазимодо"))]["shapes"], {"initial": 1})
        self.assertEqual(rows[("ru", corpus.to_keys("гаврила"))]["shapes"], {"inner": 1})
        self.assertTrue(all(row["split"] == TEST for row in rows.values()))

    def test_tokens_the_corpus_drops_are_dropped(self) -> None:
        keys = {row["keys"] for row in self.rows()}
        self.assertNotIn("x" * (ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS + 1), keys)     # too long
        self.assertFalse(any("абв" in str(key) for key in keys))                    # mixed alphabets
        self.assertNotIn(corpus.to_keys("прочитанное"), keys)                        # a page left out
        original = corpus.to_keys
        with patch.object(corpus, "to_keys", side_effect=lambda raw: "" if raw == "met" else original(raw)):
            self.assertNotIn("met", {row["keys"] for row in self.rows()})           # no keys at all

    def test_a_source_that_is_not_the_pinned_file_is_refused(self) -> None:
        (self.root / independent.SOURCES[0].name).write_text("changed\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            self.rows()

    def test_a_wikisource_page_gives_the_sentences_of_its_text(self) -> None:
        self.assertEqual(wikisource_text.wikitext_sentences(PAGE), [
            "Жили-были Гаврила и Марфа.", "Ура-а-а!", "кричал он.", "Москва и Тверь.", "Пункт\xa0списка ссылка тег.",
        ])
        self.assertEqual(wikisource_text.wikitext_sentences("Просто текст! Без разметки"), ["Просто текст!", "Без разметки"])

    def test_a_share_of_sentences_is_kept_by_a_hash_of_the_sentence(self) -> None:
        sentences = [f"sentence {index}" for index in range(INDEPENDENT_FIXTURE_SAMPLE_SENTENCES)]
        kept = [sentence for sentence in sentences if independent.sampled(sentence, INDEPENDENT_FIXTURE_SAMPLE_PERCENT)]
        self.assertTrue(INDEPENDENT_FIXTURE_SAMPLE_MIN <= len(kept) <= INDEPENDENT_FIXTURE_SAMPLE_MAX)
        self.assertEqual(kept, [s for s in sentences if independent.sampled(s, INDEPENDENT_FIXTURE_SAMPLE_PERCENT)])
        self.assertTrue(all(independent.sampled(sentence, PERCENT_SCALE) for sentence in sentences))
        self.assertFalse(any(independent.sampled(sentence, 0) for sentence in sentences))
        halved = replace(independent.SOURCES[0], sample_percent=0)
        with patch.object(independent, "SOURCES", (halved,)):
            self.assertEqual(self.rows(), [])
        # A share an earlier set took from the same file, in its own namespace, is left out.
        taken = [sentence for sentence in sentences
                 if independent.sampled(sentence, INDEPENDENT_FIXTURE_SAMPLE_PERCENT, independent.PREVIOUS_SAMPLE_NAMESPACE)]
        self.assertTrue(taken and len(taken) < len(sentences))
        path = self.root / "many.txt"
        path.write_text("".join(f"{sentence}\n" for sentence in sentences), encoding="utf-8")
        source = independent.Source("eng", "many.txt", "url", "date", hashlib.sha256(path.read_bytes()).hexdigest(), "CC0",
                                    taken_percent=INDEPENDENT_FIXTURE_SAMPLE_PERCENT)
        kept = independent.sentences(source, self.root)
        self.assertEqual(set(kept), set(sentences) - set(taken))

    def test_the_verdict_answers_every_question_of_the_population_and_features(self) -> None:
        rows = self.rows()
        asked = independent.forms(rows)
        self.assertIn("zorblax", asked["en"])
        self.assertFalse(any("é" in word for word in asked["en"]))                    # a key outside the pair is never asked
        self.assertIn("квазимодо", asked["ru"])
        models = {"en": Dictionary({"hello", "there", "met"}), "ru": Dictionary({"сказал", "привет"})}
        with patch.object(known_tool, "reference_models", return_value=models):
            evidence = independent.verdict(rows)
        self.assertEqual(evidence["en"], ["hello", "met", "there"])
        self.assertEqual(evidence["ru"], ["привет", "сказал"])

    def test_types_the_development_may_have_seen_are_left_out(self) -> None:
        snapshot = [{"split": TRAIN, "script": "en", "keys": "zorblax", "shapes": {"lower": 1}},
                    {"split": corpus.LEXICON, "script": "en", "keys": "lexiconword", "shapes": {"lower": 1}}]
        history = self.root / "history"
        history.mkdir()
        corpus.write_tokens([{"split": TEST, "script": "ru", "keys": corpus.to_keys("квазимодо"), "shapes": {"initial": 1}}],
                            history / independent.TOKENS_NAME)
        scored = self.root / "scored"
        scored.mkdir()
        corpus.write_tokens([{"split": TEST, "script": "ru", "keys": corpus.to_keys("гаврила"), "shapes": {"inner": 1}}],
                            scored / independent.TOKENS_NAME)
        original = corpus.load_tokens
        with patch.object(corpus, "load_tokens", side_effect=lambda *path: original(*path) if path else snapshot), \
                patch.object(independent, "HISTORY", ((history, history / "report.json"),)), \
                patch.object(independent, "DEVELOPMENT_SETS", (scored,)):
            excluded = independent.seen()
        self.assertIn("zorblax", excluded)
        self.assertIn(corpus.to_keys("квазимодо"), excluded)                  # an opened set is seen
        self.assertIn(corpus.to_keys("гаврила"), excluded)                    # and a set the development scored
        self.assertNotIn("lexiconword", excluded)                            # word lists are not the snapshot
        self.assertIn("ctrl", excluded)                                       # looked at by hand
        self.assertIn(read_once(corpus.to_keys("фпривет"), ORTHO_MIN_STRETCH_RUN), excluded)
        rows = self.rows() + [{"split": TEST, "script": "en", "keys": "zorrrblax", "shapes": {"lower": 1}}]
        nothing: dict[str, frozenset[str]] = {"en": frozenset(), "ru": frozenset()}
        settings = {"minimum_length": POPULATION_FIXTURE_MINIMUM_LENGTH}
        with patch.object(independent, "seen", return_value=frozenset({"zorblax"})), \
                patch.object(trainer, "config", return_value=settings):
            kept, dropped = independent.population(rows, nothing)
        self.assertNotIn(("en", "zorblax", "initial"), kept)
        self.assertNotIn(("en", "zorrrblax", "lower"), kept)                 # a stretched spelling of a seen word
        self.assertEqual(dropped["en:negative_types"], len({"zorblax", "zorrrblax"}))
        self.assertIn(("ru", corpus.to_keys("квазимодо"), "initial"), kept)

    def test_the_receipt_pins_sources_counts_snapshot_opened_sets_and_code(self) -> None:
        rows = self.rows()
        kept = {("en", "zorblax", "initial"): True, ("ru", "zorblax", "initial"): False}
        dropped = {"en:negative_types": 1}
        snapshot = self.root / "snapshot.gz"
        snapshot.write_bytes(b"snapshot")
        with patch.object(independent, "population", return_value=(kept, dropped)), \
                patch.object(corpus, "TOKENS", snapshot):
            recorded = independent.receipt(rows, "tokens", "evidence", {"en": frozenset(), "ru": frozenset()})
        self.assertEqual((recorded["schema_version"], recorded["set"]), (ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION, independent.SET_NAME))
        self.assertEqual(recorded["types"], {"en:negative_types": 1, "ru:positive_types": 1})
        self.assertEqual(recorded["excluded_as_seen"], dropped)
        self.assertEqual(recorded["snapshot_tokens_sha256"], hashlib.sha256(b"snapshot").hexdigest())
        self.assertEqual(recorded["history_tokens_sha256"], {})
        source = cast(dict[str, object], recorded["source"])
        files = cast(list[dict[str, object]], source["files"])
        self.assertEqual([entry["sha256"] for entry in files], [item.sha256 for item in independent.SOURCES])
        self.assertEqual({entry["reader"] for entry in files}, {independent.LINES, independent.WIKISOURCE})
        self.assertEqual(source["excluded_pages"], ["Прочитанный (Автор)"])
        self.assertTrue({"фпривет", "ctrl"} <= set(cast(list[str], recorded["left_out_words"])))
        self.assertEqual(set(cast(dict[str, str], recorded["provenance"])), {*independent.CODE, "keyswitch.constants"})


class HistoryTests(Workspace):
    def opened(self) -> tuple[Path, Path, dict[str, object]]:
        directory = self.root / "opened"
        directory.mkdir()
        tokens_sha = corpus.write_tokens([{"split": TEST, "script": "en", "keys": "zorblax", "shapes": {"lower": 1}}],
                                         directory / independent.TOKENS_NAME)
        evidence_sha = known_tool.write({"en": ["hello"], "ru": []}, directory / independent.EVIDENCE_NAME)
        types = {"en:negative_types": 1}
        receipt = {"tokens_sha256": tokens_sha, "evidence_sha256": evidence_sha, "source": {"commit": "c"}, "types": types}
        (directory / independent.RECEIPT_NAME).write_text(json.dumps(receipt), encoding="utf-8")
        report = {"independent_test": {"receipt_sha256": hashlib.sha256((directory / independent.RECEIPT_NAME).read_bytes()).hexdigest(),
                                       "tokens_sha256": tokens_sha, "evidence_sha256": evidence_sha, "source": {"commit": "c"}},
                  "corpus_test": {"counts": {**types, "en:false_types": 1}}}
        path = self.root / "report.json"
        path.write_text(json.dumps(report), encoding="utf-8")
        return directory, path, report

    def test_an_opened_set_is_held_to_its_receipt_and_the_report_of_its_test(self) -> None:
        directory, path, report = self.opened()
        self.assertEqual(independent.history_matches(directory, path)["types"], {"en:negative_types": 1})
        described = cast(dict[str, object], report["independent_test"])
        for changed in ({**report, "corpus_test": {"counts": {"en:negative_types": 0}}},
                        {**report, "independent_test": {**described, "source": {"commit": "other"}}},
                        {**report, "independent_test": {**described, "receipt_sha256": "other"}}):
            with self.subTest(changed=changed):
                path.write_text(json.dumps(changed), encoding="utf-8")
                with self.assertRaises(ValueError):
                    independent.history_matches(directory, path)
        path.write_text(json.dumps(report), encoding="utf-8")
        (directory / independent.EVIDENCE_NAME).write_bytes(gzip.compress(b"{}", mtime=0))
        with self.assertRaises(ValueError):
            independent.history_matches(directory, path)


class FrozenTests(Workspace):
    def freeze(self) -> dict[str, object]:
        independent.DIRECTORY.mkdir()
        rows = self.rows()
        tokens_sha = corpus.write_tokens(rows, independent.TOKENS)
        evidence_sha = known_tool.write({"en": ["hello"], "ru": ["привет"]}, independent.EVIDENCE)
        recorded = {"schema_version": ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION, "set": independent.SET_NAME,
                    "tokens_sha256": tokens_sha, "evidence_sha256": evidence_sha, "snapshot_tokens_sha256": "snapshot",
                    "history_tokens_sha256": {}, "provenance": {"code": "pins"}, "source": {"files": []}}
        independent.RECEIPT.write_text(json.dumps(recorded), encoding="utf-8")
        return recorded

    def test_the_frozen_files_are_checked_against_their_receipt(self) -> None:
        recorded = self.freeze()
        with patch.object(independent, "checksum", side_effect=lambda path: "snapshot" if path == corpus.TOKENS
                          else hashlib.sha256(path.read_bytes()).hexdigest()), \
                patch.object(independent, "provenance", return_value={"code": "pins"}):
            rows, known, read = independent.frozen()
            self.assertEqual(read, recorded)
            self.assertEqual(known["ru"], frozenset({"привет"}))
            self.assertEqual(len(rows), len(self.rows()))
            for name, value in (("SET_NAME", "another"), ("HISTORY", ((self.root, self.root / "report.json"),))):
                with self.subTest(changed=name), patch.object(independent, name, value), \
                        patch.object(independent, "history_tokens", return_value={"opened": "sha"}):
                    with self.assertRaises(ValueError):
                        independent.frozen()
            independent.EVIDENCE.write_bytes(gzip.compress(b"{}", mtime=0))
            with self.assertRaises(ValueError):
                independent.frozen()

    def test_a_changed_snapshot_or_code_is_refused(self) -> None:
        self.freeze()
        with patch.object(independent, "provenance", return_value={"code": "changed"}):
            with self.assertRaises(ValueError):
                independent.frozen()

    def test_the_opened_sets_are_pinned_by_their_tokens(self) -> None:
        history = self.root / "history"
        history.mkdir()
        (history / independent.TOKENS_NAME).write_bytes(b"tokens")
        scored = self.root / "scored"
        scored.mkdir()
        (scored / independent.TOKENS_NAME).write_bytes(b"rows")
        with patch.object(independent, "HISTORY", ((history, history / "report.json"),)), \
                patch.object(independent, "DEVELOPMENT_SETS", (scored,)):
            self.assertEqual(independent.history_tokens(), {"history": hashlib.sha256(b"tokens").hexdigest(),
                                                            "scored": hashlib.sha256(b"rows").hexdigest()})


class SingleTestTests(Workspace):
    def arrange(self, seal_matches: bool = True, receipt_types: dict[str, int] | None = None) -> dict[str, object]:
        seal, candidate, report = self.root / "seal.json", self.root / "candidate.json", self.root / "report.json"
        candidate.write_text("{}", encoding="utf-8")
        seal.write_text(json.dumps({"provenance": {"code": "pins"}, "model_version": "ortho-v1-000000000000",
                                    "candidate_sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
                                    "thresholds": {"en": 1.0, "ru": 1.0}}), encoding="utf-8")
        independent.DIRECTORY.mkdir()
        self.receipt = independent.RECEIPT
        self.receipt.write_text("{}", encoding="utf-8")
        recorded: dict[str, object] = {"tokens_sha256": "tokens", "evidence_sha256": "evidence", "set": independent.SET_NAME,
                                       "source": {"files": []}, "types": receipt_types or {}}
        settings = {"minimum_length": POPULATION_FIXTURE_MINIMUM_LENGTH,
                    "promotion": {"minimum_test_negatives": POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES,
                                  "maximum_test_false_conversions": 0, "minimum_recall": POPULATION_FIXTURE_MINIMUM_RECALL}}
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(trainer, "SEAL", seal))
        stack.enter_context(patch.object(trainer, "CANDIDATE", candidate))
        stack.enter_context(patch.object(trainer, "REPORT", report))
        stack.enter_context(patch.object(trainer, "config", return_value=settings))
        stack.enter_context(patch.object(trainer, "provenance", return_value={"code": "pins" if seal_matches else "other"}))
        stack.enter_context(patch.object(independent, "frozen",
                                         return_value=([], {"en": frozenset(), "ru": frozenset()}, recorded)))
        stack.enter_context(patch.object(independent, "population", return_value=({}, {})))
        stack.enter_context(patch.object(OrthoModel, "load", return_value=object()))
        return recorded

    def counts(self, negatives: int, false: int, positives: int, recalled: int) -> dict[str, object]:
        return {"counts": {"en:negative_types": negatives, "en:false_types": false, "en:positive_types": positives,
                           "en:recalled_types": recalled}, "worst_false": {"en": [], "ru": []}}

    def test_a_passing_test_writes_the_trainers_report_bound_to_the_independent_files(self) -> None:
        recorded = self.arrange()
        with patch.object(trainer, "outcome", return_value=self.counts(POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES, 0, 1, 1)):
            report, passed = independent.test()
        self.assertTrue(passed)
        written = json.loads(trainer.REPORT.read_bytes())
        self.assertEqual(written, report)
        self.assertTrue(written["promotion_passed"])
        self.assertEqual(written["candidate_sha256"], hashlib.sha256(trainer.CANDIDATE.read_bytes()).hexdigest())
        described = written["independent_test"]
        self.assertEqual(described["receipt_sha256"], hashlib.sha256(self.receipt.read_bytes()).hexdigest())
        self.assertEqual((described["set"], described["tokens_sha256"], described["source"]),
                         (independent.SET_NAME, recorded["tokens_sha256"], recorded["source"]))
        with self.assertRaises(ValueError):                        # a second test is refused
            independent.test()

    def test_a_false_conversion_fails_the_gate(self) -> None:
        self.arrange()
        with patch.object(trainer, "outcome", return_value=self.counts(POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES, 1, 1, 1)):
            _report, passed = independent.test()
        self.assertFalse(passed)

    def test_a_changed_seal_is_refused(self) -> None:
        self.arrange(seal_matches=False)
        with self.assertRaises(ValueError):
            independent.test()

    def test_a_population_other_than_the_frozen_one_is_refused(self) -> None:
        self.arrange(receipt_types={"en:negative_types": 1})
        with patch.object(trainer, "outcome") as scored:
            with self.assertRaises(ValueError):
                independent.test()
        scored.assert_not_called()


class CommandLineTests(Workspace):
    def test_freeze_check_and_test(self) -> None:
        receipt = {"schema_version": ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION}
        opened = [(self.root, self.root / "report.json")]
        with patch.object(independent, "verdict", return_value={"en": [], "ru": []}), \
                patch.object(independent, "receipt", return_value=receipt), \
                patch.object(independent, "HISTORY", tuple(opened)), \
                patch.object(independent, "history_matches", return_value={}) as held, \
                patch.object(independent, "frozen", return_value=([], {}, receipt)), redirect_stdout(io.StringIO()) as out:
            self.assertEqual(independent.main(["--freeze", str(self.root)]), 0)
        held.assert_called_once_with(*opened[0])                  # every opened set is checked first
        self.assertEqual(json.loads(out.getvalue()), receipt)
        self.assertTrue(independent.TOKENS.exists() and independent.EVIDENCE.exists())
        self.assertEqual(json.loads(independent.RECEIPT.read_text(encoding="utf-8")), receipt)
        with self.assertRaises(ValueError):                        # frozen once
            independent.main(["--freeze", str(self.root)])
        for passed, code in ((True, 0), (False, 1)):
            with patch.object(independent, "test", return_value=({"promotion_passed": passed}, passed)), \
                    redirect_stdout(io.StringIO()):
                self.assertEqual(independent.main(["test"]), code)

    def test_the_command_line_entry_point_runs_main(self) -> None:
        with patch.object(sys, "argv", ["ortho_independent.py", "--help"]), redirect_stdout(io.StringIO()) as output:
            with self.assertRaises(SystemExit) as stopped:
                runpy.run_path(TOOLS_PATH + "/ortho_independent.py", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)
        self.assertIn("--freeze", output.getvalue())


class VerifierTests(Workspace):
    def test_the_verifier_holds_an_independent_report_to_its_frozen_files(self) -> None:
        import verify_ortho_model as verifier

        independent.DIRECTORY.mkdir()
        independent.RECEIPT.write_text("{}", encoding="utf-8")
        source: dict[str, object] = {"files": []}
        recorded = {"set": independent.SET_NAME, "tokens_sha256": "tokens", "evidence_sha256": "evidence", "source": source,
                    "types": {"en:negative_types": 1, "ru:positive_types": 1}}
        described = {"set": independent.SET_NAME, "receipt_sha256": hashlib.sha256(independent.RECEIPT.read_bytes()).hexdigest(),
                     "tokens_sha256": "tokens", "evidence_sha256": "evidence", "source": source}
        counts = {"en:negative_types": 1, "en:false_types": 0, "ru:positive_types": 1, "ru:recalled_types": 1}
        verifier.independent_matches({"promotion_passed": True})            # a test on a split: nothing to hold
        with patch.object(independent, "frozen", return_value=([], {}, recorded)):
            verifier.independent_matches({"independent_test": described, "corpus_test": {"counts": counts}})
            for changed_test, changed_counts in (({**described, "tokens_sha256": "other"}, counts),
                                                 ({**described, "set": "other"}, counts),
                                                 ({**described, "source": {"commit": "other"}}, counts),
                                                 (described, {**counts, "en:negative_types": 0})):
                with self.assertRaises(ValueError):
                    verifier.independent_matches({"independent_test": changed_test, "corpus_test": {"counts": changed_counts}})


if __name__ == "__main__":
    unittest.main()

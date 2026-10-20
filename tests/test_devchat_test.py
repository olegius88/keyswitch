"""The developer-chat test (tools/evaluate_devchat_test.py, tools/build_devchat_messages.py, tools/devchat_floors.py)
on synthetic messages: no message of the sealed test is read here, and none may reach a record, a result or stdout."""

from __future__ import annotations

import contextlib
import gzip
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import cast
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import build_devchat_messages as builder  # noqa: E402
import devchat_floors as floors  # noqa: E402
import evaluate_devchat_test as devchat  # noqa: E402
from keyswitch.constants.corpus import DEVCHAT_SIGNIFICANCE  # noqa: E402
from fixture_values.counts import (  # noqa: E402
    DEVCHAT_FIXTURE_LOSSES_BEYOND_CHANCE, DEVCHAT_FIXTURE_LOSSES_BY_CHANCE, DEVCHAT_FIXTURE_MESSAGES,
)

SECRET = "СЕКРЕТНОЕ сообщение with words"
# The real check, before the tests patch it.
TATOEBA_PASSED = devchat.tatoeba_passed


def table(ok: int, *, errors: int = 0) -> dict[str, dict[str, int]]:
    return {mode: {"n": DEVCHAT_FIXTURE_MESSAGES, "ok": ok, "errors": errors} for mode in floors.MODES}


def paired(losses: int = 0, gains: int = 0) -> dict[str, dict[str, int]]:
    return {mode: {"gains": gains, "losses": losses} for mode in floors.MODES}


class FloorTests(unittest.TestCase):
    def test_f1_needs_more_than_the_baseline_and_no_more_corruptions(self) -> None:
        full = DEVCHAT_FIXTURE_MESSAGES
        held = floors.floors({"candidate": table(full), "released": table(full), "baseline": table(full - 1)}, paired())
        self.assertEqual(held, {"every_message_typed": True, "f1_better_than_baseline": True, "f3_not_worse_than_released": True})
        tied = floors.floors({"candidate": table(full), "released": table(full), "baseline": table(full)}, paired())
        self.assertFalse(tied["f1_better_than_baseline"])

    def test_f3_tolerates_the_losses_chance_explains_and_no_more(self) -> None:
        self.assertEqual(floors.loss_chance(0, 0), 1.0)
        self.assertGreaterEqual(floors.loss_chance(DEVCHAT_FIXTURE_LOSSES_BY_CHANCE, 0), DEVCHAT_SIGNIFICANCE)
        self.assertLess(floors.loss_chance(DEVCHAT_FIXTURE_LOSSES_BEYOND_CHANCE, 0), DEVCHAT_SIGNIFICANCE)
        full = DEVCHAT_FIXTURE_MESSAGES
        tables = {"candidate": table(full), "released": table(full), "baseline": table(0)}
        self.assertTrue(floors.floors(tables, paired(DEVCHAT_FIXTURE_LOSSES_BY_CHANCE, DEVCHAT_FIXTURE_LOSSES_BY_CHANCE))[
            "f3_not_worse_than_released"])
        self.assertFalse(floors.floors(tables, paired(DEVCHAT_FIXTURE_LOSSES_BEYOND_CHANCE))["f3_not_worse_than_released"])
        # Fewer restored in all, or more corruptions, fails whatever the pairs disagree on.
        self.assertFalse(floors.floors({**tables, "candidate": table(full - 1)}, paired())["f3_not_worse_than_released"])
        self.assertFalse(floors.floors({**tables, "candidate": table(full, errors=1)}, paired())["every_message_typed"])


class BuilderTests(unittest.TestCase):
    def test_code_and_lookalike_messages_are_dropped(self) -> None:
        for token in ("a{b}", "x|y", "$var", "f());", "a===b"):
            self.assertTrue(builder.code_like(token), token)
        for token in ("ну...", "так!!!", ":-)", "(да)", "слово"):
            self.assertFalse(builder.code_like(token), token)
        self.assertTrue(builder.lookalike(["C", "помощью", "этого"]))
        self.assertTrue(builder.lookalike(["the", "с", "end"]))
        self.assertFalse(builder.lookalike(["c#", "и", "java"]))
        self.assertFalse(builder.lookalike(["c"]))

    def test_messages_go_to_one_split_by_question_with_their_members(self) -> None:
        splits = {builder.split_of(question) for question in range(DEVCHAT_FIXTURE_MESSAGES)}
        self.assertEqual(splits, {builder.DEVELOPMENT, builder.TEST})
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            bodies = ["<p>Используйте git rebase для этого случая</p>", "Используйте git rebase для этого случая",
                      "Попробуйте $var в коде вместо этого", "Это уже было в старом дампе раньше"]
            comments: list[dict[str, object]] = [{"post_id": index, "body": body} for index, body in enumerate(bodies, 1)]
            known = {builder.text_key(bodies[-1])}
            # The second comment is on an answer to the first one's question.
            answer = {int(str(comments[1]["post_id"])): int(str(comments[0]["post_id"]))}
            counts = builder.build(comments, answer, known, out)
            self.assertEqual(counts["dropped:code"], 1)
            self.assertEqual(counts["dropped:dump"], 1)
            kept = [json.loads(line) for split in (builder.DEVELOPMENT, builder.TEST)
                    for line in gzip.open(out / f"devchat-{split}.members.jsonl.gz", "rt", encoding="utf-8")]
            self.assertEqual(kept, [{"text": builder.text_key("Используйте git rebase для этого случая"), "question": 1,
                                     "mix": builder.MIXED}])
            first = builder.content_checksum(out / f"devchat-{builder.split_of(1)}.messages.jsonl.gz")
            builder.build(comments, answer, known, out)
            self.assertEqual(builder.content_checksum(out / f"devchat-{builder.split_of(1)}.messages.jsonl.gz"), first)
            # A later batch drops every text an earlier build holds.
            earlier = frozenset(str(row["text"]) for split in (builder.DEVELOPMENT, builder.TEST)
                                for row in map(json.loads, gzip.open(out / f"devchat-{split}.members.jsonl.gz", "rt", encoding="utf-8")))
            self.assertEqual(builder.build(comments, answer, known, out / "later", earlier),
                             {"dropped:code": counts["dropped:code"], "dropped:dump": counts["dropped:dump"],
                              "dropped:earlier": len(earlier)})


class ProtocolTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.messages = self.root / "messages"
        self.ledger = self.root / "ledger"
        texts = [f"{SECRET} {index}" for index in range(DEVCHAT_FIXTURE_MESSAGES)]
        self.texts = texts
        comments = [{"post_id": index, "body": text} for index, text in enumerate(texts)]
        builder.build(comments, {}, set(), self.messages)
        self.models = {}
        for name in floors.PAIRS:
            path = self.root / f"{name}.json"
            path.write_text(json.dumps({"version": f"context-v3-{name}"}), encoding="utf-8")
            self.models[name] = path
        pins = {split: devchat.content_checksum(self.messages / f"devchat-{split}.messages.jsonl.gz")
                for split in (devchat.DEVELOPMENT, devchat.TEST)}
        self.batches = self.root / "batches.json"
        self.batches.write_text(json.dumps({"batches": []}), encoding="utf-8")
        # A read takes every message with Latin words the fixture's test holds.
        count = sum(member.mix == devchat.MIXED for member in devchat.load_members(self.messages / "devchat-test.members.jsonl.gz"))
        for patcher in (patch.dict(devchat.PINNED, pins), patch.object(devchat, "runtime", return_value={"src": "pins"}),
                        patch.object(devchat, "tatoeba_passed", return_value=True), patch.object(devchat, "TEST_BATCHES", self.batches),
                        patch.object(devchat, "DEVCHAT_TEST_COUNT", count)):
            patcher.start()
            self.addCleanup(patcher.stop)

    def typed(self, outcomes: dict[str, bool]) -> contextlib.AbstractContextManager[object]:
        def run(model: Path, texts: list[str], workers: int) -> list[devchat.Row]:
            name = model.stem
            return [(index, mode, outcomes[name]) for index in range(len(texts)) for mode in floors.MODES]
        return patch.object(devchat, "typed", side_effect=run)

    def evaluate(self, split: str) -> dict[str, object]:
        return devchat.evaluate(split, self.messages, self.models, root=self.ledger)

    def test_the_test_is_read_once_after_a_recorded_development_gain(self) -> None:
        with self.assertRaisesRegex(devchat.Refusal, "no development result"):
            self.evaluate(devchat.TEST)
        with self.typed({"candidate": True, "released": True, "baseline": False}):
            development = self.evaluate(devchat.DEVELOPMENT)
            self.assertTrue(all(cast(dict[str, bool], development["floors"]).values()))
            result = self.evaluate(devchat.TEST)
        self.assertTrue(all(cast(dict[str, bool], result["floors"]).values()))
        # A second read of the same candidate gives the recorded result, and reads nothing.
        with patch.object(devchat, "typed", side_effect=AssertionError("read again")):
            self.assertEqual(self.evaluate(devchat.TEST), result)
        # Nothing of a message reaches the ledger.
        for path in self.ledger.iterdir():
            self.assertNotIn(SECRET, path.read_text(encoding="utf-8"))

    def test_a_development_result_without_a_gain_or_on_another_runtime_reads_no_test(self) -> None:
        with self.typed({"candidate": False, "released": True, "baseline": True}):
            self.evaluate(devchat.DEVELOPMENT)
        with self.assertRaisesRegex(devchat.Refusal, "holds no gain"):
            self.evaluate(devchat.TEST)
        with self.typed({"candidate": True, "released": True, "baseline": False}):
            self.evaluate(devchat.DEVELOPMENT)
        with patch.object(devchat, "runtime", return_value={"src": "changed"}), \
                self.assertRaisesRegex(devchat.Refusal, "runtime or the pairs differ"):
            self.evaluate(devchat.TEST)
        with patch.object(devchat, "tatoeba_passed", return_value=False), \
                self.assertRaisesRegex(devchat.Refusal, "Tatoeba"):
            self.evaluate(devchat.TEST)

    def test_a_used_up_test_claims_nothing_and_a_later_batch_is_drawn_from(self) -> None:
        with self.typed({"candidate": True, "released": True, "baseline": False}):
            self.evaluate(devchat.DEVELOPMENT)
        taken = devchat.load_members(self.messages / "devchat-test.members.jsonl.gz")
        devchat.claim({"pairs": {"candidate": {"sha256": "earlier"}}, "test_membership": devchat.membership(taken)}, self.ledger)
        claims = sorted(self.ledger.glob("*.access.json"))
        with self.assertRaisesRegex(devchat.Refusal, "new batch"):
            self.evaluate(devchat.TEST)
        self.assertEqual(sorted(self.ledger.glob("*.access.json")), claims)
        # A batch of fresh comments, built apart and pinned in the registry: the read draws from it.
        later = self.root / "later"
        builder.build([{"post_id": len(self.texts) + index, "body": f"{SECRET} later {index}"} for index in range(len(self.texts))],
                      {}, set(), later)
        for kind in ("messages", "members"):
            (self.messages / f"devchat-test-2.{kind}.jsonl.gz").write_bytes((later / f"devchat-test.{kind}.jsonl.gz").read_bytes())
        self.batches.write_text(json.dumps({"batches": [{"name": "devchat-test-2", "content_sha256": devchat.content_checksum(
            self.messages / "devchat-test-2.messages.jsonl.gz")}]}), encoding="utf-8")
        fresh = devchat.load_members(self.messages / "devchat-test-2.members.jsonl.gz")
        with self.typed({"candidate": True, "released": True, "baseline": False}), \
                patch.object(devchat, "DEVCHAT_TEST_COUNT", sum(member.mix == devchat.MIXED for member in fresh)):
            result = self.evaluate(devchat.TEST)
        self.assertEqual(result["messages"], len(fresh))
        self.assertTrue(all(cast(dict[str, bool], result["floors"]).values()))
        # A batch whose messages differ from its pin is refused.
        self.batches.write_text(json.dumps({"batches": [{"name": "devchat-test-2", "content_sha256": "0" * len(devchat.PINNED[
            devchat.TEST])}]}), encoding="utf-8")
        with self.assertRaisesRegex(devchat.Refusal, "differ from the pinned"):
            self.evaluate(devchat.TEST)

    def test_an_earlier_claim_that_drew_no_message_is_no_read_of_its_candidate(self) -> None:
        with self.typed({"candidate": True, "released": True, "baseline": False}):
            self.evaluate(devchat.DEVELOPMENT)
        candidate = devchat.checksum(self.models["candidate"])
        empty = {"pairs": {"candidate": {"sha256": candidate}}, "test_membership": {"messages": [], "questions": []}}
        key = devchat.claim(empty, self.ledger)
        (self.ledger / f"{key}.result.json").write_text(json.dumps({"messages": 0, "error": "KeyError"}), encoding="utf-8")
        with self.typed({"candidate": True, "released": True, "baseline": False}):
            result = self.evaluate(devchat.TEST)
        self.assertGreater(cast(int, result["messages"]), 0)
        self.assertNotIn("error", result)

    def test_the_same_models_at_other_paths_are_the_same_pairs(self) -> None:
        recorded = {"candidate": {"path": "/abs/candidate.json", "sha256": "a", "model_version": "context-v3-a"}}
        self.assertTrue(devchat.same_models(recorded, {"candidate": {"path": "candidate.json", "sha256": "a",
                                                                     "model_version": "context-v3-a"}}))
        self.assertFalse(devchat.same_models(recorded, {"candidate": {"path": "/abs/candidate.json", "sha256": "b",
                                                                      "model_version": "context-v3-a"}}))

    def test_reads_never_overlap_and_a_failure_is_recorded_by_kind(self) -> None:
        members = devchat.load_members(self.messages / "devchat-test.members.jsonl.gz")
        identity = {"pairs": {"candidate": {"sha256": "other"}}, "test_membership": devchat.membership(members)}
        devchat.claim(identity, self.ledger)
        with self.assertRaisesRegex(devchat.Refusal, "already"):
            devchat.claim(identity, self.ledger)
        fresh = {"pairs": {"candidate": {"sha256": "third"}}, "test_membership": devchat.membership(members)}
        with self.assertRaisesRegex(devchat.Refusal, "overlaps"):
            devchat.claim(fresh, self.ledger)
        self.assertEqual(devchat.consumed(self.ledger)[0], set(devchat.membership(members)["messages"]))
        self.assertEqual(devchat.draw(members, None, 1.0, devchat.consumed(self.ledger)), [])

    def test_a_worker_failure_comes_back_as_its_kind_alone(self) -> None:
        import mixed_typing
        job = (0, SECRET, "correct", 0, "")
        with patch.object(mixed_typing, "_evaluate", side_effect=KeyError(SECRET)):
            self.assertEqual(devchat.safe_evaluate(job), (0, "correct", "KeyError"))
        with patch.object(mixed_typing, "_evaluate", return_value={"skipped": True}):
            self.assertEqual(devchat.safe_evaluate(job), (0, "correct", "skipped"))
        rows: list[devchat.Row] = [(0, "correct", True), (0, "forget_ru", "skipped"), (1, "correct", "KeyError")]
        members = [devchat.Member("a", "q", devchat.MIXED), devchat.Member("b", "q", devchat.RUSSIAN)]
        counted = devchat.counts(rows, members)
        self.assertEqual(counted["correct"], {"n": len(members), "ok": 1, "errors": 1})
        self.assertNotIn("forget_ru", counted)
        released: list[devchat.Row] = [(0, "correct", False), (0, "forget_ru", "skipped"), (1, "correct", True)]
        self.assertEqual(devchat.discordance(rows, released)["correct"], {"gains": 1, "losses": 1})

    def test_main_prints_counts_alone(self) -> None:
        output = io.StringIO()
        with self.typed({"candidate": True, "released": True, "baseline": False}), \
                patch.object(devchat, "ledger_root", return_value=self.ledger), contextlib.redirect_stdout(output):
            status = devchat.main(["--split", "dev", "--directory", str(self.messages), "--candidate", str(self.models["candidate"]),
                                   "--released", str(self.models["released"]), "--baseline", str(self.models["baseline"]),
                                   "--output", str(self.root / "result.json")])
        self.assertEqual(status, 0)
        self.assertNotIn(SECRET, output.getvalue())
        output = io.StringIO()
        with patch.object(devchat, "evaluate", side_effect=KeyError(SECRET)), contextlib.redirect_stdout(output):
            self.assertEqual(devchat.main(["--split", "test", "--directory", str(self.messages), "--candidate",
                                           str(self.models["candidate"]), "--output", str(self.root / "r.json")]), 1)
        self.assertEqual(output.getvalue().strip(), "devchat test failed: KeyError")

    def test_the_tatoeba_test_counts_only_when_read_and_passed(self) -> None:
        import evaluate_context_action_sequences as sequences
        ledger = self.root / "tatoeba"
        ledger.mkdir()
        (ledger / "k.access.json").write_text(json.dumps({"artifact_sha256": "abc"}), encoding="utf-8")
        with patch.object(sequences, "LEDGER_ROOT", ledger):
            self.assertFalse(TATOEBA_PASSED("abc"))
            (ledger / "k.outcome.json").write_text(json.dumps({"split": "test", "promotion_passed": False}), encoding="utf-8")
            self.assertFalse(TATOEBA_PASSED("abc"))
            (ledger / "k.outcome.json").write_text(json.dumps({"split": "test", "promotion_passed": True}), encoding="utf-8")
            self.assertTrue(TATOEBA_PASSED("abc"))
            self.assertFalse(TATOEBA_PASSED("other"))


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (FloorTests, BuilderTests, ProtocolTests):
        suite.addTests(loader.loadTestsFromTestCase(case))
    return suite


if __name__ == "__main__":
    unittest.main()

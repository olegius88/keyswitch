"""The hypothesis head (`тыв` after `tot `, the engine's guess that `еще` was typed for `tot`) and the short-token head (`кз`
after Russian text): their classes and evidence, the engine's mark on its hypothesis, the table of short Russian tokens,
the captured questions of schema 2 and the frames both heads are fitted on."""

from __future__ import annotations

import gzip
import hashlib
import json
import lzma
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
import train_context_model  # noqa: E402
from train_context_model import CAPTURED_COLUMNS, CAPTURED_COLUMNS_V2  # noqa: E402
from keyswitch.constants.corpus import CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION, CAPTURED_SCHEMA_VERSION  # noqa: E402
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN  # noqa: E402
from fixture_values.counts import (  # noqa: E402
    CAPTURE_FIXTURE_QUESTIONS, SHORT_FIXTURE_COUNTS, SHORT_FIXTURE_TYPO_DRAWS, SHORT_FIXTURE_TYPO_MIN_LETTERS,
)
from keyswitch.constants.models import (  # noqa: E402
    HYPOTHESIS_FEATURE_PREFIX, HYPOTHESIS_RATIO_BOUND, SHORT_FEATURE_PREFIX, SHORT_HEAD_LETTERS,
)
from keyswitch.context_action_features import (  # noqa: E402
    extract_action_features, hypothesis_question, nearest_count, short_question,
)
from keyswitch.context_model import (  # noqa: E402
    AfterOrigin, SHORT_ALPHABETS, SHORT_FREQUENCY_PATH, ContextEvidence, ContextModel, _short_frequency, _term_frequency,
)
from keyswitch.input_context import FieldContext  # noqa: E402
from keyswitch.detector import DetectionDecision, LanguageDetector  # noqa: E402
from keyswitch.word_decision import automatic_word_decision  # noqa: E402
from test_context_training_captured import captured_line, captured_question  # noqa: E402
from test_default_input_sequences import DefaultSequenceHarness  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


def hypothesis(original: str, alternative: str, group: int, before: str, *, target_identifier: bool = False,
               after_origin: AfterOrigin = "none", field: FieldContext | None = None) -> ContextEvidence:
    item = evidence(original, alternative, group, before)
    return replace(item, before_origin="planned_previous_reading", target_identifier=target_identifier,
                   after_origin=after_origin, field=field or item.field)


class HypothesisClassTests(unittest.TestCase):
    def test_only_the_engine_s_mark_makes_the_question_the_head_s(self) -> None:
        self.assertTrue(hypothesis_question(hypothesis("тыв", "nsd", 1, "tot ")))
        # The same text typed after a word the person switched the layout for is the frozen model's.
        self.assertFalse(hypothesis_question(evidence("тыв", "nsd", 1, "tot ")))
        self.assertEqual(prefixes(extract_action_features(evidence("тыв", "nsd", 1, "tot "))), set())

    def test_a_typo_of_a_common_word_has_a_common_neighbour(self) -> None:
        russian, english = _term_frequency()["russian"], _term_frequency()["english"]
        self.assertEqual(nearest_count("тыв", "russian"), max(russian["ты"], russian.get("тыв", 0), nearest_count("тыв", "russian")))
        self.assertGreaterEqual(nearest_count("тыв", "russian"), russian["ты"])
        self.assertLess(nearest_count("nsd", "english"), russian["ты"])
        self.assertGreaterEqual(nearest_count("thik", "english"), english["think"])
        self.assertEqual(nearest_count("t1", "english"), 0)
        self.assertEqual(nearest_count("", "russian"), 0)

    def test_the_head_reads_how_common_each_reading_and_its_neighbours_are(self) -> None:
        typed = extract_action_features(hypothesis("тыв", "nsd", 1, "tot ", target_identifier=True))
        self.assertEqual(prefixes(typed), {HYPOTHESIS_FEATURE_PREFIX})
        direction = ":direction:1"
        for name in ("bias", "word:0", f"near:{HYPOTHESIS_RATIO_BOUND}", "seen:0:0", "identifier:0:1",
                     f"waiting:{HYPOTHESIS_RATIO_BOUND}", "letters:3", "boundary:space"):
            self.assertIn(HYPOTHESIS_FEATURE_PREFIX + name + direction, typed)
        # An English word typed in the Russian layout after the reading of its first word: its keys are the commoner word.
        english = extract_action_features(hypothesis("тщеб", "not,", 1, "If "))
        self.assertTrue(any(name.startswith(HYPOTHESIS_FEATURE_PREFIX + "word:-") for name in english))
        # With no word before it the waiting word is no word either.
        self.assertIn(HYPOTHESIS_FEATURE_PREFIX + "waiting:0" + direction,
                      extract_action_features(hypothesis("тыв", "nsd", 1, "≈ ")))
        # Typed in the English layout, after the Cyrillic reading of the waiting word.
        latin = extract_action_features(hypothesis("nsd", "тыв", 0, "еще "))
        self.assertIn(HYPOTHESIS_FEATURE_PREFIX + "bias:direction:0", latin)

    def test_a_question_with_a_hypothesis_before_it_has_no_planned_or_kept_right_context(self) -> None:
        with self.assertRaisesRegex(ValueError, "left-context origin"):
            extract_action_features(replace(evidence("тыв", "nsd", 1, "tot "), before_origin="typed"))  # type: ignore[arg-type]
        with self.assertRaisesRegex(ValueError, "left-context origin"):
            extract_action_features(hypothesis("тыв", "nsd", 1, "tot ", after_origin="planned_next_conversion",
                                               field=FieldContext("Code", "test", "tot ", "можешь", "text")))


class ShortClassTests(unittest.TestCase):
    def test_the_class_is_a_short_token_russian_text_uses_whose_latin_reading_is_rare(self) -> None:
        for original in ("кз", "дз"):
            with self.subTest(original=original):
                self.assertTrue(short_question(original, "говорит он "))
        # Capitals, one letter, four letters, a reading with a sign key (`хз` is `[p`), after English text or none, a term
        # the term head answers (`гш` for `ui`), a common Latin reading (`ьк` is `mr`, `ин` `by`, `ср` `ch`, `тз` `np`) and
        # a token Russian text hardly uses (`ге`).
        for original, before in (("КЗ", "говорит он "), ("к", "говорит он "), ("кзкз", "говорит он "), ("хз", "говорит он "),
                                 ("кз", "he said "), ("кз", ""), ("гш", "говорит он "), ("rp", "говорит он "),
                                 ("ьк", "был с "), ("ин", "посмотри "), ("ср", "говорит он "), ("тз", "говорит он "),
                                 ("ге", "кирилл тоже ")):
            with self.subTest(original=original, before=before):
                self.assertFalse(short_question(original, before))

    def test_the_table_counts_short_russian_tokens_of_web_and_news_text(self) -> None:
        table = _short_frequency()
        self.assertEqual(sorted(table), list(SHORT_ALPHABETS))
        self.assertTrue(SHORT_FREQUENCY_PATH.is_file())
        self.assertIn("кз", table["cyrillic"])
        self.assertNotIn("сщ", table["cyrillic"])
        self.assertTrue(all(len(token) in SHORT_HEAD_LETTERS for token in table["cyrillic"]))

    def test_the_head_reads_counts_and_the_english_lexicon_but_no_letter(self) -> None:
        typed = extract_action_features(replace(evidence("кз", "rp", 1, "говорит он "), target_known=True))
        self.assertEqual(prefixes(typed), {SHORT_FEATURE_PREFIX})
        for name in ("bias", "russian:0", "english:0", "seen:1:0:1:0", "known:1:identifier:0", "letters:2", "boundary:space",
                     "after:none"):
            self.assertIn(SHORT_FEATURE_PREFIX + name, typed)
        self.assertFalse(any("кз" in name or "rp" in name for name in typed if name.startswith(SHORT_FEATURE_PREFIX)))
        # A word the lexicon knows is the frozen model's, and so is a question asked with a hypothesis before the token.
        known = extract_action_features(replace(evidence("дз", "lp", 1, "говорит он "), source_known=True))
        self.assertNotIn(SHORT_FEATURE_PREFIX + "bias", known)
        self.assertNotIn(SHORT_FEATURE_PREFIX + "bias", extract_action_features(hypothesis("кз", "rp", 1, "говорит он ")))

    def test_a_model_with_the_head_reads_the_table_when_it_loads(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.json"
            source = json.loads(Path("src/keyswitch/resources/models/context_policy_v1.json").read_text(encoding="utf-8"))
            weights = {**source["weights"], SHORT_FEATURE_PREFIX + "bias": [0.0, 0.0, 0.0, 0.0]}
            checksum = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
            path.write_text(json.dumps({**source, "weights": weights, "weights_sha256": checksum}), encoding="utf-8")
            with patch("keyswitch.context_model._short_frequency") as read:
                ContextModel.load(path)
            read.assert_called_once_with()


class HypothesisEngineTests(DefaultSequenceHarness):
    def test_the_engine_marks_the_reading_it_puts_before_the_next_word(self) -> None:
        with self.session(1, "Code") as session:
            policy = session.engine.context_policy
            decide = policy.decide
            asked: list[tuple[str, str]] = []

            def recorded(baseline: DetectionDecision, *args: object, **kwargs: object):  # type: ignore[no-untyped-def]
                asked.append((baseline.original, str(kwargs.get("before_origin", "field"))))
                return decide(baseline, *args, **kwargs)  # type: ignore[arg-type]

            with patch.object(policy, "decide", recorded):
                session.type("еще ", 1, idle_after_words=False)
                session.type("тыв ", 1, idle_after_words=False)
        self.assertIn(("тыв", "planned_previous_reading"), asked)
        self.assertIn(("тыв", "field"), asked)

    def test_the_policy_passes_the_mark_to_the_evidence(self) -> None:
        with self.session(1, "Code") as session:
            policy = session.engine.context_policy
            assert policy.model is not None
            seen: list[ContextEvidence] = []
            predict = policy.model.predict

            def recorded(item: ContextEvidence):  # type: ignore[no-untyped-def]
                seen.append(item)
                return predict(item)

            baseline = session.engine._planned_baseline("тыв", {0: "nsd"}, 1, "tot", 0)
            with patch.object(policy.model, "predict", recorded):
                policy.decide(baseline, "nsd", 0, session.engine.detector, "space", "assist",
                              field_override=FieldContext("Code", "test", "tot ", "", "text"),
                              before_origin="planned_previous_reading")
        self.assertEqual([item.before_origin for item in seen], ["planned_previous_reading"])


def _detector() -> LanguageDetector:
    from keyswitch.language_model import LanguageModel
    from keyswitch.lexicon_supplement import supplement_words

    return LanguageDetector({0: LanguageModel.load("en_US"), 1: LanguageModel.load("ru_RU", supplement_words("ru_RU"))})


class CapturedSchemaTwoTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, name: str, lines: list[str], schema: int, columns: tuple[str, ...]) -> Path:
        data = self.root / name
        data.write_bytes(lzma.compress("".join(lines).encode()))
        manifest = self.root / f"manifest-{name}.json"
        manifest.write_text(json.dumps({"schema_version": schema, "columns": list(columns), "sources": [
            {"file": data.name, "sha256": hashlib.sha256(data.read_bytes()).hexdigest(), "weight": 1.0}]}), encoding="utf-8")
        return manifest

    def test_a_manifest_of_schema_two_records_how_the_text_before_was_given(self) -> None:
        line = json.loads(captured_line(1, "тыв", "nsd", 1, "keep", "tot "))
        row = dict(zip(CAPTURED_COLUMNS, line, strict=True))
        marked = json.dumps([{**row, "before_origin": "planned_previous_reading"}[name] for name in CAPTURED_COLUMNS_V2],
                            ensure_ascii=False) + "\n"
        manifest = self.write("hypothesis.jsonl.xz", [marked, captured_line(1, "кз", "rp", 1, "keep", "говорит он ")
                                                      .replace('"none",', '"none","field",', 1)], CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION, CAPTURED_COLUMNS_V2)
        [source] = train_context_model.captured_sources(manifest)
        self.assertEqual(source.columns, CAPTURED_COLUMNS_V2)
        items = [item for item, _label, _count in train_context_model.captured_rows(source.path, source.columns)]
        self.assertEqual([item.before_origin for item in items], ["planned_previous_reading", "field"])
        # The columns must be the schema's.
        broken = self.write("broken.jsonl.xz", [marked], CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION, CAPTURED_COLUMNS)
        with self.assertRaisesRegex(ValueError, "manifest"):
            train_context_model.captured_sources(broken)
        old = self.write("old.jsonl.xz", [captured_line(1, "тыв", "nsd", 1, "keep", "tot ")], CAPTURED_SCHEMA_VERSION, CAPTURED_COLUMNS)
        [source] = train_context_model.captured_sources(old)
        [(item, _label, _count)] = list(train_context_model.captured_rows(source.path, source.columns))
        self.assertEqual(item.before_origin, "field")

    def test_the_heads_take_their_frames_and_leave_out_the_held_tokens(self) -> None:
        line = json.loads(captured_line(1, "тыв", "nsd", 1, "keep", "tot "))

        def marked(original: str, alternative: str, label: str, before: str) -> str:
            row = dict(zip(CAPTURED_COLUMNS, json.loads(captured_line(1, original, alternative, 1, label, before)), strict=True))
            return json.dumps([{**row, "before_origin": "planned_previous_reading"}[name] for name in CAPTURED_COLUMNS_V2],
                              ensure_ascii=False) + "\n"

        def plain(original: str, alternative: str, label: str, before: str) -> str:
            row = dict(zip(CAPTURED_COLUMNS, json.loads(captured_line(1, original, alternative, 1, label, before)), strict=True))
            return json.dumps([{**row, "before_origin": "field"}[name] for name in CAPTURED_COLUMNS_V2], ensure_ascii=False) + "\n"

        self.assertEqual(len(line), len(CAPTURED_COLUMNS))
        manifest = self.write("frames.jsonl.xz", [
            marked("тыв", "nsd", "keep", "tot "), marked("вдеет", "dlttn", "keep", "jy "), marked("тщеб", "not,", "convert", "If "),
            plain("кз", "rp", "keep", "говорит он "), plain("дз", "lp", "keep", "сделал "), plain("ин", "by", "convert", "сделано "),
            plain("пк", "gr", "convert", "сделано "),
            marked("дз", "lp", "keep", "сделал ")], CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION, CAPTURED_COLUMNS_V2)
        options = {"captured": {TRAIN: [[str(manifest), "frames.jsonl.xz"]]}, "sample_weight": 1.0, "held_out": ["тыв", "кз"]}
        rows, report = trainer.hypothesis_curriculum(TRAIN, [], frozenset(), options)
        self.assertEqual([(row.original, row.action, row.before_origin) for row in rows],
                         [("вдеет", "keep", "planned_previous_reading"), ("тщеб", "convert", "planned_previous_reading"),
                          ("дз", "keep", "planned_previous_reading")])
        self.assertEqual(report["frames"], len(rows))
        rows, _report = trainer.short_curriculum(TRAIN, [], frozenset(), options)
        self.assertEqual([(row.original, row.action) for row in rows], [("дз", "keep"), ("пк", "convert")])
        self.assertEqual(trainer.short_curriculum(DEVELOPMENT, [], frozenset(), options), ([], {"frames": 0, "scope": "not used"}))
        # The left context's origin joins a frame's identifier only where it is a hypothesis.
        hypothesis_rows, _report = trainer.hypothesis_curriculum(TRAIN, [], frozenset(), {**options, "held_out": []})
        short_rows, _report = trainer.short_curriculum(TRAIN, [], frozenset(), {**options, "held_out": []})
        marked_keys = {row.identifier.split(":", 1)[1] for row in hypothesis_rows if row.original == "дз"}
        plain_keys = {row.identifier.split(":", 1)[1] for row in short_rows if row.original == "дз"}
        self.assertTrue(marked_keys and plain_keys and not marked_keys & plain_keys)

    def test_the_heads_are_fitted_onto_a_frozen_base_with_epochs_of_their_own(self) -> None:
        self.assertEqual((trainer.HEAD_PREFIXES["hypothesis"], trainer.HEAD_PREFIXES["short"]),
                         (HYPOTHESIS_FEATURE_PREFIX, SHORT_FEATURE_PREFIX))
        self.assertTrue({HYPOTHESIS_FEATURE_PREFIX, SHORT_FEATURE_PREFIX} <= set(trainer.OWN_EPOCH_HEADS))
        heads = frozenset({HYPOTHESIS_FEATURE_PREFIX, SHORT_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("тыв", "nsd", "tot ", "", "space", "none", heads, "planned_previous_reading"))
        self.assertFalse(trainer.head_question("тыв", "nsd", "tot ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("тыв", "nsd", "tot ", "", "space", "kept_next_word", heads, "planned_previous_reading"))
        self.assertTrue(trainer.head_question("кз", "rp", "говорит он ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("кз", "rp", "говорит он ", "", "space", "none", frozenset({SHORT_FEATURE_PREFIX}),
                                               "planned_previous_reading"))
        self.assertFalse(trainer.head_question("кз", "rp", "говорит он ", "", "space", "none", frozenset()))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("тыв", "tot ", "", "space", "planned_previous_reading"))
        self.assertTrue(cuda.heads_question("кз", "говорит он ", "", "space"))
        self.assertFalse(cuda.heads_question("тыв", "tot ", "", "space"))

    def test_a_hypothesis_frame_gives_the_detector_the_reading_alone(self) -> None:
        row = trainer.ActionRow("hypothesis:x", "тыв", 1, FieldContext("Telegram", "captured", "tot ", "", "text"), "space", "",
                                "keep", "hypothesis", " ", before_origin="planned_previous_reading")
        with patch.object(trainer, "automatic_word_decision", wraps=automatic_word_decision) as decision:
            item = trainer.evidence(row, _detector(), None)
            trainer.evidence(replace(row, before_origin="field"), _detector(), None)
        self.assertEqual([call.kwargs["previous_words"] for call in decision.call_args_list], [{0: "tot"}, {0: "tot", 1: "еще"}])
        self.assertEqual(item.before_origin, "planned_previous_reading")


class CaptureToolTests(unittest.TestCase):
    def test_a_recipe_may_add_the_key_pressed_beside_a_letter_and_type_typos_into_words_of_two_letters(self) -> None:
        kinds = (*mixed_typing.TYPO_KINDS, "extra")
        typed = {mixed_typing.typo("ты", f"seed:{index}", kinds, SHORT_FIXTURE_TYPO_MIN_LETTERS) for index in range(SHORT_FIXTURE_TYPO_DRAWS)}
        self.assertTrue(any(len(word) == len("ты") + 1 and word.startswith("ты") and word != "тыы" for word in typed))
        self.assertEqual(mixed_typing.typo("ты", "seed:0"), "ты")
        self.assertEqual(mixed_typing.with_typos("ты где", 0, "seed"), "ты где")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with gzip.open(root / "messages.jsonl.gz", "wt", encoding="utf-8") as handle:
                handle.write(json.dumps({"qid": 1, "kind": "tatoeba_chat", "mix": "russian", "text": "еще ты можешь"}, ensure_ascii=False) + "\n")
            recipe = {"seed": "s", "kinds": ["tatoeba_chat"], "count": 1, "russian_share": 1.0, "typo_rate": 1.0,
                      "typo_kinds": list(kinds), "typo_min_letters": SHORT_FIXTURE_TYPO_MIN_LETTERS}
            [(text, _qid)] = mixed_typing.recipe_messages(root / "messages.jsonl.gz", recipe)
            self.assertNotEqual(text, "еще ты можешь")
            with self.assertRaisesRegex(ValueError, "typo"):
                mixed_typing.recipe_messages(root / "messages.jsonl.gz", {**recipe, "typo_kinds": ["sideways"]})

    def test_a_capture_keeps_the_questions_of_each_class(self) -> None:
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["hypothesis"](
            captured_question("тыв", "nsd", "tot ", before_origin="planned_previous_reading")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["hypothesis"](captured_question("тыв", "nsd", "tot ")))
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["short"](captured_question("кз", "rp", "говорит он ")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["short"](
            captured_question("кз", "rp", "говорит он ", before_origin="planned_previous_reading")))
        self.assertEqual(mixed_typing.CAPTURED_FIELDS_BY_SCHEMA[CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION],
                         tuple(name for name in CAPTURED_COLUMNS_V2 if name not in ("count", "before", "after", "role", "label")))

    def test_a_row_holds_the_fields_of_its_manifest_s_schema(self) -> None:
        item = hypothesis("тыв", "nsd", 1, "tot ")
        with patch.dict(mixed_typing._state, {"fields": mixed_typing.CAPTURED_FIELDS_BY_SCHEMA[CAPTURED_SCHEMA_LEFT_ORIGIN_VERSION]}):
            self.assertIn("planned_previous_reading", mixed_typing._row(item, "keep"))
        with patch.dict(mixed_typing._state, {"fields": mixed_typing.CAPTURED_FIELDS}):
            self.assertNotIn("planned_previous_reading", mixed_typing._row(item, "keep"))

    def test_the_short_token_table_sums_the_word_lists(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            small, large, rare = SHORT_FIXTURE_COUNTS
            for corpus, rows in (("rus_news_1", (("кз", small), ("КЗ", large), ("кзк", large), ("кзкз", large), ("рп", rare))),
                                 ("rus_web_1", (("кз", rare), ("qr", large)))):
                folder = root / f"x-{corpus}" / corpus
                folder.mkdir(parents=True)
                lines = [f"{index}\t{word}\t{count}" for index, (word, count) in enumerate(rows)] + ["bad"]
                (folder / f"{corpus}-words.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
            table = mixed_typing.build_short(root)
        self.assertEqual(table, {"cyrillic": {"кз": small + large + rare, "кзк": large}})


class OpeningShortSelectionTests(unittest.TestCase):
    def test_messages_open_with_a_word_of_at_most_three_letters(self) -> None:
        self.assertTrue(selection.opening_short("еще ты можешь"))
        self.assertTrue(selection.opening_short("If not, you"))
        self.assertFalse(selection.opening_short("можешь ещё"))
        self.assertFalse(selection.opening_short("2 раза"))
        self.assertFalse(selection.opening_short(""))

    def test_tatoeba_messages_are_taken_among_the_listed_sentences_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            listed = root / "authors.tsv.xz"
            listed_question, other_question = CAPTURE_FIXTURE_QUESTIONS
            listed.write_bytes(lzma.compress(f"# header\n{listed_question}\talice\n".encode()))
            messages = root / "tatoeba-chat-train.jsonl.gz"
            with gzip.open(messages, "wt", encoding="utf-8") as handle:
                for qid, text in ((listed_question, "еще ты можешь"), (other_question, "не знаю")):
                    handle.write(json.dumps({"qid": qid, "kind": "tatoeba_chat", "mix": "russian", "text": text}, ensure_ascii=False) + "\n")
            report = selection.select(messages, [], "opening-short", 0.0, root / "out", "tatoeba-chat", listed)
            self.assertEqual(report["messages"], {"train": 1, "dev": 0, "left_out": 0})
            with gzip.open(root / "out" / "tatoeba-chat-opening-short-train.jsonl.gz", "rt", encoding="utf-8") as handle:
                self.assertEqual([json.loads(line)["qid"] for line in handle], [listed_question])
            self.assertIsNone(selection.listed_ids(None))


if __name__ == "__main__":
    unittest.main()

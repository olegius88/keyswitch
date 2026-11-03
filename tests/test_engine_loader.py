"""The engine built while the window comes up: subscriptions, the start and errors wait for it and reach it."""

from __future__ import annotations

import sys
import threading
import unittest
from collections.abc import Callable
from pathlib import Path
from typing import cast
from unittest.mock import MagicMock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from keyswitch.engine import EngineSnapshot, KeySwitchEngine  # noqa: E402
from keyswitch.engine_loader import LOADING_ACTION, DeferredEngine  # noqa: E402
from keyswitch.learning import LearningStore  # noqa: E402


class LoaderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.posted: list[Callable[[], None]] = []
        self.engine = MagicMock()
        self.engine.snapshot = EngineSnapshot(running=True, last_action="ready")
        self.learning = cast(LearningStore, MagicMock())

    def loader(self, build: Callable[[], KeySwitchEngine] | None = None, *, background: bool = False) -> DeferredEngine:
        return DeferredEngine(build or (lambda: cast(KeySwitchEngine, self.engine)), learning=self.learning,
                              enabled=False, post=self.posted.append, background=background)

    def gated(self) -> tuple[DeferredEngine, threading.Event]:
        """A loader whose engine is built only once the gate opens."""

        gate = threading.Event()

        def build() -> KeySwitchEngine:
            gate.wait()
            return cast(KeySwitchEngine, self.engine)

        return self.loader(build, background=True), gate

    def drain(self) -> None:
        while self.posted:
            self.posted.pop(0)()

    def test_before_the_engine_is_built_nothing_reaches_it_and_then_everything_does(self) -> None:
        loader, gate = self.gated()
        self.assertIs(loader.learning, self.learning)
        self.assertFalse(loader.ready)
        self.assertEqual(loader.snapshot, EngineSnapshot(enabled=False, last_action=LOADING_ACTION))
        snapshots: list[EngineSnapshot] = []
        failures: list[Exception] = []
        loader.subscribe(snapshots.append)
        for name in ("subscribe_corrections", "subscribe_learning_prompts", "subscribe_rule_requests"):
            getattr(loader, name)(print)
        loader.start(failures.append)
        gate.set()
        self.assertIs(loader.engine(), self.engine)
        self.assertTrue(loader.ready)
        # Built, not yet attached: the subscriptions and the start wait for the window's thread.
        self.engine.subscribe.assert_not_called()
        self.engine.start.assert_not_called()
        self.drain()
        self.engine.subscribe.assert_called_once_with(snapshots.append)
        self.assertEqual(snapshots, [self.engine.snapshot])
        for name in ("subscribe_corrections", "subscribe_learning_prompts", "subscribe_rule_requests"):
            getattr(self.engine, name).assert_called_once_with(print)
        self.engine.start.assert_called_once_with()
        self.assertEqual(failures, [])
        # Attached: everything goes straight to the engine.
        loader.subscribe(print)
        self.engine.subscribe.assert_called_with(print)
        self.assertIs(loader.snapshot, self.engine.snapshot)
        self.assertIs(loader.learning_prompt, self.engine.learning_prompt)
        self.assertIs(loader.engine(), self.engine)
        for name in ("intent_model_status", "context_policy", "prefix_model"):
            self.assertIs(getattr(loader, name), getattr(self.engine, name))
        self.assertIs(loader.confirm_learning_prompt(), self.engine.confirm_learning_prompt.return_value)
        self.assertIs(loader.dismiss_learning_prompt(), self.engine.dismiss_learning_prompt.return_value)
        self.assertIs(loader.select_alternate_group(), self.engine.select_alternate_group.return_value)
        loader.set_rule_editor_open(True)
        loader.set_test_field_focused(True)
        self.engine.set_rule_editor_open.assert_called_once_with(True)
        self.engine.set_test_field_focused.assert_called_once_with(True)
        rule = MagicMock()
        self.assertIs(loader.add_learning_rule(rule, application="Telegram"), self.engine.add_learning_rule.return_value)
        self.engine.add_learning_rule.assert_called_once_with(rule, replacing=None, application="Telegram")
        self.engine.start.reset_mock()
        loader.start(failures.append)
        self.engine.start.assert_called_once_with()
        self.engine.start.side_effect = RuntimeError("hook")
        loader.start(failures.append)
        self.assertEqual([str(error) for error in failures], ["hook"])
        loader.stop()
        self.engine.stop.assert_called_once_with()

    def test_a_loading_engine_answers_safely_and_a_stop_before_it_is_built_keeps_it_from_starting(self) -> None:
        build: list[KeySwitchEngine] = []
        loader = self.loader(lambda: build[0])
        # The build failed for want of the engine (IndexError): the error stands in the snapshot.
        self.assertIn("index", loader.snapshot.last_error)
        failures: list[Exception] = []
        loader.start(failures.append)
        self.drain()
        self.assertEqual(len(failures), 1)
        with self.assertRaises(IndexError):
            loader.engine()
        self.assertIsNone(loader.learning_prompt)
        self.assertFalse(loader.confirm_learning_prompt())
        self.assertFalse(loader.dismiss_learning_prompt())
        self.assertFalse(loader.select_alternate_group())
        loader.set_rule_editor_open(True)
        loader.set_test_field_focused(False)
        loader.stop()
        stopped, gate = self.gated()
        stopped.start(failures.append)
        stopped.stop()
        gate.set()
        stopped.engine()
        self.drain()
        self.engine.start.assert_not_called()
        self.engine.stop.assert_called_once_with()

    def test_a_start_that_fails_once_the_engine_is_built_reaches_the_window(self) -> None:
        self.engine.start.side_effect = RuntimeError("no access")
        failures: list[Exception] = []
        loader, gate = self.gated()
        loader.start(failures.append)
        gate.set()
        loader.engine()
        self.drain()
        self.assertEqual([str(error) for error in failures], ["no access"])
        unrequested = self.loader(lambda: (_ for _ in ()).throw(ValueError("broken")))
        self.drain()
        self.assertEqual(unrequested.snapshot.last_error, "broken")

    def test_built_at_once_the_engine_takes_everything_directly(self) -> None:
        loader = self.loader()
        self.assertIs(loader.snapshot, self.engine.snapshot)
        loader.subscribe(print)
        self.engine.subscribe.assert_called_once_with(print)
        self.drain()
        self.engine.start.assert_not_called()


if __name__ == "__main__":
    unittest.main()

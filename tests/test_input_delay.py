"""The technical log says when the keyboard hook saw keys late: the measure of a typing stall.

After 0.43.0 the owner reported typing that froze for moments (08.10.2026), and the log, which
timed every correction, had nothing to say about the hook every key passes through first.
"""

from __future__ import annotations

import tempfile
import unittest
from dataclasses import asdict, replace
from pathlib import Path
from unittest.mock import patch

from keyswitch import context_model, context_policy, ortho_model
from keyswitch.backend import KeyEvent
from keyswitch.constants.timing import (
    INPUT_DELAY_LATE_MS,
    INPUT_DELAY_REPORT_INTERVAL_SECONDS,
    INPUT_DELAY_SLOW_CALLBACK_MS,
    INPUT_DELAY_UNSTAMPED_MS,
)
from keyswitch.constants.units import MILLISECONDS_PER_SECOND
from keyswitch.constants.windows import DWORD_MASK, VK_RETURN
from keyswitch.context_model import ContextModel
from keyswitch.context_policy import ContextPolicy
from keyswitch.input_delay import InputDelay, InputDelayReport
from keyswitch.windows_backend import NativeKeyEvent, WindowsBackend
import test_input_sequence_matrix as sequences
from fixture_values.clock import HOOK_KEY_TIME_MS
from fixture_values.keys import SCAN_CODE_ENTER
from test_reopened_word import technical_events
from test_windows_backend import FakeWindowsAPI


class InputDelayTests(unittest.TestCase):
    def test_the_counts_keep_the_late_and_slow_keys_and_start_afresh(self) -> None:
        delay = InputDelay()
        # Three keys: on time and quick, late, slow.
        delay.record(INPUT_DELAY_LATE_MS - 1, INPUT_DELAY_SLOW_CALLBACK_MS - 1)
        delay.record(INPUT_DELAY_LATE_MS, 0)
        delay.record(0, INPUT_DELAY_SLOW_CALLBACK_MS)
        report = delay.take()
        self.assertEqual(report, InputDelayReport(len("abc"), 1, INPUT_DELAY_LATE_MS, 1, INPUT_DELAY_SLOW_CALLBACK_MS))
        self.assertTrue(report.stalled)
        self.assertEqual(delay.take(), InputDelayReport(0, 0, 0, 0, 0))
        self.assertFalse(delay.take().stalled)

    def test_a_key_stamped_with_a_time_of_its_own_is_no_delay(self) -> None:
        # One such key every ten minutes or so read as late by the computer's uptime (0.44.0).
        delay = InputDelay()
        delay.record(INPUT_DELAY_UNSTAMPED_MS, 0)
        delay.record(INPUT_DELAY_UNSTAMPED_MS - 1, 0)
        report = delay.take()
        self.assertEqual(report, InputDelayReport(len("ab"), 1, INPUT_DELAY_UNSTAMPED_MS - 1, 0, 0, unstamped_keys=1))
        delay.record(INPUT_DELAY_UNSTAMPED_MS, 0)
        self.assertFalse(delay.take().stalled)

    def test_another_programs_keys_are_timed_by_the_answer_alone(self) -> None:
        delay = InputDelay()
        delay.record_foreign(INPUT_DELAY_SLOW_CALLBACK_MS)
        delay.record_foreign(0)
        report = delay.take()
        self.assertEqual(report, InputDelayReport(0, 0, 0, 1, INPUT_DELAY_SLOW_CALLBACK_MS, foreign_keys=len("ab")))
        self.assertTrue(report.stalled)

    def test_the_windows_hook_measures_typed_keys_only(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        api.ticks = HOOK_KEY_TIME_MS + INPUT_DELAY_LATE_MS
        key = NativeKeyEvent(True, VK_RETURN, SCAN_CODE_ENTER, False, False, HOOK_KEY_TIME_MS)
        backend._handle_native(key)
        # KeySwitch's own keys, their replays and the pointer are no typing.
        backend._handle_native(replace(key, pressed=False, injected=True))
        backend._handle_native(replace(key, pressed=False, replayed=True))
        backend._handle_native(NativeKeyEvent(True, 0, 0, False, False, 0))
        # A key time ahead of the clock says nothing about a delay.
        backend._handle_native(replace(key, pressed=False, timestamp=api.ticks + 1))
        report = backend.take_input_delay()
        self.assertEqual((report.keys, report.late_keys, report.worst_late_ms), (1, 1, INPUT_DELAY_LATE_MS))
        # The counter wraps after 49.7 days; a key stamped just before is still on time.
        api.ticks = 0
        backend._handle_native(replace(key, pressed=False, timestamp=DWORD_MASK))
        self.assertEqual(backend.take_input_delay(), InputDelayReport(1, 0, 1, 0, 0))
        # A key another program sent with a time of zero, long after the computer started.
        api.ticks = INPUT_DELAY_UNSTAMPED_MS
        backend._handle_native(replace(key, timestamp=0))
        self.assertEqual(backend.take_input_delay(), InputDelayReport(1, 0, 0, 0, 0, unstamped_keys=1))

    def test_the_windows_hook_times_its_answer_to_another_programs_keys(self) -> None:
        # TeamViewer types every character it receives on the computer it controls.
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        api.ticks = HOOK_KEY_TIME_MS + INPUT_DELAY_UNSTAMPED_MS
        seconds = INPUT_DELAY_SLOW_CALLBACK_MS / MILLISECONDS_PER_SECOND
        with patch("keyswitch.windows_backend.time.perf_counter", side_effect=(0.0, seconds)):
            backend._handle_native(
                NativeKeyEvent(True, VK_RETURN, SCAN_CODE_ENTER, False, False, HOOK_KEY_TIME_MS, foreign=True))
        self.assertEqual(backend.take_input_delay(),
                         InputDelayReport(0, 0, 0, 1, INPUT_DELAY_SLOW_CALLBACK_MS, foreign_keys=1))

    def test_a_slow_answer_is_counted_with_its_length(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        api.ticks = HOOK_KEY_TIME_MS
        seconds = INPUT_DELAY_SLOW_CALLBACK_MS / MILLISECONDS_PER_SECOND
        with patch("keyswitch.windows_backend.time.perf_counter", side_effect=(0.0, seconds)):
            backend._handle_native(NativeKeyEvent(True, VK_RETURN, SCAN_CODE_ENTER, False, False, HOOK_KEY_TIME_MS))
        self.assertEqual(backend.take_input_delay(), InputDelayReport(1, 0, 0, 1, INPUT_DELAY_SLOW_CALLBACK_MS))

    def test_the_engine_logs_a_stall_once_a_minute_and_nothing_else(self) -> None:
        stalled = InputDelayReport(1, 1, INPUT_DELAY_LATE_MS, 0, 0)
        with sequences.session(0) as current:
            current.settings.set("diagnostics.technical_logging", True)
            engine = current.engine
            # A backend that cannot measure the hook has nothing to report.
            engine._input_delay_due = 0.0
            engine._report_input_delay()
            reports = [stalled, InputDelayReport(1, 0, 0, 0, 0)]
            current.backend.take_input_delay = lambda: reports.pop(0)  # type: ignore[attr-defined]
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                engine._run_timers()
                # Not due again for a minute.
                engine._report_input_delay()
                self.assertEqual(reports, [InputDelayReport(1, 0, 0, 0, 0)])
                engine._input_delay_due -= INPUT_DELAY_REPORT_INTERVAL_SECONDS
                engine._report_input_delay()
                engine._technical_event("marker")
            self.assertEqual(reports, [])
            events = [event for event in technical_events(logs.output) if event["event"] == "input_delay"]
            self.assertEqual(events, [{"schema": 1, "event": "input_delay", **asdict(stalled),
                                       "late_threshold_ms": INPUT_DELAY_LATE_MS,
                                       "slow_threshold_ms": INPUT_DELAY_SLOW_CALLBACK_MS,
                                       "unstamped_threshold_ms": INPUT_DELAY_UNSTAMPED_MS}])


class FirstWordTests(unittest.TestCase):
    """The first word after a start is answered from tables already read.

    Every slow hook answer in the owner's logs since 0.46.0, 31 to 249 ms, came with the first word the context
    model was asked about after a start (09-10.10.2026): the model read its frequency table and the identifier
    lexicon on that first question, and the hook's thread waited for the interpreter while they were parsed. The
    orthotactic model bound its dictionaries there too, normalising the Russian supplement once more.
    """

    def test_the_policy_reads_its_model_s_tables_as_it_starts(self) -> None:
        with patch.object(context_model, "_TERM_FREQUENCY", None), patch.object(context_policy, "_SHARED_IDENTIFIERS", None):
            policy = ContextPolicy()
            self.assertIsNotNone(policy.model)
            self.assertIsNotNone(context_model._TERM_FREQUENCY)
            self.assertIsNotNone(context_policy._SHARED_IDENTIFIERS)

    def test_a_policy_reads_only_what_its_models_ask_for(self) -> None:
        # Without a context model nothing asks for the identifiers; without an orthotactic model nothing is bound.
        with patch.object(context_policy, "_SHARED_IDENTIFIERS", None), \
                patch.object(ContextModel, "try_load", return_value=(None, "missing")):
            self.assertIsNone(ContextPolicy().model)
            self.assertIsNone(context_policy._SHARED_IDENTIFIERS)
        with patch.object(context_policy, "_SHARED_IDENTIFIERS", None), \
                patch.object(ContextPolicy, "_shared_ortho", (None, "unavailable")):
            policy = ContextPolicy()
            self.assertEqual((policy.model is not None, policy.ortho), (True, None))
            self.assertIsNotNone(context_policy._SHARED_IDENTIFIERS)

    def test_an_action_model_without_its_table_does_not_load(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing.json"
            with patch.object(context_model, "_TERM_FREQUENCY", None), \
                    patch.object(context_model, "TERM_FREQUENCY_PATH", missing):
                model, reason = ContextModel.try_load()
        self.assertIsNone(model)
        self.assertIn(missing.name, reason)

    def test_the_first_word_parses_nothing_and_binds_no_dictionaries(self) -> None:
        # A fresh start: no table read, no orthotactic model bound to its dictionaries.
        with patch.object(context_model, "_TERM_FREQUENCY", None), patch.object(context_policy, "_SHARED_IDENTIFIERS", None), \
                patch.object(ContextPolicy, "_shared_ortho", None), \
                patch.object(ortho_model, "_engine_dictionaries", wraps=ortho_model._engine_dictionaries) as dictionaries, \
                sequences.session(0) as current:
            model = current.engine.context_policy.model
            assert model is not None
            self.assertEqual(dictionaries.call_count, 1)
            with patch("json.loads", side_effect=AssertionError) as parse, \
                    patch.object(model, "predict", wraps=model.predict) as predict:
                current.physical("ghbdtn ")
            self.assertEqual((current.backend.text, parse.call_count, dictionaries.call_count), ("привет ", 0, 1))
            self.assertTrue(predict.called)


if __name__ == "__main__":
    unittest.main()

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
    HOOK_GAP_MS,
    HOOK_SILENCE_MS,
    INPUT_DELAY_LATE_MS,
    INPUT_DELAY_REPORT_INTERVAL_SECONDS,
    INPUT_DELAY_SLOW_CALLBACK_MS,
    INPUT_DELAY_UNSTAMPED_MS,
)
from keyswitch.constants.units import MILLISECONDS_PER_SECOND
from keyswitch.constants.windows import DWORD_MASK, VK_RETURN
from keyswitch.context_model import ContextModel
from keyswitch.context_policy import ContextPolicy
from keyswitch.input_delay import HookGap, InputDelay, InputDelayReport
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


class HookGapTests(unittest.TestCase):
    """Keys that passed the Windows hook: a key the hook reached or answered HOOK_GAP_MS late ends the tracked text.

    Under CPU load the hook's thread waited 1.5 to 3.8 s for the processor, and Windows gave each key that waited
    about 300 ms behind it to the window without the hook (VM, 28.10.2026); all 23 such keys lay within the wait of a
    key the hook reached that late.
    """

    def test_a_late_or_slow_key_leaves_one_gap_until_it_is_taken(self) -> None:
        delay = InputDelay()
        delay.note_wait(HOOK_GAP_MS - 1)
        delay.record(HOOK_GAP_MS - 1, 0)
        self.assertIsNone(delay.take_gap())
        # Late as the hook reaches it: noted then, and not once more when its answer is counted.
        delay.note_wait(HOOK_GAP_MS)
        self.assertEqual(delay.take_gap(), HookGap(HOOK_GAP_MS, 0))
        delay.record(HOOK_GAP_MS, 1)
        self.assertIsNone(delay.take_gap())
        # On time but answered slowly enough to keep the keys behind it as long.
        delay.record(HOOK_GAP_MS - 1, 1)
        self.assertEqual(delay.take_gap(), HookGap(HOOK_GAP_MS - 1, 1))
        self.assertIsNone(delay.take_gap())
        # A key with a time of its own says nothing about a wait.
        delay.note_wait(INPUT_DELAY_UNSTAMPED_MS)
        delay.record(INPUT_DELAY_UNSTAMPED_MS, HOOK_GAP_MS)
        self.assertIsNone(delay.take_gap())

    def test_another_programs_key_leaves_a_gap_by_its_answer(self) -> None:
        delay = InputDelay()
        delay.record_foreign(HOOK_GAP_MS - 1)
        self.assertIsNone(delay.take_gap())
        delay.record_foreign(HOOK_GAP_MS)
        self.assertEqual(delay.take_gap(), HookGap(0, HOOK_GAP_MS))

    def test_the_windows_hook_notes_the_gap_before_the_key_goes_on(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        key = NativeKeyEvent(True, VK_RETURN, SCAN_CODE_ENTER, False, False, HOOK_KEY_TIME_MS)
        api.ticks = HOOK_KEY_TIME_MS + HOOK_GAP_MS
        seen: list[HookGap | None] = []
        backend._listener = lambda event: seen.append(backend._input_delay._gap)
        backend._handle_native(key)
        self.assertEqual(seen, [HookGap(HOOK_GAP_MS, 0)])
        self.assertEqual(backend.take_hook_gap(), HookGap(HOOK_GAP_MS, 0))
        # A key time ahead of the clock, and another program's key, are not waits the hook can know.
        backend._handle_native(replace(key, pressed=False, timestamp=api.ticks + 1))
        backend._handle_native(replace(key, timestamp=0, foreign=True))
        self.assertIsNone(backend.take_hook_gap())

    def test_the_engine_lets_go_of_the_word_after_a_gap(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("diagnostics.technical_logging", True)
            gaps = [HookGap(HOOK_GAP_MS, 0)]
            current.physical("ghb")
            current.backend.take_hook_gap = lambda: gaps.pop() if gaps else None  # type: ignore[attr-defined]
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("dtn ")
                self.assertEqual(current.engine.snapshot.last_action, "Клавиши прошли мимо KeySwitch")
                # The rest of the token is left alone; the next word is the engine's again.
                current.physical("ghbdtn ")
            self.assertEqual(current.backend.text, "ghbdtn привет ")
            gap = next(event for event in technical_events(logs.output) if event["event"] == "hook_gap")
            self.assertEqual((gap["late_ms"], gap["callback_ms"], gap["threshold_ms"]), (HOOK_GAP_MS, 0, HOOK_GAP_MS))

    def test_a_correction_is_not_made_over_a_gap(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("diagnostics.technical_logging", True)
            current.physical("ghbdtn")
            space = current.key(" ")
            current.send(space)
            # The space comes up on time, and the gap arrives as the correction is about to be made.
            gaps: list[HookGap | None] = [HookGap(0, HOOK_GAP_MS), None]
            current.backend.take_hook_gap = lambda: gaps.pop() if gaps else None  # type: ignore[attr-defined]
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.send(replace(space, pressed=False))
            self.assertEqual(gaps, [])
            self.assertEqual(current.backend.text, "ghbdtn ")
            aborted = next(event for event in technical_events(logs.output) if event["event"] == "correction_aborted")
            self.assertEqual(aborted["reason"], "hook_gap")


class UnseenInputTests(unittest.TestCase):
    """Input that reached a program without any hook of KeySwitch is noticed and its program logged once.

    A program in front that reads the keyboard as raw input and has a keyboard hook of its own keeps every keyboard
    hook silent, while GetLastInputInfo moves on with each key (Windows 10 and 11, 28.10.2026).
    """

    def test_an_input_time_no_hook_got_is_certain_after_a_later_event_or_a_long_silence(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        # No hook event yet: an input before the hooks came says nothing.
        api.last_input = HOOK_KEY_TIME_MS
        self.assertIsNone(backend.take_unseen_input())
        # Every input the hooks got.
        api.input_times = [HOOK_KEY_TIME_MS]
        self.assertIsNone(backend.take_unseen_input())
        # A key the hooks have not got yet: perhaps late, so nothing is said until a later event comes.
        api.last_input = HOOK_KEY_TIME_MS + 1
        api.ticks = api.last_input
        self.assertIsNone(backend.take_unseen_input())
        self.assertIsNone(backend.take_unseen_input())
        # It came late after all: nothing passed the hooks.
        api.input_times.append(api.last_input)
        self.assertIsNone(backend.take_unseen_input())
        # A key the hooks never get, and a pointer move they get after it.
        api.last_input = HOOK_KEY_TIME_MS + HOOK_GAP_MS
        self.assertIsNone(backend.take_unseen_input())
        api.input_times.append(api.last_input + 1)
        api.ticks = api.last_input + HOOK_GAP_MS
        self.assertEqual(backend.take_unseen_input(), HOOK_GAP_MS)
        # Keys in a silent program and nothing else: certain once HOOK_SILENCE_MS went by.
        api.last_input = api.ticks
        self.assertIsNone(backend.take_unseen_input())
        api.ticks += HOOK_SILENCE_MS - 1
        self.assertIsNone(backend.take_unseen_input())
        api.ticks += 1
        self.assertEqual(backend.take_unseen_input(), HOOK_SILENCE_MS)
        # The same input is reported once, however long the silence lasts.
        api.ticks += HOOK_SILENCE_MS
        self.assertIsNone(backend.take_unseen_input())
        self.assertIsNone(backend.take_unseen_input())
        # The next key typed into the silent program is new input.
        api.last_input = api.ticks
        self.assertIsNone(backend.take_unseen_input())
        api.ticks += HOOK_SILENCE_MS
        self.assertEqual(backend.take_unseen_input(), HOOK_SILENCE_MS)

    def test_the_tick_clock_wraps_and_a_hook_restart_forgets_nothing_it_needs(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        api.input_times = [DWORD_MASK]
        api.last_input = 0
        self.assertIsNone(backend.take_unseen_input())
        # The event times are gone (a new hook): only the silence decides.
        api.input_times = []
        api.ticks = HOOK_SILENCE_MS
        self.assertEqual(backend.take_unseen_input(), HOOK_SILENCE_MS)
        # An input time older than the newest event got is no input the hooks missed after it.
        api.input_times = [HOOK_KEY_TIME_MS]
        api.last_input = HOOK_KEY_TIME_MS - 1
        self.assertIsNone(backend.take_unseen_input())
        self.assertIsNone(backend._unseen_input)

    def test_the_engine_lets_go_of_the_text_and_names_each_program_once(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("diagnostics.technical_logging", True)
            engine = current.engine
            # A backend that cannot tell says nothing.
            engine._report_unseen_input()
            unseen = [None, HOOK_GAP_MS, HOOK_SILENCE_MS, HOOK_SILENCE_MS]
            current.backend.take_unseen_input = lambda: unseen.pop(0)  # type: ignore[attr-defined]
            current.physical("ghb")
            application = ["game"]
            current.backend.active_application = lambda: application[0]  # type: ignore[method-assign]
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                engine._report_unseen_input()
                self.assertEqual(engine._strokes[-1].character, "b")
                engine._report_unseen_input()
                self.assertEqual(engine._strokes, [])
                self.assertTrue(engine._untracked_token)
                engine._report_unseen_input()
                application[0] = "other"
                engine._report_unseen_input()
            reported = [(event["application"], event["unseen_ms"]) for event in technical_events(logs.output)
                        if event["event"] == "input_unseen"]
            self.assertEqual(reported, [("game", HOOK_GAP_MS), ("other", HOOK_SILENCE_MS)])
            self.assertEqual(unseen, [])


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

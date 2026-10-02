"""Text Input Sources calls reach the main thread, or give way to their fallback in time."""

from __future__ import annotations

import queue
import threading
import unittest

from fixture_values.clock import (
    MAIN_THREAD_RUNNER_LONG_WAIT_SECONDS,
    MAIN_THREAD_RUNNER_RELEASE_AFTER_SECONDS,
    MAIN_THREAD_RUNNER_SHORT_WAIT_SECONDS,
)
from keyswitch.macos_main_thread import MainThreadRunner


class SimulatedMainThread:
    """A thread standing in for the main one: it runs every job number posted to it, in order."""

    def __init__(self) -> None:
        self.posted: queue.Queue[int | None] = queue.Queue()
        self.runner = MainThreadRunner(lambda: threading.get_ident() == self.ident, self.posted.put)
        self.ident: int | None = None
        self.thread = threading.Thread(target=self._loop, daemon=True)
        self.thread.start()

    def _loop(self) -> None:
        self.ident = threading.get_ident()
        while (number := self.posted.get()) is not None:
            self.runner.run(number)

    def close(self) -> None:
        self.posted.put(None)
        self.thread.join(MAIN_THREAD_RUNNER_LONG_WAIT_SECONDS)


class MainThreadRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.main = SimulatedMainThread()
        self.addCleanup(self.main.close)

    def test_a_call_from_another_thread_runs_on_the_main_one(self) -> None:
        ran_on: list[int] = []

        def read() -> str:
            ran_on.append(threading.get_ident())
            return "com.apple.keylayout.ABC"

        answer = self.main.runner.call(read, "", MAIN_THREAD_RUNNER_LONG_WAIT_SECONDS)
        self.assertEqual(answer, "com.apple.keylayout.ABC")
        self.assertEqual(ran_on, [self.main.ident])

    def test_a_call_on_the_main_thread_runs_at_once_without_posting(self) -> None:
        posted: list[int] = []
        runner = MainThreadRunner(lambda: True, posted.append)
        self.assertEqual(runner.call(lambda: "ABC", "", MAIN_THREAD_RUNNER_SHORT_WAIT_SECONDS), "ABC")
        self.assertEqual(posted, [])

    def test_a_job_the_main_thread_does_not_begin_in_time_is_withdrawn(self) -> None:
        posted: list[int] = []
        ran: list[str] = []

        def switch() -> str:
            ran.append("late")
            return "Russian"

        runner = MainThreadRunner(lambda: False, posted.append)
        answer = runner.call(switch, "ABC", MAIN_THREAD_RUNNER_SHORT_WAIT_SECONDS)
        self.assertEqual(answer, "ABC")
        # The main thread reaches it afterwards: a withdrawn layout switch must not happen late.
        runner.run(posted[0])
        self.assertEqual(ran, [])

    def test_an_error_on_the_main_thread_is_raised_to_the_caller(self) -> None:
        def fail() -> str:
            raise OSError("TISCopyCurrentKeyboardInputSource failed")

        with self.assertRaisesRegex(OSError, "TISCopyCurrentKeyboardInputSource"):
            self.main.runner.call(fail, "", MAIN_THREAD_RUNNER_LONG_WAIT_SECONDS)

    def test_a_job_begun_as_the_wait_ends_is_waited_for(self) -> None:
        begun = threading.Event()
        release = threading.Event()

        def slow() -> str:
            begun.set()
            release.wait(MAIN_THREAD_RUNNER_LONG_WAIT_SECONDS)
            return "Russian"

        threading.Timer(MAIN_THREAD_RUNNER_RELEASE_AFTER_SECONDS, release.set).start()
        answer = self.main.runner.call(slow, "ABC", MAIN_THREAD_RUNNER_SHORT_WAIT_SECONDS)
        self.assertTrue(begun.is_set())
        self.assertEqual(answer, "Russian")


if __name__ == "__main__":
    unittest.main()

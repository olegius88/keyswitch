"""Calls macOS answers only on the main thread, made from any thread.

HIToolbox requires the Text Input Sources functions to run on the main dispatch queue and stops
the process with SIGILL when one runs elsewhere (``dispatch_assert_queue`` under
``TISCopyCurrentKeyboardInputSource``). KeySwitch asks for the current layout from the engine's
worker every half second and from the event tap's thread on every key: on a MacBook Pro with
macOS 13.7.8 every start ended that way within seconds (02.10.2026).

:class:`MainThreadRunner` hands such a call to the main thread and waits for it with a time
limit. A call the main thread does not begin in time is withdrawn, so it never runs late behind
the caller's back, and the caller keeps its fallback. The module is pure Python:
:mod:`keyswitch.macos_native` supplies the two system pieces, whether the current thread is the
main one and how a job number reaches the main queue.
"""

from __future__ import annotations

import itertools
import threading
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import TypeVar, cast

_T = TypeVar("_T")


@dataclass
class _Job:
    function: Callable[[], object]
    done: threading.Event = field(default_factory=threading.Event)
    started: bool = False
    result: object = None
    error: Exception | None = None


class MainThreadRunner:
    """Run a function on the main thread and wait for its answer, within a time limit."""

    def __init__(self, is_main_thread: Callable[[], bool], post: Callable[[int], None]) -> None:
        self._is_main_thread = is_main_thread
        self._post = post
        self._jobs: dict[int, _Job] = {}
        self._lock = threading.Lock()
        self._numbers = itertools.count(1)

    def call(self, function: Callable[[], _T], fallback: _T, timeout: float) -> _T:
        """``function()`` on the main thread, or ``fallback`` if the main thread does not begin it in time."""

        if self._is_main_thread():
            return function()
        job = _Job(function)
        with self._lock:
            number = next(self._numbers)
            self._jobs[number] = job
        self._post(number)
        if not job.done.wait(timeout):
            with self._lock:
                if not job.started:
                    del self._jobs[number]
                    return fallback
            # The main thread began it just now; the system call it makes is short.
            job.done.wait()
        if job.error is not None:
            raise job.error
        return cast(_T, job.result)

    def run(self, number: int) -> None:
        """Run the posted job ``number``. Called on the main thread; a withdrawn job is skipped."""

        with self._lock:
            job = self._jobs.pop(number, None)
            if job is None:
                return
            job.started = True
        try:
            job.result = job.function()
        except Exception as error:
            # Raised again on the caller's thread, where it belongs.
            job.error = error
        finally:
            job.done.set()

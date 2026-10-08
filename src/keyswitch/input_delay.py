"""How late the keyboard hook saw the user's keys, as evidence of a stall in the technical log.

A low-level hook stands between every key and every window: when it answers late, all typing in
the system waits for it. After 0.43.0 the owner reported typing that froze for moments while the
log showed every correction on time (08.10.2026); the log had no measure of the hook itself. The
hook thread counts here how long after the key it ran and how long it took; the engine's worker
takes the counts once a minute and logs them when a key was late.

Keys other programs inject (TeamViewer types every character it receives as one) carry no time
the user pressed them at; only how long the hook took over them is counted, so that on a
computer controlled from another one the log still says whether KeySwitch held its input up.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass, replace

from .constants.timing import INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS, INPUT_DELAY_UNSTAMPED_MS


@dataclass(frozen=True)
class InputDelayReport:
    keys: int
    # Keys the hook saw INPUT_DELAY_LATE_MS or more after the system stamped them.
    late_keys: int
    worst_late_ms: int
    # Keys, typed or another program's, whose hook callback took INPUT_DELAY_SLOW_CALLBACK_MS or more.
    slow_callbacks: int
    worst_callback_ms: int
    # Typed keys stamped INPUT_DELAY_UNSTAMPED_MS or more before the hook saw them: a time of
    # their sender's own, no delay.
    unstamped_keys: int = 0
    # Keys another program injected, timed by the hook's answer alone.
    foreign_keys: int = 0

    @property
    def stalled(self) -> bool:
        return bool(self.late_keys or self.slow_callbacks)


class InputDelay:
    """Counts the hook thread adds to and the engine's worker takes."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._report = InputDelayReport(0, 0, 0, 0, 0)

    def record(self, late_ms: int, callback_ms: int) -> None:
        """A typed key: how long after its key time the hook ran, and how long the answer took."""

        stamped = late_ms < INPUT_DELAY_UNSTAMPED_MS
        with self._lock:
            report = self._report
            self._report = replace(
                report,
                keys=report.keys + 1,
                late_keys=report.late_keys + (stamped and late_ms >= INPUT_DELAY_LATE_MS),
                worst_late_ms=max(report.worst_late_ms, late_ms) if stamped else report.worst_late_ms,
                slow_callbacks=report.slow_callbacks + (callback_ms >= INPUT_DELAY_SLOW_CALLBACK_MS),
                worst_callback_ms=max(report.worst_callback_ms, callback_ms),
                unstamped_keys=report.unstamped_keys + (not stamped),
            )

    def record_foreign(self, callback_ms: int) -> None:
        """A key another program injected: how long the answer took."""

        with self._lock:
            report = self._report
            self._report = replace(
                report,
                slow_callbacks=report.slow_callbacks + (callback_ms >= INPUT_DELAY_SLOW_CALLBACK_MS),
                worst_callback_ms=max(report.worst_callback_ms, callback_ms),
                foreign_keys=report.foreign_keys + 1,
            )

    def take(self) -> InputDelayReport:
        """The counts since the last take, and a fresh start."""

        with self._lock:
            report, self._report = self._report, InputDelayReport(0, 0, 0, 0, 0)
        return report

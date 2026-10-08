"""How late the keyboard hook saw the user's keys, as evidence of a stall in the technical log.

A low-level hook stands between every key and every window: when it answers late, all typing in
the system waits for it. After 0.43.0 the owner reported typing that froze for moments while the
log showed every correction on time (08.10.2026); the log had no measure of the hook itself. The
hook thread counts here how long after the key it ran and how long it took; the engine's worker
takes the counts once a minute and logs them when a key was late.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass

from .constants.timing import INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS


@dataclass(frozen=True)
class InputDelayReport:
    keys: int
    # Keys the hook saw INPUT_DELAY_LATE_MS or more after the system stamped them.
    late_keys: int
    worst_late_ms: int
    # Keys whose hook callback took INPUT_DELAY_SLOW_CALLBACK_MS or more.
    slow_callbacks: int
    worst_callback_ms: int

    @property
    def stalled(self) -> bool:
        return bool(self.late_keys or self.slow_callbacks)


class InputDelay:
    """Counts the hook thread adds to and the engine's worker takes."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._report = InputDelayReport(0, 0, 0, 0, 0)

    def record(self, late_ms: int, callback_ms: int) -> None:
        with self._lock:
            report = self._report
            self._report = InputDelayReport(
                report.keys + 1,
                report.late_keys + (late_ms >= INPUT_DELAY_LATE_MS),
                max(report.worst_late_ms, late_ms),
                report.slow_callbacks + (callback_ms >= INPUT_DELAY_SLOW_CALLBACK_MS),
                max(report.worst_callback_ms, callback_ms),
            )

    def take(self) -> InputDelayReport:
        """The counts since the last take, and a fresh start."""

        with self._lock:
            report, self._report = self._report, InputDelayReport(0, 0, 0, 0, 0)
        return report

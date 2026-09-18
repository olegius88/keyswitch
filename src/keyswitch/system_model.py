"""What every platform reports about installed programs and about autostart.

The shapes are the same everywhere - a program the user can exclude, and the
answer to "will KeySwitch start with the session" - so they are defined once and
each platform fills them in its own way.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Application:
    """A program as the exclusion list names it."""

    name: str
    identifier: str
    executable: str


@dataclass(frozen=True)
class AutostartStatus:
    """What the user actually gets at the next logon, and why.

    ``command`` is what the system will run, empty when nothing is registered.
    ``blocked_by_windows`` records a refusal Windows keeps separately from the
    entry itself: Task Manager and Settings mark a startup entry disabled, and
    such an entry is skipped however often it is rewritten. Systems without that
    notion leave it false. ``target_missing`` marks an entry pointing at a
    program that is no longer there.
    """

    command: str | None
    blocked_by_windows: bool
    target_missing: bool

    @property
    def effective(self) -> bool:
        return bool(self.command) and not self.blocked_by_windows and not self.target_missing

    def as_dict(self) -> dict[str, object]:
        return {"command": self.command, "blocked_by_windows": self.blocked_by_windows,
                "target_missing": self.target_missing, "effective": self.effective}

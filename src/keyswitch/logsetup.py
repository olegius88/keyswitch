"""Application log file with a rotation budget that follows the log mode.

Ordinary operation writes a handful of lines per session, while the
diagnostics ("developer") mode writes a JSON line for every evaluated word, so
one budget cannot serve both.  The handler therefore carries the limits of the
mode that is switched on, and turning the diagnostics mode on starts a fresh
file so the log that is later attached to a report contains that session only.
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from logging.handlers import RotatingFileHandler
from pathlib import Path

from .config import SettingsStore
from .history import data_dir


LOG_FILE_NAME = "keyswitch.log"
LOG_FORMAT = "%(asctime)s %(levelname)s %(name)s: %(message)s"
TECHNICAL_LOGGING_PATH = "diagnostics.technical_logging"
# Rotation budgets: (bytes per file, kept files). The diagnostics mode keeps
# more and larger files because a busy hour of typing fills megabytes.
DEFAULT_ROTATION = (1024 * 1024, 2)
TECHNICAL_ROTATION = (5 * 1024 * 1024, 5)


def log_directory() -> Path:
    """Directory holding the log file and its rotated copies."""

    return data_dir()


def log_path() -> Path:
    return log_directory() / LOG_FILE_NAME


def rotation_limits(technical: bool) -> tuple[int, int]:
    return TECHNICAL_ROTATION if technical else DEFAULT_ROTATION


def rotation_summary(technical: bool) -> str:
    """Human wording of the current budget, shown on the diagnostics page."""

    maximum, backups = rotation_limits(technical)
    return f"{maximum // (1024 * 1024)} МБ × {backups + 1} файлов"


def file_handler() -> RotatingFileHandler | None:
    """The rotating handler installed by :func:`configure_logging`, if any."""

    for handler in logging.getLogger().handlers:
        if isinstance(handler, RotatingFileHandler):
            return handler
    return None


def apply_rotation(handler: RotatingFileHandler, *, technical: bool) -> bool:
    """Give the handler the budget of ``technical`` and report a rollover.

    Switching the diagnostics mode on rolls the current file over, so the new
    session starts empty; switching it off only shrinks the budget, because
    discarding the diagnostics that were just recorded would be surprising.
    """

    maximum, backups = rotation_limits(technical)
    handler.acquire()
    try:
        was_technical = handler.maxBytes == TECHNICAL_ROTATION[0]
        handler.maxBytes = maximum
        handler.backupCount = backups
        if was_technical or not technical:
            return False
        path = Path(handler.baseFilename)
        if not path.exists() or path.stat().st_size == 0:
            return False
        handler.doRollover()
        return True
    finally:
        handler.release()


def follow_settings(
    settings: SettingsStore,
    handler: RotatingFileHandler | None = None,
) -> Callable[[], None] | None:
    """Track the diagnostics setting on ``handler`` and apply it right away.

    Returns the unsubscribe callable, or ``None`` when no file handler is
    installed (``--diagnose`` and the tests run without one).
    """

    target = file_handler() if handler is None else handler
    if target is None:
        return None

    def changed(path: str, _value: object) -> None:
        if path not in (TECHNICAL_LOGGING_PATH, "*"):
            return
        apply_rotation(
            target,
            technical=bool(settings.get(TECHNICAL_LOGGING_PATH, False)),
        )

    apply_rotation(
        target, technical=bool(settings.get(TECHNICAL_LOGGING_PATH, False))
    )
    return settings.subscribe(changed)


def configure_logging(settings: SettingsStore | None = None) -> RotatingFileHandler:
    """Install the rotating file log; follow ``settings`` when they are known."""

    directory = log_directory()
    directory.mkdir(parents=True, exist_ok=True)
    technical = (
        settings is not None
        and bool(settings.get(TECHNICAL_LOGGING_PATH, False))
    )
    maximum, backups = rotation_limits(technical)
    handler = RotatingFileHandler(
        log_path(),
        maxBytes=maximum,
        backupCount=backups,
        encoding="utf-8",
    )
    logging.basicConfig(level=logging.INFO, format=LOG_FORMAT, handlers=[handler])
    if settings is not None:
        follow_settings(settings, handler)
    return handler

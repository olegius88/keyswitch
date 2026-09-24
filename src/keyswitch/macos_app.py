"""macOS application entry point.

The module is platform-only so a Linux or Windows launch never imports it, and
it holds no window code: the engine runs headless here, reporting through the
log, while the menu bar item is attached separately.
"""

from __future__ import annotations

import argparse
import json
import logging
import signal
import sys
import threading
import time
from collections.abc import Callable
from types import FrameType

from . import __version__
from .constants.file_formats import DIAGNOSTICS_JSON_INDENT
from .context_model import ContextModel
from .history import HistoryStore, data_dir
from .intent_model import LinearNgramModel
from .logsetup import configure_logging as configure_logging
from .macos_backend import MacBackend
from .config import SettingsStore
from .macos_context import SOURCE as CONTEXT_SOURCE
from .constants.timing import MACOS_PERMISSION_POLL_SECONDS

STOP_SIGNALS = (signal.SIGINT, signal.SIGTERM)
PERMISSION_REQUEST_MESSAGE = (
    "KeySwitch нужен доступ к клавиатуре. Открываю «Конфиденциальность и безопасность» → "
    "«Универсальный доступ»: включите там KeySwitch, и программа продолжит сама."
)
PERMISSION_GRANTED_MESSAGE = "Доступ получен, KeySwitch начинает работу."

LOGGER = logging.getLogger(__name__)


def _running_on_macos() -> bool:
    return sys.platform == "darwin"


def diagnose() -> int:
    """Everything a support report needs, without starting the engine."""

    backend = MacBackend()
    probe = backend.probe()
    permission = backend.permission_granted()
    _intent_model, intent_status = LinearNgramModel.try_load_default()
    context_model, context_status = ContextModel.try_load()
    print(json.dumps({
        "keyswitch": __version__,
        "available": probe.available,
        "session_type": probe.session_type,
        "display": probe.display,
        "hook": probe.record_version,
        "injection": probe.xtest_version,
        "layouts": probe.xkb_version,
        "current_group": probe.current_group,
        "intent_model": intent_status.as_dict(),
        "context_model": {"available": context_model is not None, "status": context_status},
        # The accessibility tree answers both questions, so the reader is
        # available exactly when the permission is.
        "context_field_access": {"available": permission, "source": CONTEXT_SOURCE},
        "accessibility_permission": permission,
        "error": probe.error,
    }, ensure_ascii=False, indent=DIAGNOSTICS_JSON_INDENT))
    backend.close()
    return 0 if probe.available else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="keyswitch",
        description="Автоматическое исправление раскладки EN/RU в macOS",
    )
    parser.add_argument("--hidden", action="store_true")
    parser.add_argument("--diagnose", action="store_true")
    parser.add_argument("--no-engine", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--version", action="version", version=f"KeySwitch {__version__}")
    return parser


def ensure_permission(
    backend: MacBackend,
    finished: threading.Event,
    *,
    wait: Callable[[float], None] = time.sleep,
) -> bool:
    """Do not start without the permission; ask for it and wait for the answer.

    Starting anyway would give a program that looks alive and corrects nothing,
    which is the hardest kind of fault to report. Waiting instead means the user
    can grant the permission and see KeySwitch carry on without relaunching it.
    """

    if backend.permission_granted():
        return True
    LOGGER.warning(PERMISSION_REQUEST_MESSAGE)
    print(PERMISSION_REQUEST_MESSAGE)
    backend.request_permission()
    while not finished.is_set():
        if backend.permission_granted():
            LOGGER.info(PERMISSION_GRANTED_MESSAGE)
            print(PERMISSION_GRANTED_MESSAGE)
            return True
        wait(MACOS_PERMISSION_POLL_SECONDS)
    return False


def run_window(*, hidden: bool, no_engine: bool) -> int:
    """Hand the main thread to the settings window, as Windows does.

    Tk and AppKit cannot each own the main thread, and on macOS Tk is an AppKit
    program underneath, so the menu bar item is created inside the Tk loop
    rather than beside it.
    """

    from .desktop_ui import run_application
    from .macos_services import MacServices

    backend = MacBackend()
    if not ensure_permission(backend, threading.Event()):
        return 0
    return run_application(MacServices(backend), hidden=hidden, no_engine=no_engine)


def main(argv: list[str] | None = None) -> int:
    arguments = build_parser().parse_args(argv)
    configure_logging()
    if arguments.diagnose:
        return diagnose()
    data_dir().mkdir(parents=True, exist_ok=True)
    return run_window(hidden=arguments.hidden, no_engine=arguments.no_engine)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

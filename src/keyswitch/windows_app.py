"""Windows application entry point.

The complete Tk settings frontend is kept in this platform-only module so a
Linux launch never imports Tk or Win32-only dependencies.
"""

from __future__ import annotations

import argparse
import json
import logging
import sys
from logging.handlers import RotatingFileHandler

from . import __version__
from .history import data_dir
from .intent_model import LinearNgramModel
from .windows_backend import WindowsBackend


def configure_logging() -> None:
    directory = data_dir()
    directory.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        handlers=[
            RotatingFileHandler(
                directory / "keyswitch.log",
                maxBytes=5 * 1024 * 1024,
                backupCount=3,
                encoding="utf-8",
            )
        ],
    )


def diagnose() -> int:
    backend = WindowsBackend()
    probe = backend.probe()
    _intent_model, intent_status = LinearNgramModel.try_load_default()
    print(
        json.dumps(
            {
                "keyswitch": __version__,
                "available": probe.available,
                "session_type": probe.session_type,
                "display": probe.display,
                "hook": probe.record_version,
                "injection": probe.xtest_version,
                "layouts": probe.xkb_version,
                "current_group": probe.current_group,
                "intent_model": intent_status.as_dict(),
                "error": probe.error,
            },
            # Keep redirected output valid even when a legacy Windows console
            # exposes a code page that cannot encode Russian diagnostics.
            ensure_ascii=True,
            indent=2,
        )
    )
    backend.close()
    return 0 if probe.available else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="keyswitch",
        description="Автоматическое исправление раскладки EN/RU в Windows",
    )
    parser.add_argument("--hidden", action="store_true")
    parser.add_argument("--diagnose", action="store_true")
    parser.add_argument("--no-engine", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--smoke-ui", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--version", action="version", version=f"KeySwitch {__version__}")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = build_parser().parse_args(argv)
    configure_logging()
    if arguments.diagnose:
        return diagnose()
    from .windows_instance import WindowsSingleInstance

    instance = WindowsSingleInstance()
    if not instance.acquire():
        instance.activate_existing()
        instance.close()
        return 0
    # The UI is attached below after the platform services have been composed.
    from .windows_ui import run_windows_application

    try:
        if arguments.smoke_ui:
            return run_windows_application(
                hidden=False,
                no_engine=True,
                quit_after_ms=300,
            )
        return run_windows_application(
            hidden=arguments.hidden,
            no_engine=arguments.no_engine,
        )
    finally:
        instance.close()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

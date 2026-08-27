"""Select the native desktop frontend without importing the other platform."""

from __future__ import annotations

import sys


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def main(argv: list[str] | None = None) -> int:
    if _running_on_windows():
        from .windows_app import main as platform_main
    else:
        from .app import main as platform_main
    return platform_main(argv)

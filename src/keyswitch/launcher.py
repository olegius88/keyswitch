"""Select the native desktop frontend without importing the other platform."""

from __future__ import annotations

import sys


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def _running_on_macos() -> bool:
    return sys.platform == "darwin"


def main(argv: list[str] | None = None) -> int:
    if _running_on_windows():
        from .windows_app import main as platform_main
    elif _running_on_macos():
        from .macos_app import main as platform_main
    else:
        args = sys.argv[1:] if argv is None else argv
        match args:
            case ["--wayland-input-helper", raw_fd] if raw_fd.isdigit():
                from .wayland_native import run_wayland_input_helper

                return run_wayland_input_helper(int(raw_fd))
            case _:
                from .app import main as platform_main
    return platform_main(argv)

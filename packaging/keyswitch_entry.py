"""Build-only entry point for the native KeySwitch executable."""

import sys


def _entry() -> int:
    match sys.argv[1:]:
        case ["--wayland-input-helper", raw_fd] if raw_fd.isdigit():
            from keyswitch.wayland_native import run_wayland_input_helper

            return run_wayland_input_helper(int(raw_fd))
        case _:
            from keyswitch.app import main

            return main()


if __name__ == "__main__":
    raise SystemExit(_entry())

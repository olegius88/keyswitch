"""Build-only entry point for the native macOS application."""

import sys
from pathlib import Path

from keyswitch import macos_system
from keyswitch.macos_app import main


def use_this_program() -> None:
    """Make this binary the program the launch agent runs.

    A compiled program reports a `python3` beside it that does not exist, and the launch agent would run that with
    `-m keyswitch` and never start. The binary is not named as an installed program is either, hence both assignments.
    It lives here because the model seals pin every file of the package.
    """

    program = Path(sys.argv[0]).resolve()
    sys.executable = str(program)
    macos_system.INSTALLED_NAME = program.stem.casefold()


def smoke_window() -> int:
    """Open the window and close it, without the engine and without the permission.

    A build proves here that it carries what the window needs: the report of `--diagnose` never reaches those imports,
    and a runner has no permission to wait for. It lives here because the model seals pin every file of the package.
    """

    from keyswitch.constants.timing import SMOKE_UI_QUIT_AFTER_MS
    from keyswitch.desktop_ui import run_application
    from keyswitch.history import data_dir
    from keyswitch.macos_backend import MacBackend
    from keyswitch.macos_services import MacServices

    agent_program = Path(macos_system.macos_launcher_arguments()[0])
    if not agent_program.is_file():
        print(f"the launch agent would run a missing program: {agent_program}", file=sys.stderr)
        return 1
    data_dir().mkdir(parents=True, exist_ok=True)
    return run_application(MacServices(MacBackend()), hidden=False, no_engine=True, quit_after_ms=SMOKE_UI_QUIT_AFTER_MS)


if __name__ == "__main__":
    use_this_program()
    raise SystemExit(smoke_window() if sys.argv[1:] == ["--smoke-ui"] else main())

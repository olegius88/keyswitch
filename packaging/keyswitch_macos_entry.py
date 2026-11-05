"""Build-only entry point for the native macOS application."""

import sys

from keyswitch.macos_app import main


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

    data_dir().mkdir(parents=True, exist_ok=True)
    return run_application(MacServices(MacBackend()), hidden=False, no_engine=True, quit_after_ms=SMOKE_UI_QUIT_AFTER_MS)


if __name__ == "__main__":
    raise SystemExit(smoke_window() if sys.argv[1:] == ["--smoke-ui"] else main())

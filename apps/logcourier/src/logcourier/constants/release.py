"""Release pipeline, packaging, APT and verification limits and timeouts."""

from __future__ import annotations

from typing import Final

COMPILER_TIMEOUT_SECONDS: Final = 600
CAPTURE_DELAY_MILLISECONDS: Final = 500
CLI_VERSION_CHECK_TIMEOUT_SECONDS: Final = 20
GUI_SELF_TEST_TIMEOUT_SECONDS: Final = 30
INSTALLER_TIMEOUT_SECONDS: Final = 180
VERIFY_FROZEN_TIMEOUT_SECONDS: Final = 60
UNINSTALL_POLL_ATTEMPTS: Final = 50
UNINSTALL_POLL_INTERVAL_SECONDS: Final = 0.2

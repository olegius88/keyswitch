"""Fake operating-system values used by tests."""

from __future__ import annotations

from typing import Final

# Fake winreg access-right identifiers; the fixture never checks their value, only that
# OpenKey/CreateKeyEx were reached, so any distinct numbers do.
WINREG_KEY_READ: Final = 2
WINREG_KEY_SET_VALUE: Final = 3

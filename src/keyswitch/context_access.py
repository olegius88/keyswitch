"""Optional, explicitly enabled accessibility reads outside keyboard hooks."""

from __future__ import annotations

import sys
import time
from typing import Protocol, runtime_checkable

from .input_context import FieldContext, FieldReader


RETRY_DELAYS = (5.0, 15.0, 60.0)


class _ManagedReader(FieldReader, Protocol):
    def close(self) -> None: ...


@runtime_checkable
class _WindowProcess(Protocol):
    def window_process_id(self, window: int) -> int: ...


class PlatformFieldReader:
    def __init__(self, backend: object = None) -> None:
        self._reader: _ManagedReader | None = None
        self._process = backend.window_process_id if isinstance(backend, _WindowProcess) else None
        self.status = "not_requested"
        self._failure_stage: str | None = None
        self._failure_type: str | None = None
        self._retry_attempts = 0
        self._retry_after: float | None = None

    def diagnostics(self) -> dict[str, object]:
        """Return provider status without exception details or field contents."""
        return {
            "status": self.status,
            "failure_stage": self._failure_stage,
            "failure_type": self._failure_type,
        }

    def close(self) -> None:
        if self._reader is not None:
            self._reader.close()
        self._reader = None
        self.status = "not_requested"
        self._failure_stage = None
        self._failure_type = None
        self._retry_attempts = 0
        self._retry_after = None

    def retry_diagnostics(self) -> dict[str, object]:
        return {
            "attempts": self._retry_attempts,
            "limit": len(RETRY_DELAYS),
            "after_ms": None if self._retry_after is None else max(0, round((self._retry_after - time.monotonic()) * 1000)),
        }

    def read(self, application: str, window: int) -> FieldContext | None:
        if not application or not window:
            return None
        stage = "initialization"
        try:
            if self.status == "unavailable":
                if self._retry_after is None or time.monotonic() < self._retry_after:
                    return None
                self._retry_attempts += 1
                stage = "recovery"
                # COM/provider ownership stays on the engine worker. Reopen a
                # failed bridge, never carry stale field ranges into a retry.
                if self._reader is not None:
                    self._reader.close()
                    self._reader = None
                stage = "initialization"
            if self._reader is None:
                if sys.platform == "win32":
                    from .windows_context import WindowsFieldReader
                    self._reader = WindowsFieldReader()
                else:
                    from .atspi_context import AtspiFieldReader
                    self._reader = AtspiFieldReader(process_for_window=self._process)
            stage = "read"
            result = self._reader.read(application, window)
        except Exception as error:
            # Never format provider exceptions: they may contain user text.
            # Missing dependencies, access denial and failed cleanup require
            # an explicit reset, not repeated calls against the same failure.
            self.status = "unavailable"
            self._failure_stage = stage
            if isinstance(error, ImportError):
                self._failure_type = "import_error"
            elif isinstance(error, OSError):
                self._failure_type = "os_error"
            elif isinstance(error, RuntimeError):
                self._failure_type = "runtime_error"
            else:
                self._failure_type = "provider_error"
            self._retry_after = (
                time.monotonic() + RETRY_DELAYS[self._retry_attempts]
                if stage != "recovery" and not isinstance(error, (ImportError, PermissionError))
                and self._retry_attempts < len(RETRY_DELAYS) else None
            )
            return None
        self.status = "available" if result is not None else "unsupported_field"
        self._failure_stage = None
        self._failure_type = None
        self._retry_attempts = 0
        self._retry_after = None
        return result.bounded() if result is not None else None

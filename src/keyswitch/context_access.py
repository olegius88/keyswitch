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
        self._failure_name: str | None = None
        self._failure_code: int | None = None
        self._last_read_ms: int | None = None
        self._retry_attempts = 0
        self._retry_after: float | None = None

    def diagnostics(self) -> dict[str, object]:
        """Provider status without field contents.

        The exception's class name and, where the provider gives one, its
        HRESULT are named here on purpose: they say which call fails without
        carrying a single character the user typed. Anything else the provider
        puts into an exception stays out of the log.
        """

        result: dict[str, object] = {
            "status": self.status,
            "failure_stage": self._failure_stage,
            "failure_type": self._failure_type,
        }
        if self._failure_name is not None:
            result["failure_name"] = self._failure_name
        if self._failure_code is not None:
            result["failure_code"] = self._failure_code
        if self._last_read_ms is not None:
            result["last_read_ms"] = self._last_read_ms
        return result

    def close(self) -> None:
        if self._reader is not None:
            self._reader.close()
        self._reader = None
        self.status = "not_requested"
        self._failure_stage = None
        self._failure_type = None
        self._failure_name = None
        self._failure_code = None
        self._last_read_ms = None
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
                # A bridge that cannot even be closed is dropped all the same:
                # holding the reference would only send the next attempt into
                # the same broken object.
                reader, self._reader = self._reader, None
                if reader is not None:
                    reader.close()
                stage = "initialization"
            if self._reader is None:
                if sys.platform == "win32":
                    from .windows_context import WindowsFieldReader
                    self._reader = WindowsFieldReader()
                elif sys.platform == "darwin":
                    from .macos_context import MacFieldReader
                    self._reader = MacFieldReader()
                else:
                    from .atspi_context import AtspiFieldReader
                    self._reader = AtspiFieldReader(process_for_window=self._process)
            stage = "read"
            started = time.monotonic()
            try:
                result = self._reader.read(application, window)
            finally:
                # How long the provider took is a fact about the provider, and
                # the only way to tell "too slow for the timeout" from "cannot
                # do it at all" without printing anything the user typed.
                self._last_read_ms = round((time.monotonic() - started) * 1000)
        except Exception as error:
            # Never format provider exceptions: they may contain user text.
            # Missing dependencies, access denial and failed cleanup require
            # an explicit reset, not repeated calls against the same failure.
            self.status = "unavailable"
            self._failure_stage = stage
            self._failure_name = type(error).__name__
            code = getattr(error, "hresult", None)
            self._failure_code = code if isinstance(code, int) else None
            if isinstance(error, ImportError):
                self._failure_type = "import_error"
            elif isinstance(error, OSError):
                self._failure_type = "os_error"
            elif isinstance(error, RuntimeError):
                self._failure_type = "runtime_error"
            else:
                self._failure_type = "provider_error"
            # A missing module or a denied right needs a new process, so nothing
            # is gained by asking again. Everything else is about one window at
            # one moment - a provider that has not woken up, a call that ran out
            # of time - and the next application may answer immediately, so the
            # attempts slow down to one a minute instead of stopping for good.
            self._retry_after = (
                None
                if isinstance(error, (ImportError, PermissionError))
                else time.monotonic() + RETRY_DELAYS[min(self._retry_attempts, len(RETRY_DELAYS) - 1)]
            )
            return None
        self.status = "available" if result is not None else "unsupported_field"
        self._failure_stage = None
        self._failure_type = None
        self._failure_name = None
        self._failure_code = None
        self._retry_attempts = 0
        self._retry_after = None
        return result.bounded() if result is not None else None

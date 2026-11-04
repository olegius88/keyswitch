"""The native Windows executable ends its process itself once the program's main has returned."""

from __future__ import annotations

import importlib.util
import sys
import threading
import unittest
from pathlib import Path
from types import ModuleType
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))


def entry_module() -> ModuleType:
    spec = importlib.util.spec_from_file_location("keyswitch_windows_entry", ROOT / "packaging/keyswitch_windows_entry.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Left(Exception):
    """Stands for the end of the process in `os._exit`."""


class LeaveTests(unittest.TestCase):
    def setUp(self) -> None:
        self.entry = entry_module()
        self.exit = patch.object(self.entry.os, "_exit", side_effect=Left).start()
        self.addCleanup(patch.stopall)

    def test_the_status_of_main_ends_the_process_with_the_log_and_the_streams_flushed(self) -> None:
        output, errors = MagicMock(), MagicMock()
        with patch.object(self.entry.sys, "stdout", output), patch.object(self.entry.sys, "stderr", errors), \
                patch.object(self.entry.logging, "shutdown") as logs, self.assertRaises(Left):
            self.entry.leave(1)
        logs.assert_called_once_with()
        output.flush.assert_called_once_with()
        errors.flush.assert_called_once_with()
        self.exit.assert_called_once_with(1)

    def test_a_program_without_streams_or_with_a_closed_one_still_ends(self) -> None:
        closed = MagicMock()
        closed.flush.side_effect = ValueError("closed")
        with patch.object(self.entry.sys, "stdout", None), patch.object(self.entry.sys, "stderr", closed), \
                patch.object(self.entry.logging, "shutdown"), self.assertRaises(Left):
            self.entry.leave(0)
        self.exit.assert_called_once_with(0)

    def test_it_waits_for_ordinary_threads_and_not_for_a_loader_still_building(self) -> None:
        release, finished = threading.Event(), threading.Event()

        def ordinary() -> None:
            release.wait()
            finished.set()

        worker = threading.Thread(target=ordinary)
        loader_gate = threading.Event()
        loader = threading.Thread(target=loader_gate.wait, daemon=True)
        worker.start()
        loader.start()
        self.addCleanup(loader_gate.set)
        threading.Timer(0.0, release.set).start()
        with patch.object(self.entry.logging, "shutdown"), self.assertRaises(Left):
            self.entry.leave(0)
        self.assertTrue(finished.is_set())
        self.assertTrue(loader.is_alive())


if __name__ == "__main__":
    unittest.main()

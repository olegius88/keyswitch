"""The persistent headless test environment must not request document mounts."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipIf(sys.platform == "win32", "Linux GUI test runners")
class GuiTestEnvironmentTests(unittest.TestCase):
    def test_test_environment_disables_portals_without_disabling_accessibility(self) -> None:
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; exec "$2" -c "$3"', "test-env",
             str(ROOT / "tools/gui-test-env.sh"), sys.executable,
             'import json, os; print(json.dumps({key: os.environ.get(key) for key in '
             '("GDK_DEBUG", "ADW_DISABLE_PORTAL", "GTK_USE_PORTAL", "GIO_USE_VFS", "GDK_BACKEND", "GTK_A11Y")}))'],
            env={**os.environ, "GDK_DEBUG": "settings", "GTK_A11Y": "atspi"},
            capture_output=True, text=True, timeout=10, check=True,
        )
        self.assertEqual(json.loads(result.stdout), {
            "GDK_DEBUG": "settings,no-portals", "ADW_DISABLE_PORTAL": "1", "GTK_USE_PORTAL": "0",
            "GIO_USE_VFS": "local", "GDK_BACKEND": "x11", "GTK_A11Y": "atspi",
        })

    def test_runner_requires_a_command(self) -> None:
        result = subprocess.run([str(ROOT / "tools/run-gui-test.sh"), "--"],
                                capture_output=True, text=True, timeout=10, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage:", result.stderr)

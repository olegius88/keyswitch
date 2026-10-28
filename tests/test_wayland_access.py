"""Raw keyboard access for a Wayland session is given only by an administrator's explicit command.

The package installed a udev rule that let every program of the session read every key and type keys of its own, for
everyone who installed or updated it (a code-scanning review of 28.10.2026). The rule now ships inside the application:
`sudo keyswitch --enable-wayland-input` installs it, `--disable-wayland-input` takes the access away at once.
"""

from __future__ import annotations

import os
import stat
import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path
from unittest.mock import patch

from keyswitch import wayland_access
from keyswitch.constants.wayland import WAYLAND_ACCESS_FILE_MODE
from keyswitch.wayland_access import (
    ACCESS_FILES,
    ACCESS_HINT,
    DISABLE_COMMAND,
    ENABLE_COMMAND,
    LOAD_UINPUT,
    RELOAD_COMMANDS,
    RESOURCES,
    SETTLE,
    access_enabled,
    disable_wayland_input,
    enable_wayland_input,
)
from fixture_values.platform import FAKE_USER_ID


class Recorder:
    """A runner that records the commands and fails those it is told to."""

    def __init__(self, failing: Sequence[str] = ()) -> None:
        self.commands: list[tuple[str, ...]] = []
        self.failing = set(failing)

    def __call__(self, command: Sequence[str]) -> int:
        self.commands.append(tuple(command))
        return 1 if command[0] in self.failing else 0


def root_user() -> int:
    return 0


class WaylandAccessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.lines: list[str] = []

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_the_rule_ships_inside_the_application_and_nothing_enables_it(self) -> None:
        for name, _ in ACCESS_FILES:
            self.assertTrue((RESOURCES / name).is_file(), name)
        rule = (RESOURCES / ACCESS_FILES[0][0]).read_text(encoding="utf-8")
        self.assertIn('TAG+="uaccess"', rule)
        self.assertIn("--enable-wayland-input", rule)
        self.assertFalse(access_enabled(self.root))
        # Every report of the missing access names the command and what it grants.
        self.assertIn(ENABLE_COMMAND, ACCESS_HINT)
        self.assertIn(DISABLE_COMMAND, ACCESS_HINT)

    def test_without_root_nothing_is_touched(self) -> None:
        run = Recorder()
        for command, expected in ((enable_wayland_input, ENABLE_COMMAND), (disable_wayland_input, DISABLE_COMMAND)):
            self.lines.clear()
            self.assertEqual(command(root=self.root, run=run, euid=lambda: FAKE_USER_ID, out=self.lines.append), 1)
            self.assertIn(expected, self.lines[0])
        self.assertEqual(run.commands, [])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_enabling_installs_the_files_loads_uinput_and_applies_the_rule(self) -> None:
        run = Recorder()
        self.assertEqual(enable_wayland_input(root=self.root, run=run, euid=root_user, out=self.lines.append), 0)
        for name, destination in ACCESS_FILES:
            target = self.root / destination
            self.assertEqual(target.read_bytes(), (RESOURCES / name).read_bytes())
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), WAYLAND_ACCESS_FILE_MODE)
        self.assertTrue(access_enabled(self.root))
        self.assertEqual(run.commands, [LOAD_UINPUT, *RELOAD_COMMANDS])
        self.assertEqual(len(self.lines), 1)
        self.assertIn(DISABLE_COMMAND, self.lines[0])
        # A command that fails is named, and the access comes with the next boot.
        self.lines.clear()
        self.assertEqual(enable_wayland_input(root=self.root, run=Recorder(["modprobe"]), euid=root_user,
                                              out=self.lines.append), 0)
        self.assertIn(" ".join(LOAD_UINPUT), self.lines[1])

    def test_disabling_removes_the_files_and_strips_the_device_acls_before_udev_runs_again(self) -> None:
        enable_wayland_input(root=self.root, run=Recorder(), euid=root_user, out=self.lines.append)
        devices = [self.root / "dev" / "uinput", self.root / "dev" / "input" / "event0"]
        for device in devices:
            device.parent.mkdir(parents=True, exist_ok=True)
            device.touch()
        run = Recorder()
        self.lines.clear()
        self.assertEqual(disable_wayland_input(root=self.root, run=run, euid=root_user, out=self.lines.append), 0)
        self.assertFalse(access_enabled(self.root))
        self.assertFalse(any((self.root / destination).exists() for _, destination in ACCESS_FILES))
        # The ACLs go once udev and logind have handled the triggered events, which gave the uinput one back.
        self.assertEqual(run.commands, [*RELOAD_COMMANDS, SETTLE, ("setfacl", "-b", *sorted(str(device) for device in devices))])
        self.assertEqual(len(self.lines), 1)
        # Disabling twice is harmless; without setfacl, or with nothing to strip, the access goes at the next boot.
        for runner in (Recorder(["setfacl"]), Recorder(["udevadm"])):
            self.lines.clear()
            self.assertEqual(disable_wayland_input(root=self.root, run=runner, euid=root_user,
                                                   out=self.lines.append), 0)
            self.assertIn("выключен", self.lines[0])
            self.assertIn("после перезагрузки", self.lines[-1])
        for device in devices:
            device.unlink()
        self.lines.clear()
        run = Recorder()
        disable_wayland_input(root=self.root, run=run, euid=root_user, out=self.lines.append)
        self.assertEqual(run.commands, [*RELOAD_COMMANDS, SETTLE])
        self.assertIn("после перезагрузки", self.lines[-1])

    def test_the_commands_ask_the_system_who_runs_them(self) -> None:
        self.assertEqual(wayland_access._effective_user(), getattr(os, "geteuid")())

    def test_a_missing_program_is_a_failed_command(self) -> None:
        self.assertNotEqual(wayland_access._run(("keyswitch-no-such-program",)), 0)
        with patch("keyswitch.wayland_access.subprocess.run") as run:
            run.return_value.returncode = 0
            self.assertEqual(wayland_access._run(("udevadm", "control", "--reload-rules")), 0)
        run.assert_called_once()


if __name__ == "__main__":
    unittest.main()

"""Autostart and the program list on macOS, checked as files and directories.

None of this needs a Mac: a launch agent is a property list and the installed
programs are bundles on disk, so both are built in a temporary directory here.
"""

from __future__ import annotations

import plistlib
import tempfile
import unittest
from pathlib import Path

from keyswitch.macos_system import (
    ARGUMENTS_KEY,
    AUTOSTART_LABEL,
    HIDDEN_ARGUMENT,
    LABEL_KEY,
    RUN_AT_LOAD_KEY,
    MacApplicationCatalog,
    MacAutostartManager,
    MacSystemError,
    launch_agent_path,
    macos_launcher_arguments,
)
from fixture_values.counts import MACOS_CATALOG_EXPECTED_INSTALLED_COUNT
from fixture_values.platform import NON_MAPPING_PLIST

PROGRAM = "/Applications/KeySwitch.app/Contents/MacOS/keyswitch"


class LauncherCommandTests(unittest.TestCase):
    def test_an_installed_program_is_started_on_its_own(self) -> None:
        arguments = macos_launcher_arguments(Path(PROGRAM))
        self.assertEqual(arguments, [PROGRAM, HIDDEN_ARGUMENT])

    def test_anything_else_is_started_through_the_interpreter(self) -> None:
        arguments = macos_launcher_arguments(Path("/usr/bin/python3"), start_hidden=False)
        self.assertEqual(arguments, ["/usr/bin/python3", "-m", "keyswitch"])

    def test_the_agent_lives_where_launchd_reads_it(self) -> None:
        path = launch_agent_path(Path("/Users/someone"))
        self.assertEqual(path.parent.as_posix(), "/Users/someone/Library/LaunchAgents")
        self.assertTrue(path.name.startswith(AUTOSTART_LABEL))


class AutostartTests(unittest.TestCase):

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.home = Path(directory.name)
        self.path = launch_agent_path(self.home)

    def manager(self, **extra: object) -> MacAutostartManager:
        return MacAutostartManager(
            self.path, arguments=[PROGRAM, HIDDEN_ARGUMENT],
            exists=lambda program: program == PROGRAM, **extra)

    def test_nothing_registered_reads_as_no_autostart(self) -> None:
        status = self.manager().status()
        self.assertIsNone(status.command)
        self.assertFalse(status.effective)

    def test_turning_it_on_writes_an_agent_launchd_will_run(self) -> None:
        self.manager().set_enabled(True)
        document = plistlib.loads(self.path.read_bytes())
        self.assertEqual(document[LABEL_KEY], AUTOSTART_LABEL)
        self.assertEqual(document[ARGUMENTS_KEY], [PROGRAM, HIDDEN_ARGUMENT])
        self.assertTrue(document[RUN_AT_LOAD_KEY])
        self.assertTrue(self.manager().enabled())

    def test_turning_it_off_removes_the_agent(self) -> None:
        manager = self.manager()
        manager.set_enabled(True)
        manager.set_enabled(False)
        self.assertFalse(self.path.exists())
        self.assertFalse(manager.enabled())

    def test_turning_it_off_when_it_was_never_on_is_quiet(self) -> None:
        self.manager().set_enabled(False)
        self.assertFalse(self.path.exists())

    def test_an_agent_that_will_not_run_at_login_is_not_an_autostart(self) -> None:
        """It sits on disk and starts nothing, so it must not read as enabled."""

        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(plistlib.dumps({
            LABEL_KEY: AUTOSTART_LABEL, ARGUMENTS_KEY: [PROGRAM], RUN_AT_LOAD_KEY: False}))
        self.assertIsNone(self.manager().status().command)

    def test_an_agent_pointing_at_a_program_that_is_gone_says_so(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(plistlib.dumps({
            LABEL_KEY: AUTOSTART_LABEL, ARGUMENTS_KEY: ["/removed/keyswitch"],
            RUN_AT_LOAD_KEY: True}))
        status = self.manager().status()
        self.assertTrue(status.target_missing)
        self.assertFalse(status.effective)
        self.assertIsNotNone(status.command)

    def test_a_damaged_agent_is_read_as_none_rather_than_raising(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(b"not a property list")
        self.assertIsNone(self.manager().status().command)

    def test_an_agent_without_a_command_is_read_as_none(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(plistlib.dumps({
            LABEL_KEY: AUTOSTART_LABEL, ARGUMENTS_KEY: [], RUN_AT_LOAD_KEY: True}))
        self.assertIsNone(self.manager().status().command)
        self.path.write_bytes(plistlib.dumps({LABEL_KEY: AUTOSTART_LABEL, RUN_AT_LOAD_KEY: True}))
        self.assertIsNone(self.manager().status().command)

    def test_a_list_that_is_not_a_dictionary_is_read_as_none(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(plistlib.dumps(NON_MAPPING_PLIST))
        self.assertIsNone(self.manager().status().command)

    def test_a_directory_that_cannot_be_written_is_reported(self) -> None:
        manager = MacAutostartManager(Path("/proc/keyswitch/agent.plist"))
        with self.assertRaises(MacSystemError):
            manager.set_enabled(True)

    def test_the_default_agent_sits_in_the_users_own_library(self) -> None:
        manager = MacAutostartManager()
        self.assertEqual(manager.path, launch_agent_path())

    def test_whether_the_program_is_there_is_answered_from_disk_by_default(self) -> None:
        """Without this the agent of a deleted program would look healthy."""

        present = self.root_program()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_bytes(plistlib.dumps({
            LABEL_KEY: AUTOSTART_LABEL, ARGUMENTS_KEY: [str(present)], RUN_AT_LOAD_KEY: True}))
        self.assertFalse(MacAutostartManager(self.path).status().target_missing)
        present.unlink()
        self.assertTrue(MacAutostartManager(self.path).status().target_missing)

    def root_program(self) -> Path:
        program = self.home / "keyswitch"
        program.touch()
        return program


class CatalogTests(unittest.TestCase):

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        for name in ("TextEdit.app", "Safari.app", "notes.txt"):
            (self.root / name).mkdir() if name.endswith(".app") else (self.root / name).touch()
        (self.root / "Utilities").mkdir()
        (self.root / "Utilities" / "Terminal.app").mkdir()

    def test_every_bundle_is_listed_and_nothing_else_is(self) -> None:
        catalog = MacApplicationCatalog([self.root])
        names = [application.name for application in catalog.installed()]
        self.assertEqual(names, ["Safari", "Terminal", "TextEdit"])

    def test_a_program_is_identified_by_the_name_the_engine_will_see(self) -> None:
        catalog = MacApplicationCatalog([self.root])
        found = {application.identifier: application for application in catalog.installed()}
        self.assertIn("textedit", found)
        self.assertTrue(found["textedit"].executable.endswith("TextEdit.app"))

    def test_a_directory_that_is_not_there_is_simply_not_a_source(self) -> None:
        catalog = MacApplicationCatalog([self.root / "missing", self.root])
        self.assertEqual(len(catalog.installed()), MACOS_CATALOG_EXPECTED_INSTALLED_COUNT)

    def test_the_same_program_in_two_places_is_listed_once(self) -> None:
        second = self.root / "second"
        (second / "TextEdit.app").mkdir(parents=True)
        catalog = MacApplicationCatalog([self.root, second])
        self.assertEqual(sum(1 for item in catalog.installed() if item.identifier == "textedit"), 1)

    def test_a_bundle_path_alone_names_the_program(self) -> None:
        application = MacApplicationCatalog.from_executable(" /Applications/Pages.app ")
        assert application is not None
        self.assertEqual((application.name, application.identifier), ("Pages", "pages"))

    def test_a_program_outside_a_bundle_is_still_named(self) -> None:
        application = MacApplicationCatalog.from_executable("/usr/bin/vim")
        assert application is not None
        self.assertEqual((application.name, application.identifier), ("vim", "vim"))

    def test_an_empty_path_names_nothing(self) -> None:
        self.assertIsNone(MacApplicationCatalog.from_executable("   "))

    def test_a_bundle_with_no_name_is_left_out_of_the_list(self) -> None:
        (self.root / ".app").mkdir()
        names = [application.name for application in MacApplicationCatalog([self.root]).installed()]
        self.assertEqual(names, ["Safari", "Terminal", "TextEdit"])

    def test_the_default_sources_are_the_usual_places(self) -> None:
        catalog = MacApplicationCatalog()
        self.assertTrue(any(str(item).endswith("Applications") for item in catalog._directories))


if __name__ == "__main__":
    unittest.main()

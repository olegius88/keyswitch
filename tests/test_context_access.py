"""Accessibility contracts: no clipboard, password reads or stale ranges."""

from __future__ import annotations

import importlib
import unittest
import sys
from dataclasses import replace
from typing import Protocol, cast
from unittest.mock import MagicMock, PropertyMock, patch

from keyswitch.atspi_context import AtspiFieldReader, _native_api
from keyswitch.context_access import PlatformFieldReader
from keyswitch.context_model import ContextModel
from keyswitch.context_policy import ContextPolicy
from keyswitch.input_context import CONTEXT_LIMIT, FieldContext
from keyswitch.windows_context import (
    CONNECTION_TIMEOUT_MS,
    SUFFIX_LIMIT_CHARACTERS,
    TEXT_PATTERN_ID,
    UI_AUTOMATION_LIBRARY,
    WindowsFieldReader,
    probe_uia,
)
from test_context_policy import ContextEngineTests

# Read out of the caller's globals by comtypes' `_check_version`; see the
# Windows-only test below. Any existing file proves the timestamp comparison.
typelib_path = sys.executable

# An arbitrary HRESULT used wherever a fixture provider failure needs a code.
FIXTURE_HRESULT = -2147220991
# An arbitrary window id fed to read() wherever the id itself carries no meaning.
WINDOW_ID = 5


class _VersionChecker(Protocol):
    def _check_version(self, actual: str, cached_mtime: float) -> None: ...


class _CodeGenerator(Protocol):
    @property
    def version(self) -> str: ...


class WindowsContextTests(unittest.TestCase):
    MATCHING_PROCESS_ID = 42
    MISMATCHED_PROCESS_ID = 43
    ELEMENT_RUNTIME_ID = [1, 2, 3]
    OTHER_ELEMENT_RUNTIME_ID = [99]
    MULTIPLE_SELECTION_LENGTH = 2
    PREEXISTING_COINIT_FLAGS = 2

    def setUp(self) -> None:
        self.automation = MagicMock()
        self.element = self.automation.GetFocusedElement.return_value
        self.element.CurrentProcessId = self.MATCHING_PROCESS_ID
        self.element.CurrentIsPassword = False
        self.element.CurrentAutomationId = "message"
        self.element.GetRuntimeId.return_value = self.ELEMENT_RUNTIME_ID
        pattern = self.element.GetCurrentPattern.return_value.QueryInterface.return_value
        pattern.GetSelection.return_value.Length = 1
        self.caret = pattern.GetSelection.return_value.GetElement.return_value
        self.caret.CompareEndpoints.return_value = 0
        self.before, self.after = MagicMock(), MagicMock()
        self.before.GetText.return_value = "ранее вставленный текст ghbdtn "
        self.after.GetText.return_value = " после курсора"
        self.caret.Clone.side_effect = [self.before, self.after]
        self.reader = WindowsFieldReader(self.automation, object())
        self.pid = patch.object(WindowsFieldReader, "_process_for_window", return_value=self.MATCHING_PROCESS_ID)
        self.pid.start()
        self.addCleanup(self.pid.stop)

    def test_caret_range_and_password_before_any_gettext(self) -> None:
        field = self.reader.read("chat", 1)
        assert field is not None
        self.assertTrue(field.before.startswith("ранее"))
        self.assertEqual(field.after, " после курсора")
        self.assertEqual(field.source, "uia")
        self.reader.close()  # Injected test automation owns no COM apartment.
        self.before.MoveEndpointByUnit.assert_called_once_with(0, 0, -CONTEXT_LIMIT)
        self.after.MoveEndpointByUnit.assert_called_once_with(1, 0, SUFFIX_LIMIT_CHARACTERS)
        self.element.CurrentIsPassword = True
        self.element.GetCurrentPattern.reset_mock()
        field = self.reader.read("chat", 1)
        assert field is not None
        self.assertTrue(field.sensitive)
        self.assertEqual(field.before, "")
        self.element.GetCurrentPattern.assert_not_called()

    def test_selection_process_focus_change_and_missing_ranges(self) -> None:
        self.element.CurrentProcessId = self.MISMATCHED_PROCESS_ID
        self.assertIsNone(self.reader.read("chat", 1))
        self.element.CurrentProcessId = self.MATCHING_PROCESS_ID
        pattern = self.element.GetCurrentPattern.return_value.QueryInterface.return_value
        ranges = pattern.GetSelection.return_value
        ranges.Length = 0
        self.assertIsNone(self.reader.read("chat", 1))
        ranges.Length = self.MULTIPLE_SELECTION_LENGTH
        multiple = self.reader.read("chat", 1)
        assert multiple is not None
        self.assertTrue(multiple.selection)
        ranges.Length = 1
        self.caret.CompareEndpoints.return_value = 1
        field = self.reader.read("chat", 1)
        assert field is not None
        self.assertTrue(field.selection)
        self.caret.Clone.assert_not_called()
        self.caret.CompareEndpoints.return_value = 0
        other = MagicMock()
        other.GetRuntimeId.return_value = self.OTHER_ELEMENT_RUNTIME_ID
        other.CurrentIsPassword = False
        self.automation.GetFocusedElement.side_effect = [self.element, other]
        changed = self.reader.read("chat", 1)
        assert changed is not None
        self.assertEqual(changed.before, "")

    def test_password_discovered_during_read_and_diagnostic_readiness(self) -> None:
        current = MagicMock()
        current.CurrentIsPassword = True
        self.automation.GetFocusedElement.side_effect = [self.element, current]
        snapshot = self.reader.read("chat", 1)
        assert snapshot is not None
        self.assertTrue(snapshot.sensitive)
        with patch("keyswitch.windows_context.WindowsFieldReader") as factory:
            probe = probe_uia()
            self.assertEqual(
                {key: probe[key] for key in ("available", "focused_text_pattern")},
                {"available": True, "focused_text_pattern": True},
            )
            self.assertIsInstance(probe["focused_probe_ms"], int)
            factory.return_value.close.assert_called_once()

            # A window that offers no text says so; it is not a broken provider.
            factory.return_value.automation.GetFocusedElement.return_value.GetCurrentPattern.return_value = None
            self.assertIs(probe_uia()["focused_text_pattern"], False)

            # The provider's own failure is named by class and HRESULT only.
            class ProviderError(Exception):
                hresult = FIXTURE_HRESULT

            factory.return_value.automation.GetFocusedElement.side_effect = ProviderError("private")
            probe = probe_uia()
            self.assertEqual(
                {key: probe[key] for key in ("available", "focused_text_pattern", "focused_error", "hresult")},
                {"available": True, "focused_text_pattern": None,
                 "focused_error": "ProviderError", "hresult": FIXTURE_HRESULT},
            )

            factory.side_effect = OSError("provider details are private")
            self.assertEqual(probe_uia(), {"available": False, "error": "OSError"})

    def test_a_window_without_text_is_an_unreadable_field_not_a_broken_provider(self) -> None:
        """Chromium and Qt expose a focused element long before its text.

        GetCurrentPattern answers S_OK with a null pointer there, which comtypes
        hands over as None. Treating that as an exception used to mark the whole
        bridge unavailable and, after three retries, stop accessibility reads for
        the rest of the session.
        """

        self.element.GetCurrentPattern.return_value = None
        self.assertIsNone(self.reader.read("chat", 1))
        self.element.GetCurrentPattern.assert_called_once_with(TEXT_PATTERN_ID)

        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader",
        ) as factory:
            factory.return_value.read.return_value = None
            self.assertIsNone(reader.read("chat", 1))
            self.assertEqual(reader.status, "unsupported_field")
            self.assertIsNone(reader.diagnostics()["failure_type"])
            self.assertEqual(reader.retry_diagnostics()["after_ms"], None)

    def test_timeout_configuration_failure_closes_the_apartment(self) -> None:
        with patch.object(type(self.automation), "TransactionTimeout", new_callable=PropertyMock, create=True) as timeout:
            timeout.side_effect = OSError("cannot configure timeout")
            with patch.object(WindowsFieldReader, "close") as close:
                with self.assertRaises(OSError):
                    WindowsFieldReader(self.automation)
            close.assert_called_once()

    def test_native_factory_and_search_role(self) -> None:
        com, client = MagicMock(), MagicMock()
        client.CreateObject.return_value = self.automation
        imported = "comtypes" in sys.modules
        with patch("keyswitch.windows_context.importlib.import_module", side_effect=[com, client]):
            reader = WindowsFieldReader()
        self.assertEqual(com.CoInitializeEx.call_count, int(imported))
        self.assertEqual(reader.automation.ConnectionTimeout, CONNECTION_TIMEOUT_MS)
        self.element.CurrentAutomationId = "search-box"
        field = reader.read("chat", 1)
        assert field is not None
        self.assertEqual(field.role, "search")
        reader.close()
        com.CoUninitialize.assert_called_once()

    def test_reused_com_initializes_worker_and_cleans_failed_factory(self) -> None:
        com, client = MagicMock(), MagicMock()
        client.CreateObject.side_effect = OSError("provider unavailable")
        with patch.dict("sys.modules", {"comtypes": com}), patch.dict("sys.__dict__", {"coinit_flags": self.PREEXISTING_COINIT_FLAGS}), patch("keyswitch.windows_context.importlib.import_module", side_effect=[com, client]):
            with self.assertRaises(OSError):
                WindowsFieldReader()
        com.CoInitializeEx.assert_called_once_with(0)
        com.CoUninitialize.assert_called_once()

    def test_bundled_type_library_is_opened_as_a_frozen_import(self) -> None:
        """The generated wrapper must import on a machine that is not the build one.

        comtypes rejects its own generated module wherever the modification time
        of UIAutomationCore.dll differs, unless ``sys.frozen`` exists, and Nuitka
        never sets it. Without this guard the installed application reported
        `import_error` on every read from the first run on.
        """

        seen: list[tuple[str, object]] = []

        def record(name: str) -> MagicMock:
            seen.append((name, sys.__dict__.get("frozen", "<absent>")))
            return MagicMock()

        com, client = MagicMock(), MagicMock()
        client.GetModule.side_effect = record
        client.CreateObject.return_value = self.automation
        self.assertNotIn("frozen", sys.__dict__)
        with patch("keyswitch.windows_context.importlib.import_module", side_effect=[com, client]):
            WindowsFieldReader().close()
        self.assertEqual(seen, [(UI_AUTOMATION_LIBRARY, True)])
        self.assertNotIn("frozen", sys.__dict__)

        seen.clear()
        com, client = MagicMock(), MagicMock()
        client.GetModule.side_effect = record
        client.CreateObject.return_value = self.automation
        with patch.dict("sys.__dict__", {"frozen": "console_exe"}):
            with patch("keyswitch.windows_context.importlib.import_module", side_effect=[com, client]):
                WindowsFieldReader().close()
            self.assertEqual(sys.__dict__["frozen"], "console_exe")
        self.assertEqual(seen, [(UI_AUTOMATION_LIBRARY, True)])
        self.assertNotIn("frozen", sys.__dict__)

    @unittest.skipUnless(sys.platform == "win32", "comtypes ships only on Windows")
    def test_comtypes_still_rejects_a_foreign_type_library_timestamp(self) -> None:
        """Pin the reason the guard exists, against the installed comtypes.

        `_check_version` compares the modification time of the type library
        recorded in the generated module with the one on this machine and
        raises ImportError unless `sys.frozen` exists. `typelib_path` is read
        from this caller's globals, so the module-level name below is what it
        stats; any existing file proves the comparison, and 0.0 is a timestamp
        no file has. If a future comtypes drops the check, this test fails and
        the guard in windows_context can go with it.
        """

        comtypes = cast(_VersionChecker, importlib.import_module("comtypes"))
        codegenerator = cast(
            _CodeGenerator, importlib.import_module("comtypes.tools.codegenerator")
        )
        with self.assertRaises(ImportError):
            comtypes._check_version(codegenerator.version, 0.0)
        with patch.dict("sys.__dict__", {"frozen": True}):
            comtypes._check_version(codegenerator.version, 0.0)

    def test_win32_pid_binding(self) -> None:
        self.pid.stop()
        with patch("keyswitch.windows_context.ctypes.CDLL") as dll:
            self.assertEqual(WindowsFieldReader._process_for_window(1), 0)
            dll.return_value.GetWindowThreadProcessId.assert_called_once()


class AtspiContextTests(unittest.TestCase):
    CARET_OFFSET = 8
    TEXT_LENGTH = 10
    STALE_CARET_OFFSET = 9
    FAILED_INIT_STATUS = 2
    CACHE_PROBE_ATTEMPTS = 2
    MATCHING_WINDOW_PROCESS_ID = 42
    MISMATCHED_WINDOW_PROCESS_ID = 43

    def setUp(self) -> None:
        _native_api.cache_clear()
        self.addCleanup(_native_api.cache_clear)
        self.api = MagicMock()
        self.api.init.return_value = 0
        self.desktop = self.api.get_desktop.return_value
        self.desktop.get_child_count.return_value = 1
        self.app = self.desktop.get_child_at_index.return_value
        self.app.get_name.return_value = "chat"
        self.app.get_process_id.return_value = 0
        self.app.get_state_set.return_value.contains.return_value = False
        self.app.get_child_count.return_value = 1
        self.node = self.app.get_child_at_index.return_value
        self.node.get_state_set.return_value.contains.return_value = True
        self.node.get_role.return_value = "text"
        self.text = self.node.get_text_iface.return_value
        self.text.get_n_selections.return_value = 0
        self.text.get_caret_offset.return_value = self.CARET_OFFSET
        self.text.get_character_count.return_value = self.TEXT_LENGTH
        self.api.Text.get_text.side_effect = ["before x", " y"]
        self.reader = AtspiFieldReader(self.api)

    def test_bounded_text_password_selection_and_missing_support(self) -> None:
        field = self.reader.read("chat", WINDOW_ID)
        assert field is not None
        self.assertEqual(field.before, "before x")
        self.assertEqual(field.field_id, "5:0:0")
        self.api.Text.get_text.reset_mock()
        self.node.get_role.return_value = self.api.Role.PASSWORD_TEXT
        field = self.reader.read("chat", WINDOW_ID)
        assert field is not None
        self.assertTrue(field.sensitive)
        self.api.Text.get_text.assert_not_called()
        self.node.get_role.return_value = "text"
        self.text.get_n_selections.return_value = 1
        field = self.reader.read("chat", WINDOW_ID)
        assert field is not None
        self.assertTrue(field.selection)
        self.api.Text.get_text.assert_not_called()
        self.node.get_text_iface.return_value = None
        self.assertIsNone(self.reader.read("chat", WINDOW_ID))

    def test_negative_caret_and_stale_caret(self) -> None:
        self.text.get_caret_offset.return_value = -1
        self.assertIsNone(self.reader.read("chat", WINDOW_ID))
        self.text.get_caret_offset.side_effect = [self.CARET_OFFSET, self.STALE_CARET_OFFSET]
        changed = self.reader.read("chat", WINDOW_ID)
        assert changed is not None
        self.assertEqual(changed.before, "")

    def test_masked_entry_and_selection_changed_during_read(self) -> None:
        self.api.Text.get_text.side_effect = ["••••", "••"]
        masked = self.reader.read("chat", WINDOW_ID)
        assert masked is not None
        self.assertTrue(masked.sensitive)
        self.assertEqual(masked.before, "")
        self.api.Text.get_text.side_effect = ["before x", " y"]
        self.text.get_n_selections.side_effect = [0, 1]
        selected = self.reader.read("chat", WINDOW_ID)
        assert selected is not None
        self.assertTrue(selected.selection)

    def test_process_match_missing_application_and_time_budget(self) -> None:
        self.assertIsNone(self.reader.read("elsewhere", WINDOW_ID))
        with patch("keyswitch.atspi_context.Path.read_text", return_value="process\n"):
            self.assertTrue(self.reader._matches("process", self.app))
        with patch("keyswitch.atspi_context.time.monotonic", side_effect=[0, 1]):
            self.assertIsNone(self.reader.read("chat", WINDOW_ID))
        with patch("keyswitch.atspi_context.time.monotonic", side_effect=[0, 0, 0, 1]):
            self.assertIsNone(self.reader.read("chat", WINDOW_ID))
        self.desktop.get_child_at_index.return_value = None
        self.assertIsNone(self.reader.read("chat", WINDOW_ID))

    def test_factory_and_empty_child(self) -> None:
        gi = MagicMock()
        with patch("keyswitch.atspi_context.importlib.import_module", side_effect=[gi, self.api]):
            reader = AtspiFieldReader()
        gi.require_version.assert_called_once_with("Atspi", "2.0")
        self.api.init.assert_called_once_with()
        self.app.get_child_at_index.return_value = None
        self.assertIsNone(reader.read("chat", WINDOW_ID))
        reader.close()

    def test_already_initialized_api_and_failed_initialization_are_cached(self) -> None:
        for status in (1, self.FAILED_INIT_STATUS):
            with self.subTest(status=status):
                _native_api.cache_clear()
                self.api.reset_mock()
                self.api.init.return_value = status
                with patch("keyswitch.atspi_context.importlib.import_module", side_effect=[MagicMock(), self.api]):
                    for _ in range(self.CACHE_PROBE_ATTEMPTS):
                        if status == 1:
                            self.assertIs(AtspiFieldReader().api, self.api)
                        else:
                            with self.assertRaisesRegex(RuntimeError, "AT-SPI initialization failed"):
                                AtspiFieldReader()
                self.api.init.assert_called_once_with()
                self.api.get_desktop.assert_not_called()
                if status == self.FAILED_INIT_STATUS:
                    self.api.set_timeout.assert_not_called()

    def test_window_process_overrides_ambiguous_application_name(self) -> None:
        self.app.get_process_id.return_value = self.MATCHING_WINDOW_PROCESS_ID
        reader = AtspiFieldReader(self.api, process_for_window=lambda window: self.MATCHING_WINDOW_PROCESS_ID)
        self.assertIsNotNone(reader.read("OtherWMClass", WINDOW_ID))
        self.app.get_process_id.return_value = self.MISMATCHED_WINDOW_PROCESS_ID
        self.assertIsNone(reader.read("chat", WINDOW_ID))


class PlatformReaderTests(unittest.TestCase):
    FACTORY_CALLS_AFTER_REOPEN = 2

    def test_each_platform_is_read_by_its_own_provider(self) -> None:
        """A wrong choice here shows up as a silent absence of context."""

        for platform, target in (
            ("win32", "keyswitch.windows_context.WindowsFieldReader"),
            ("darwin", "keyswitch.macos_context.MacFieldReader"),
            ("linux", "keyswitch.atspi_context.AtspiFieldReader"),
        ):
            with self.subTest(platform=platform):
                reader = PlatformFieldReader()
                with patch("keyswitch.context_access.sys.platform", platform), \
                        patch(target) as factory:
                    reader.read("chat", WINDOW_ID)
                factory.assert_called_once()

    def test_diagnostics_start_empty_and_are_independent_snapshots(self) -> None:
        reader = PlatformFieldReader()
        expected = {"status": "not_requested", "failure_stage": None, "failure_type": None}
        with patch("keyswitch.context_access.sys.platform", "linux"), patch("keyswitch.atspi_context.AtspiFieldReader") as factory:
            self.assertEqual(reader.diagnostics(), expected)
            snapshot = reader.diagnostics()
            snapshot["status"] = "changed"
            snapshot["failure_type"] = "changed"
            self.assertIsNone(reader.read("", 1))
            self.assertIsNone(reader.read("chat", 0))
            reader.close()
            self.assertEqual(reader.diagnostics(), expected)
            factory.assert_not_called()

    def test_failure_diagnostics_categorize_each_stage_without_retry(self) -> None:
        for platform, target in (
            ("win32", "keyswitch.windows_context.WindowsFieldReader"),
            ("darwin", "keyswitch.macos_context.MacFieldReader"),
            ("linux", "keyswitch.atspi_context.AtspiFieldReader"),
        ):
            for stage in ("initialization", "read"):
                for error, category in (
                    (ImportError, "import_error"),
                    (ModuleNotFoundError, "import_error"),
                    (OSError, "os_error"),
                    (PermissionError, "os_error"),
                    (RuntimeError, "runtime_error"),
                    (ValueError, "provider_error"),
                    (Exception, "provider_error"),
                ):
                    with self.subTest(platform=platform, stage=stage, error=error.__name__):
                        reader = PlatformFieldReader()
                        with patch("keyswitch.context_access.sys.platform", platform), patch(target) as factory:
                            failing_call = factory if stage == "initialization" else factory.return_value.read
                            failing_call.side_effect = error("private provider message")
                            self.assertIsNone(reader.read("chat", 1))
                            expected: dict[str, object] = {
                                "status": "unavailable", "failure_stage": stage,
                                "failure_type": category, "failure_name": error.__name__,
                            }
                            if stage == "read":
                                # A read that raised still says how long it took.
                                measured = reader.diagnostics()["last_read_ms"]
                                self.assertIsInstance(measured, int)
                                expected["last_read_ms"] = measured
                            self.assertEqual(reader.diagnostics(), expected)
                            factory.assert_called_once()
                            self.assertEqual(factory.return_value.read.call_count, int(stage == "read"))
                            factory.reset_mock()
                            self.assertIsNone(reader.read("", 1))
                            self.assertIsNone(reader.read("chat", 0))
                            self.assertIsNone(reader.read("chat", 1))
                            self.assertEqual(reader.diagnostics(), expected)
                            factory.assert_not_called()
                            factory.return_value.read.assert_not_called()
                            factory.return_value.close.assert_not_called()

    def test_close_clears_failures_before_fresh_reads(self) -> None:
        for stage in ("initialization", "read"):
            for result in (FieldContext("chat", "1", "text"), None):
                with self.subTest(stage=stage, supported=result is not None):
                    reader = PlatformFieldReader()
                    with patch("keyswitch.context_access.sys.platform", "linux"), patch("keyswitch.atspi_context.AtspiFieldReader") as factory:
                        factory.return_value.read.return_value = result
                        failing_call = factory if stage == "initialization" else factory.return_value.read
                        failing_call.side_effect = RuntimeError("private provider message")
                        self.assertIsNone(reader.read("chat", 1))
                        self.assertEqual(reader.diagnostics()["failure_stage"], stage)
                        reader.close()
                        self.assertEqual(factory.return_value.close.call_count, int(stage == "read"))
                        self.assertEqual(reader.diagnostics(), {
                            "status": "not_requested", "failure_stage": None, "failure_type": None,
                        })
                        failing_call.side_effect = None
                        self.assertEqual(reader.read("chat", 1), result)
                        self.assertEqual(factory.call_count, self.FACTORY_CALLS_AFTER_REOPEN)
                        after = reader.diagnostics()
                        self.assertEqual(
                            {key: after[key] for key in ("status", "failure_stage", "failure_type")},
                            {
                                "status": "available" if result is not None else "unsupported_field",
                                "failure_stage": None, "failure_type": None,
                            },
                        )
                        self.assertIsInstance(after["last_read_ms"], int)
                        reader.close()
                        reader.close()
                        self.assertEqual(reader.diagnostics(), {
                            "status": "not_requested", "failure_stage": None, "failure_type": None,
                        })

    def test_a_provider_hresult_is_reported_as_a_number(self) -> None:
        """COM failures carry an HRESULT; the number says which call refused."""

        class ProviderError(Exception):
            hresult = FIXTURE_HRESULT

        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "linux"), \
                patch("keyswitch.atspi_context.AtspiFieldReader") as factory:
            factory.return_value.read.side_effect = ProviderError("private")
            self.assertIsNone(reader.read("chat", 1))
            diagnostics = reader.diagnostics()
            self.assertEqual(diagnostics["failure_name"], "ProviderError")
            self.assertEqual(diagnostics["failure_code"], FIXTURE_HRESULT)
            reader.close()
            self.assertNotIn("failure_code", reader.diagnostics())

    def test_diagnostics_exclude_field_and_unformatted_exception_details(self) -> None:
        class PrivateProviderError(Exception):
            def __str__(self) -> str:
                raise AssertionError("Provider exceptions must not be formatted")

            def __repr__(self) -> str:
                raise AssertionError("Provider exceptions must not be formatted")

        reader = PlatformFieldReader()
        field = FieldContext("private application", "private field", "private before", "private after", source="private source")
        with patch("keyswitch.context_access.sys.platform", "linux"), patch("keyswitch.atspi_context.AtspiFieldReader") as factory:
            factory.return_value.read.return_value = field
            self.assertEqual(reader.read(field.application, 1), field)
            available = reader.diagnostics()
            self.assertEqual(
                {key: available[key] for key in ("status", "failure_stage", "failure_type")},
                {"status": "available", "failure_stage": None, "failure_type": None},
            )
            self.assertIsInstance(available["last_read_ms"], int)
            for stage in ("read", "initialization"):
                failing_call = factory if stage == "initialization" else factory.return_value.read
                failing_call.side_effect = PrivateProviderError("private exception text", field)
                self.assertIsNone(reader.read(field.application, 1))
                failed = reader.diagnostics()
                self.assertEqual(
                    {key: failed[key] for key in ("status", "failure_stage", "failure_type", "failure_name")},
                    {"status": "unavailable", "failure_stage": stage,
                     "failure_type": "provider_error", "failure_name": "PrivateProviderError"},
                )
                reader.close()

    def test_explicit_lazy_read_and_exception_privacy(self) -> None:
        reader = PlatformFieldReader()
        self.assertIsNone(reader.read("", 1))
        self.assertIsNone(reader.read("chat", 0))
        for platform, target in (("win32", "keyswitch.windows_context.WindowsFieldReader"), ("linux", "keyswitch.atspi_context.AtspiFieldReader")):
            reader = PlatformFieldReader()
            with patch("keyswitch.context_access.sys.platform", platform), patch(target) as factory:
                factory.return_value.read.return_value = FieldContext("chat", "1", "secret", sensitive=True)
                result = reader.read("chat", 1)
                assert result is not None
                self.assertEqual(result.before, "")
                factory.return_value.read.return_value = None
                self.assertIsNone(reader.read("chat", 1))
                self.assertEqual(reader.status, "unsupported_field")
                factory.return_value.read.side_effect = RuntimeError("private text")
                self.assertIsNone(reader.read("chat", 1))
                self.assertEqual(reader.status, "unavailable")
                factory.return_value.read.reset_mock()
                self.assertIsNone(reader.read("chat", 1))
                factory.return_value.read.assert_not_called()
                reader.close()
                self.assertEqual(reader.status, "not_requested")
                reader.close()


class FieldPolicyTests(ContextEngineTests):
    def test_native_suffix_privacy_and_unknown_provider_contracts(self) -> None:
        self.choose("convert")
        reader = MagicMock()
        self.engine.context_policy.reader = reader
        self.engine.context_policy.stream.focus("TestEditor", 1)
        baseline = self.engine.detector.decide("ghbdtn", {1: "привет"}, 0)
        for snapshot, expected, source in (
            (None, True, "observed"),
            (FieldContext("other", "A", "другое поле", source="uia"), True, "observed"),
            (FieldContext("TestEditor", "A", "раньше ghbdtn", source="uia"), True, "uia"),
            (FieldContext("TestEditor", "A", "раньше ghbdtn ", source="uia"), True, "uia"),
            (FieldContext("TestEditor", "A", "уже другой текст", source="uia"), False, "uia"),
            (FieldContext("TestEditor", "A", selection=True, source="uia"), False, "uia"),
            (FieldContext("TestEditor", "A", sensitive=True, source="uia"), False, "uia"),
        ):
            reader.read.return_value = snapshot
            result = self.engine.context_policy.decide(baseline, "привет", 1, self.engine.detector, "space", "assist", read_field=True)
            self.assertEqual(result.decision.should_convert, expected)
            assert result.field is not None
            self.assertEqual(result.field.source, source)

    def test_sensitive_field_discovered_at_boundary_is_redacted(self) -> None:
        self.choose("convert")
        reader = MagicMock()
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        reader.read.return_value = None
        self.type("ghbdtn")
        reader.read.return_value = FieldContext("TestEditor", "A", sensitive=True, source="uia")
        self.type(" ")
        self.assertEqual(self.backend.injections, [])
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertEqual(self.engine.context_policy.stream.text, "")

    def test_native_snapshot_correction_succeeds_when_field_is_unchanged(self) -> None:
        self.choose("convert")
        reader = MagicMock()
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        reader.read.side_effect = lambda _app, _window: FieldContext("TestEditor", "A", self.backend.text, source="uia")
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")

    def test_live_context_is_anchored_and_revalidated(self) -> None:
        self.choose("convert")
        reader = MagicMock()
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        reader.read.side_effect = lambda _app, _window: FieldContext("TestEditor", "field-A", "прошлый текст " + self.backend.text, source="uia")
        self.type("ghbdtn")
        boundary = self.key("space", " ")
        self.send(boundary)
        self.assertIsNotNone(self.engine._pending)
        reader.read.side_effect = None
        reader.read.return_value = FieldContext("TestEditor", "field-B", "ghbdtn ", source="uia")
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.assertEqual(self.backend.injections, [])

    def test_password_is_never_accumulated_logged_or_converted(self) -> None:
        self.choose("convert")
        reader = MagicMock()
        reader.read.return_value = FieldContext("TestEditor", "secret", sensitive=True, role="password", source="uia")
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        with self.assertLogs("keyswitch.engine", level="INFO") as output:
            self.type("secret ghbdtn ")
            self.tap(self.key("Pause"))
        self.assertNotIn("secret", "\n".join(output.output))
        self.assertEqual(self.engine.context_policy.stream.text, "")
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertEqual(self.backend.injections, [])
        self.assertFalse(self.engine.consumes_key(self.key("Return")))
        self.tap(self.key("Pointer"))
        self.assertIsNone(self.engine._sensitive_context_window)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (WindowsContextTests, AtspiContextTests, PlatformReaderTests, FieldPolicyTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite

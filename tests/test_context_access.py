"""Accessibility contracts: no clipboard, password reads or stale ranges."""

from __future__ import annotations

import unittest
import sys
from dataclasses import replace
from unittest.mock import MagicMock, PropertyMock, patch

from keyswitch.atspi_context import AtspiFieldReader, _native_api
from keyswitch.context_access import PlatformFieldReader
from keyswitch.context_model import ContextModel
from keyswitch.context_policy import ContextPolicy
from keyswitch.input_context import FieldContext
from keyswitch.windows_context import WindowsFieldReader, probe_uia
from test_context_policy import ContextEngineTests


class WindowsContextTests(unittest.TestCase):
    def setUp(self) -> None:
        self.automation = MagicMock()
        self.element = self.automation.GetFocusedElement.return_value
        self.element.CurrentProcessId = 42
        self.element.CurrentIsPassword = False
        self.element.CurrentAutomationId = "message"
        self.element.GetRuntimeId.return_value = [1, 2, 3]
        pattern = self.element.GetCurrentPattern.return_value.QueryInterface.return_value
        pattern.GetSelection.return_value.Length = 1
        self.caret = pattern.GetSelection.return_value.GetElement.return_value
        self.caret.CompareEndpoints.return_value = 0
        self.before, self.after = MagicMock(), MagicMock()
        self.before.GetText.return_value = "ранее вставленный текст ghbdtn "
        self.after.GetText.return_value = " после курсора"
        self.caret.Clone.side_effect = [self.before, self.after]
        self.reader = WindowsFieldReader(self.automation, object())
        self.pid = patch.object(WindowsFieldReader, "_process_for_window", return_value=42)
        self.pid.start()
        self.addCleanup(self.pid.stop)

    def test_caret_range_and_password_before_any_gettext(self) -> None:
        field = self.reader.read("chat", 1)
        assert field is not None
        self.assertTrue(field.before.startswith("ранее"))
        self.assertEqual(field.after, " после курсора")
        self.assertEqual(field.source, "uia")
        self.reader.close()  # Injected test automation owns no COM apartment.
        self.before.MoveEndpointByUnit.assert_called_once_with(0, 0, -512)
        self.after.MoveEndpointByUnit.assert_called_once_with(1, 0, 128)
        self.element.CurrentIsPassword = True
        self.element.GetCurrentPattern.reset_mock()
        field = self.reader.read("chat", 1)
        assert field is not None
        self.assertTrue(field.sensitive)
        self.assertEqual(field.before, "")
        self.element.GetCurrentPattern.assert_not_called()

    def test_selection_process_focus_change_and_missing_ranges(self) -> None:
        self.element.CurrentProcessId = 43
        self.assertIsNone(self.reader.read("chat", 1))
        self.element.CurrentProcessId = 42
        pattern = self.element.GetCurrentPattern.return_value.QueryInterface.return_value
        ranges = pattern.GetSelection.return_value
        ranges.Length = 0
        self.assertIsNone(self.reader.read("chat", 1))
        ranges.Length = 2
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
        other.GetRuntimeId.return_value = [99]
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
            self.assertEqual(probe_uia(), {"available": True})
            factory.return_value.close.assert_called_once()
            factory.side_effect = OSError("provider details are private")
            self.assertEqual(probe_uia(), {"available": False, "error": "OSError"})

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
        self.assertEqual(reader.automation.ConnectionTimeout, 50)
        self.element.CurrentAutomationId = "search-box"
        field = reader.read("chat", 1)
        assert field is not None
        self.assertEqual(field.role, "search")
        reader.close()
        com.CoUninitialize.assert_called_once()

    def test_reused_com_initializes_worker_and_cleans_failed_factory(self) -> None:
        com, client = MagicMock(), MagicMock()
        client.CreateObject.side_effect = OSError("provider unavailable")
        with patch.dict("sys.modules", {"comtypes": com}), patch.dict("sys.__dict__", {"coinit_flags": 2}), patch("keyswitch.windows_context.importlib.import_module", side_effect=[com, client]):
            with self.assertRaises(OSError):
                WindowsFieldReader()
        com.CoInitializeEx.assert_called_once_with(0)
        com.CoUninitialize.assert_called_once()

    def test_win32_pid_binding(self) -> None:
        self.pid.stop()
        with patch("keyswitch.windows_context.ctypes.CDLL") as dll:
            self.assertEqual(WindowsFieldReader._process_for_window(1), 0)
            dll.return_value.GetWindowThreadProcessId.assert_called_once()


class AtspiContextTests(unittest.TestCase):
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
        self.text.get_caret_offset.return_value = 8
        self.text.get_character_count.return_value = 10
        self.api.Text.get_text.side_effect = ["before x", " y"]
        self.reader = AtspiFieldReader(self.api)

    def test_bounded_text_password_selection_and_missing_support(self) -> None:
        field = self.reader.read("chat", 5)
        assert field is not None
        self.assertEqual(field.before, "before x")
        self.assertEqual(field.field_id, "5:0:0")
        self.api.Text.get_text.reset_mock()
        self.node.get_role.return_value = self.api.Role.PASSWORD_TEXT
        field = self.reader.read("chat", 5)
        assert field is not None
        self.assertTrue(field.sensitive)
        self.api.Text.get_text.assert_not_called()
        self.node.get_role.return_value = "text"
        self.text.get_n_selections.return_value = 1
        field = self.reader.read("chat", 5)
        assert field is not None
        self.assertTrue(field.selection)
        self.api.Text.get_text.assert_not_called()
        self.node.get_text_iface.return_value = None
        self.assertIsNone(self.reader.read("chat", 5))

    def test_negative_caret_and_stale_caret(self) -> None:
        self.text.get_caret_offset.return_value = -1
        self.assertIsNone(self.reader.read("chat", 5))
        self.text.get_caret_offset.side_effect = [8, 9]
        changed = self.reader.read("chat", 5)
        assert changed is not None
        self.assertEqual(changed.before, "")

    def test_masked_entry_and_selection_changed_during_read(self) -> None:
        self.api.Text.get_text.side_effect = ["••••", "••"]
        masked = self.reader.read("chat", 5)
        assert masked is not None
        self.assertTrue(masked.sensitive)
        self.assertEqual(masked.before, "")
        self.api.Text.get_text.side_effect = ["before x", " y"]
        self.text.get_n_selections.side_effect = [0, 1]
        selected = self.reader.read("chat", 5)
        assert selected is not None
        self.assertTrue(selected.selection)

    def test_process_match_missing_application_and_time_budget(self) -> None:
        self.assertIsNone(self.reader.read("elsewhere", 5))
        with patch("keyswitch.atspi_context.Path.read_text", return_value="process\n"):
            self.assertTrue(self.reader._matches("process", self.app))
        with patch("keyswitch.atspi_context.time.monotonic", side_effect=[0, 1]):
            self.assertIsNone(self.reader.read("chat", 5))
        with patch("keyswitch.atspi_context.time.monotonic", side_effect=[0, 0, 0, 1]):
            self.assertIsNone(self.reader.read("chat", 5))
        self.desktop.get_child_at_index.return_value = None
        self.assertIsNone(self.reader.read("chat", 5))

    def test_factory_and_empty_child(self) -> None:
        gi = MagicMock()
        with patch("keyswitch.atspi_context.importlib.import_module", side_effect=[gi, self.api]):
            reader = AtspiFieldReader()
        gi.require_version.assert_called_once_with("Atspi", "2.0")
        self.api.init.assert_called_once_with()
        self.app.get_child_at_index.return_value = None
        self.assertIsNone(reader.read("chat", 5))
        reader.close()

    def test_already_initialized_api_and_failed_initialization_are_cached(self) -> None:
        for status in (1, 2):
            with self.subTest(status=status):
                _native_api.cache_clear()
                self.api.reset_mock()
                self.api.init.return_value = status
                with patch("keyswitch.atspi_context.importlib.import_module", side_effect=[MagicMock(), self.api]):
                    for _ in range(2):
                        if status == 1:
                            self.assertIs(AtspiFieldReader().api, self.api)
                        else:
                            with self.assertRaisesRegex(RuntimeError, "AT-SPI initialization failed"):
                                AtspiFieldReader()
                self.api.init.assert_called_once_with()
                self.api.get_desktop.assert_not_called()
                if status == 2:
                    self.api.set_timeout.assert_not_called()

    def test_window_process_overrides_ambiguous_application_name(self) -> None:
        self.app.get_process_id.return_value = 42
        reader = AtspiFieldReader(self.api, process_for_window=lambda window: 42)
        self.assertIsNotNone(reader.read("OtherWMClass", 5))
        self.app.get_process_id.return_value = 43
        self.assertIsNone(reader.read("chat", 5))


class PlatformReaderTests(unittest.TestCase):
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

"""GTK interaction tests for the complete settings window."""

from __future__ import annotations

import os
import platform
import tempfile
import unittest
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace
from typing import ClassVar
from unittest.mock import Mock, call, patch

import gi

gi.require_version("Adw", "1")
gi.require_version("Gtk", "4.0")
from gi.repository import Adw, Gio, GLib, Gtk

from keyswitch import ui
from keyswitch import logsetup
from keyswitch.config import SettingsStore
from keyswitch.engine import EngineSnapshot
from keyswitch.history import HistoryEntry, HistoryStore
from keyswitch.intent_model import IntentModelStatus
from keyswitch.learning import LearningStore
from keyswitch.ui import ApplicationChoice, MainWindow
from keyswitch.updates import UpdatePhase, UpdateSnapshot
from keyswitch.x11_backend import BackendProbe


DISPLAY_AVAILABLE = bool(os.environ.get("DISPLAY")) and Gtk.init_check()


class FakeBackend:
    def __init__(self, available: bool = True) -> None:
        self.application = "ExternalEditor"
        self.probe_value = BackendProbe(
            available,
            "x11",
            ":99",
            "1.13" if available else "—",
            "2.2" if available else "—",
            "1.0" if available else "—",
            0 if available else -1,
            "" if available else "XRecord unavailable",
        )

    def active_application(self) -> str:
        return self.application

    def probe(self) -> BackendProbe:
        return self.probe_value


@dataclass
class FakeLanguageModel:
    frequencies: dict[str, int]
    source: str


class FakeEngine:
    def __init__(self, root: Path, *, backend_available: bool = True) -> None:
        self.backend = FakeBackend(backend_available)
        self.learning = LearningStore(root / "learning.json")
        self.models: dict[int, FakeLanguageModel] = {
            0: FakeLanguageModel({"hello": 10}, "test-en"),
            1: FakeLanguageModel({"привет": 10}, "test-ru"),
        }
        self.intent_model_status = IntentModelStatus(
            True,
            root / "layout_intent_v1.ksm",
            "test-v1",
            "0123456789abcdef",
            None,
        )
        self.snapshot = EngineSnapshot()
        self.callbacks: list[Callable[[EngineSnapshot], None]] = []

    def subscribe(self, callback: Callable[[EngineSnapshot], None]) -> None:
        self.callbacks.append(callback)
        callback(self.snapshot)


class FakeAutostart:
    def __init__(self) -> None:
        self.state = False
        self.calls: list[tuple[bool, bool]] = []
        self.error: OSError | None = None

    def enabled(self) -> bool:
        return self.state

    def set_enabled(self, enabled: bool, *, start_hidden: bool = True) -> None:
        if self.error:
            raise self.error
        self.state = enabled
        self.calls.append((enabled, start_hidden))


class FakeUpdates:
    def __init__(self) -> None:
        self._snapshot = UpdateSnapshot(
            UpdatePhase.IDLE,
            "Обновления ещё не проверялись",
            "0.4.0",
        )
        self.callbacks: list[Callable[[UpdateSnapshot], None]] = []
        self.checks: list[tuple[bool, bool]] = []
        self.check_result = True

    @property
    def snapshot(self) -> UpdateSnapshot:
        return self._snapshot

    def subscribe(self, callback: Callable[[UpdateSnapshot], None]) -> None:
        self.callbacks.append(callback)
        callback(self._snapshot)

    def check(self, *, automatic: bool, install_automatically: bool) -> bool:
        self.checks.append((automatic, install_automatically))
        return self.check_result

    def install_available(self) -> bool:
        return False

    def installation_failed(self, _error: Exception) -> None:
        return

    def emit(self, snapshot: UpdateSnapshot) -> None:
        self._snapshot = snapshot
        for callback in tuple(self.callbacks):
            callback(snapshot)


def descendants(widget: Gtk.Widget) -> Iterator[Gtk.Widget]:
    """Yield a GTK widget subtree using GTK 4 sibling traversal."""

    child = widget.get_first_child() if hasattr(widget, "get_first_child") else None
    while child is not None:
        yield child
        yield from descendants(child)
        child = child.get_next_sibling()


class InstalledApplicationsTests(unittest.TestCase):
    def test_catalog_filters_deduplicates_sorts_and_survives_bad_desktop_files(self) -> None:
        icon = Gio.ThemedIcon.new("utilities-terminal")

        class App:
            def __init__(
                self,
                name: str,
                identifier: str,
                *,
                visible: bool = True,
                startup: str = "",
                broken: bool = False,
            ) -> None:
                self.name = name
                self.identifier = identifier
                self.visible = visible
                self.startup = startup
                self.broken = broken

            def should_show(self) -> bool:
                if self.broken:
                    raise OSError("bad desktop file")
                return self.visible

            def get_id(self) -> str:
                return f"{self.identifier}.desktop"

            def get_string(self, _key: str) -> str:
                return self.startup

            def get_executable(self) -> str:
                return f"/usr/bin/{self.identifier}"

            def get_display_name(self) -> str:
                return self.name

            def get_name(self) -> str:
                return self.name

            def get_icon(self) -> Gio.Icon:
                return icon

        class AppWithoutString:
            def should_show(self) -> bool: return True
            def get_id(self) -> str: return "fallback.desktop"
            def get_executable(self) -> str: return ""
            def get_display_name(self) -> str: return "Fallback"
            def get_name(self) -> str: return "Fallback"
            def get_icon(self) -> None: return None

        class BlankApp(AppWithoutString):
            def get_id(self) -> str: return ""
            def get_display_name(self) -> str: return ""
            def get_name(self) -> str: return ""

        applications = [
            App("Zulu", "zulu", startup="ZuluClass"),
            App("Alpha", "alpha"),
            App("Duplicate", "duplicate", startup="alphA"),
            App("Hidden", "hidden", visible=False),
            App("Broken", "broken", broken=True),
            AppWithoutString(),
            BlankApp(),
        ]
        fake_gio = SimpleNamespace(AppInfo=SimpleNamespace(get_all=lambda: applications))
        with patch.object(ui, "Gio", fake_gio):
            choices = ui.installed_application_choices()
        self.assertEqual([item.name for item in choices], ["Alpha", "Fallback", "Zulu"])
        self.assertEqual(choices[-1].identifier, "ZuluClass")
        self.assertIn("zuluclass", choices[-1].search_text)


@unittest.skipUnless(DISPLAY_AVAILABLE, "GTK display is required")
class MainWindowInteractionTests(unittest.TestCase):
    application: ClassVar[Adw.Application]

    @classmethod
    def setUpClass(cls) -> None:
        cls.application = Adw.Application(application_id="io.github.olegius88.KeySwitchUiTests")
        cls.application.register(None)

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.settings = SettingsStore(self.root / "config.json")
        self.history = HistoryStore(self.root / "history.jsonl")
        self.history.append(HistoryEntry.create("ghbdtn", "привет", "", 4.256))
        self.engine = FakeEngine(self.root)
        self.autostart = FakeAutostart()
        self.updates = FakeUpdates()
        self.choices = [
            ApplicationChoice("Code", "code", "code", Gio.ThemedIcon.new("text-editor")),
            ApplicationChoice("Terminal", "terminal", "terminal", None),
        ]
        with (
            patch("keyswitch.ui.AutostartManager", return_value=self.autostart),
            patch("keyswitch.ui.installed_application_choices", return_value=self.choices),
        ):
            self.window = MainWindow(
                self.application,
                self.settings,
                self.history,
                self.engine,
                self.updates,
                lambda: True,
            )

    def tearDown(self) -> None:
        for source in tuple(self.window._text_save_sources.values()):
            if source:
                try:
                    GLib.source_remove(source)
                except GLib.Error:
                    pass
        if self.window._app_picker_dialog is not None:
            self.window._app_picker_dialog.close()
        self.window.destroy()
        self.temporary.cleanup()

    def test_full_window_builds_all_pages_and_bound_controls(self) -> None:
        self.assertEqual(self.window.get_title(), "KeySwitch")
        self.assertEqual(len(self.window.NAVIGATION), 9)
        for name, _label, _icon in self.window.NAVIGATION:
            self.assertIsNotNone(self.window.stack.get_child_by_name(name))
        menu = self.window._header_menu()
        self.assertEqual(menu.get_n_items(), 3)
        self.assertEqual(self.window.history_total_label.get_label(), "1 запись")
        self.assertEqual(len(self.window._application_rows), 3)
        self.assertEqual(self.window._learning_summary(2, 1), "Подтверждённых правил: 2 · запретов после отмены: 1")

        minimum = self.window._settings_controls["detection.minimum_length"]
        assert isinstance(minimum, Adw.SpinRow)
        minimum.set_value(5)
        self.assertEqual(self.settings.get("detection.minimum_length"), 5)
        confidence = self.window._settings_controls["detection.confidence"]
        assert isinstance(confidence, Adw.SpinRow)
        confidence.set_value(3.5)
        self.assertEqual(self.settings.get("detection.confidence"), 3.5)
        switch = self.window._settings_controls["detection.aggressive"]
        assert isinstance(switch, Adw.SwitchRow)
        switch.set_active(True)
        self.assertTrue(self.settings.get("detection.aggressive"))
        intent_model = self.window._settings_controls[
            "detection.intent_model_enabled"
        ]
        assert isinstance(intent_model, Adw.SwitchRow)
        intent_model.set_active(False)
        self.assertFalse(self.settings.get("detection.intent_model_enabled"))
        manual_layout = self.window._settings_controls[
            "detection.respect_manual_layout"
        ]
        assert isinstance(manual_layout, Adw.SwitchRow)
        self.assertTrue(manual_layout.get_active())
        manual_layout.set_active(False)
        self.assertFalse(self.settings.get("detection.respect_manual_layout"))
        technical_logging = self.window._settings_controls[
            "diagnostics.technical_logging"
        ]
        assert isinstance(technical_logging, Adw.SwitchRow)
        self.assertFalse(technical_logging.get_active())
        technical_logging.set_active(True)
        self.assertTrue(self.settings.get("diagnostics.technical_logging"))
        pause_correction = self.window._settings_controls[
            "detection.correct_on_pause"
        ]
        assert isinstance(pause_correction, Adw.SwitchRow)
        self.assertTrue(pause_correction.get_active())
        pause_correction.set_active(False)
        self.assertFalse(self.settings.get("detection.correct_on_pause"))
        indicator = self.window._settings_controls["appearance.indicator_style"]
        assert isinstance(indicator, Adw.ComboRow)
        indicator.set_selected(1)
        self.assertEqual(self.settings.get("appearance.indicator_style"), "flags")
        confirmations = self.window._settings_controls[
            "detection.learning_confirmations"
        ]
        assert isinstance(confirmations, Adw.SpinRow)
        confirmations.set_value(3)
        self.assertEqual(self.settings.get("detection.learning_confirmations"), 3)
        theme = next(
            widget
            for widget in descendants(self.window)
            if isinstance(widget, Adw.ComboRow) and widget.get_title() == "Тема"
        )
        theme.set_selected(1)
        self.assertEqual(self.settings.get("appearance.theme"), "light")
        with patch("keyswitch.ui.Gdk.Display.get_default", return_value=None):
            self.window._install_css()

    def test_buttons_entries_and_focus_controller_dispatch_callbacks(self) -> None:
        widgets = list(descendants(self.window))

        def button(
            *,
            label: str | None = None,
            tooltip: str | None = None,
            icon_name: str | None = None,
        ) -> Gtk.Button:
            return next(
                widget
                for widget in widgets
                if isinstance(widget, Gtk.Button)
                and (label is None or widget.get_label() == label)
                and (tooltip is None or widget.get_tooltip_text() == tooltip)
                and (icon_name is None or widget.get_icon_name() == icon_name)
            )

        hotkey = self.window._settings_controls["hotkeys.toggle"]
        assert isinstance(hotkey, Gtk.Entry)
        controllers = hotkey.observe_controllers()
        focus = next(
            controllers.get_item(index)
            for index in range(controllers.get_n_items())
            if isinstance(controllers.get_item(index), Gtk.EventControllerFocus)
        )
        assert isinstance(focus, Gtk.EventControllerFocus)
        with patch.object(self.window, "_save_hotkey") as save_hotkey:
            hotkey.emit("activate")
            focus.emit("leave")
        self.assertEqual(save_hotkey.call_count, 2)

        with (
            patch.object(self.window, "_confirm_clear_learning") as clear_learning,
            patch.object(self.window, "_show_application_picker") as show_picker,
            patch.object(
                self.window, "_start_active_application_capture"
            ) as capture_application,
            patch.object(self.window, "_add_manual_application") as add_manual,
            patch.object(self.window, "_confirm_reset") as reset,
            patch.object(self.window, "_confirm_clear_history") as clear_history,
            patch.object(self.window, "_copy_diagnostics") as copy_diagnostics,
            patch.object(self.window, "_check_updates") as check_updates,
            patch.object(self.window, "_open_update_release") as open_update_release,
            patch.object(
                self.window, "_remove_application_exclusion"
            ) as remove_application,
        ):
            button(label="Очистить").emit("clicked")
            button(label="Из списка…").emit("clicked")
            button(label="Выбрать окно").emit("clicked")
            self.window.manual_app_entry.emit("activate")
            button(label="Добавить").emit("clicked")
            button(label="Сбросить").emit("clicked")
            button(icon_name="user-trash-symbolic").emit("clicked")
            button(icon_name="edit-copy-symbolic").emit("clicked")
            button(label="Проверить сейчас").emit("clicked")
            button(label="Открыть выпуск").emit("clicked")
            next(
                widget
                for widget in widgets
                if isinstance(widget, Gtk.Button)
                and (widget.get_tooltip_text() or "").startswith("Удалить ")
            ).emit("clicked")

        clear_learning.assert_called_once_with()
        show_picker.assert_called_once_with()
        capture_application.assert_called_once_with()
        self.assertEqual(add_manual.call_count, 2)
        reset.assert_called_once_with()
        clear_history.assert_called_once_with()
        copy_diagnostics.assert_called_once_with(self.engine.backend.probe())
        check_updates.assert_called_once_with()
        open_update_release.assert_called_once_with()
        remove_application.assert_called_once_with("keepassxc")

    def test_hotkeys_text_debounce_navigation_and_setting_updates(self) -> None:
        entry = self.window._settings_controls["hotkeys.toggle"]
        assert isinstance(entry, Gtk.Entry)
        with patch.object(self.window, "toast") as toast:
            entry.set_text("   ")
            self.window._save_hotkey("hotkeys.toggle", entry)
            self.assertEqual(entry.get_text(), "Ctrl+Alt+P")
            toast.assert_called_with("Комбинация не может быть пустой")
        entry.set_text("Ctrl+Shift+K")
        self.window._save_hotkey("hotkeys.toggle", entry)
        self.assertEqual(self.settings.get("hotkeys.toggle"), "Ctrl+Shift+K")

        buffer = self.window.words_view.get_buffer()
        self.window._text_save_sources["exclusions.words"] = 999
        with patch("keyswitch.ui.GLib.source_remove") as remove, patch("keyswitch.ui.GLib.timeout_add", return_value=123) as timeout:
            buffer.set_text("one\n\n two \n")
            callback = timeout.call_args.args[1]
            self.assertFalse(callback())
        remove.assert_called_with(999)
        self.assertEqual(self.settings.get("exclusions.words"), ["one", "two"])
        self.assertNotIn("exclusions.words", self.window._text_save_sources)
        with patch("keyswitch.ui.GLib.timeout_add", return_value=124) as fresh_timeout:
            self.window._debounce_text_save("fresh.path", buffer)
            fresh_timeout.call_args.args[1]()

        self.window._navigation_selected(self.window.nav_list, None)
        history_row = self.window.nav_list.get_row_at_index(7)
        automation_row = self.window.nav_list.get_row_at_index(1)
        with patch.object(self.window, "refresh_history") as refresh:
            self.window._navigation_selected(self.window.nav_list, history_row)
        refresh.assert_called_once_with()
        with patch.object(self.window, "_refresh_learning_status") as refresh_learning:
            self.window._navigation_selected(self.window.nav_list, automation_row)
        refresh_learning.assert_called_once_with()
        with patch.object(self.window, "refresh_history") as refresh:
            self.assertTrue(self.window.show_page("history"))
        refresh.assert_called_once_with()
        self.assertFalse(self.window.show_page("missing"))

        with patch("keyswitch.ui.GLib.idle_add") as idle:
            self.window._setting_update_from_thread("enabled", False)
        idle.assert_called_once_with(self.window._apply_setting_update, "enabled", False)
        self.assertFalse(self.window._apply_setting_update("enabled", False))
        self.window._apply_setting_update("detection.minimum_length", 7)
        self.window._apply_setting_update("detection.minimum_length", 7)
        self.window._apply_setting_update("detection.minimum_length", "bad")
        self.window._apply_setting_update("appearance.indicator_style", "flags")
        self.window._apply_setting_update("appearance.indicator_style", "flags")
        with patch.object(self.window, "_refresh_application_exclusions") as refresh_apps:
            self.window._apply_setting_update("exclusions.applications", [])
        refresh_apps.assert_called_once_with()

    def test_engine_snapshots_cover_running_paused_error_and_layouts(self) -> None:
        with patch("keyswitch.ui.GLib.idle_add") as idle:
            snapshot = EngineSnapshot(running=True, enabled=True, current_group=0)
            self.window._engine_update_from_thread(snapshot)
        idle.assert_called_once_with(self.window._apply_engine_snapshot, snapshot)
        self.assertFalse(self.window._apply_engine_snapshot(snapshot))
        self.assertEqual(self.window.hero_pill.get_label(), "АКТИВНО")
        paused = EngineSnapshot(running=True, enabled=False, current_group=1, current_word="word", correction_count=5)
        self.window._apply_engine_snapshot(paused)
        self.assertEqual(self.window.hero_pill.get_label(), "ПАУЗА")
        self.assertEqual(self.window.stat_layout.get_label(), "RU")
        failed = EngineSnapshot(running=False, enabled=True, current_group=-1, last_error="backend failed")
        self.window._apply_engine_snapshot(failed)
        self.assertEqual(self.window.hero_pill.get_label(), "ОШИБКА")
        self.assertEqual(self.window.hero_subtitle.get_label(), "backend failed")

    def test_update_page_states_checks_and_release_opening(self) -> None:
        available = UpdateSnapshot(
            UpdatePhase.AVAILABLE,
            "Доступна версия 1.0.0",
            "0.4.0",
            "1.0.0",
            "https://github.com/olegius88/keyswitch/releases/tag/v1.0.0",
        )
        with patch("keyswitch.ui.GLib.idle_add") as idle:
            self.window._update_from_thread(available)
        idle.assert_called_once_with(self.window._apply_update_snapshot, available)

        self.assertFalse(self.window._apply_update_snapshot(available))
        self.assertEqual(
            self.window.update_version_row.get_subtitle(),
            "Доступна 1.0.0",
        )
        self.assertTrue(self.window.update_open_button.get_sensitive())
        for phase in (UpdatePhase.CHECKING, UpdatePhase.DOWNLOADING):
            snapshot = UpdateSnapshot(phase, "Занято", "0.4.0")
            self.window._apply_update_snapshot(snapshot)
            self.assertFalse(self.window.update_check_button.get_sensitive())
        self.window._apply_update_snapshot(
            UpdateSnapshot(UpdatePhase.CURRENT, "Актуально", "0.4.0")
        )
        self.assertTrue(self.window.update_check_button.get_sensitive())
        self.assertEqual(
            self.window.update_version_row.get_subtitle(),
            "Новых выпусков пока не найдено",
        )

        self.window._check_updates()
        self.assertEqual(self.updates.checks[-1], (False, False))
        self.updates.check_result = False
        with patch.object(self.window, "toast") as toast:
            self.window._check_updates()
        toast.assert_called_once_with("Проверка обновлений уже выполняется")

        self.updates.emit(UpdateSnapshot(UpdatePhase.IDLE, "Ожидание", "0.4.0"))
        with patch.object(self.window, "toast") as toast:
            self.window._open_update_release()
        toast.assert_called_once_with("Сначала проверьте наличие новой версии")

        self.updates.emit(available)
        with patch(
            "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
            return_value=True,
        ) as launch:
            self.window._open_update_release()
        launch.assert_called_once_with(available.release_url, None)
        with (
            patch(
                "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
                return_value=False,
            ),
            patch.object(self.window, "toast") as toast,
        ):
            self.window._open_update_release()
        toast.assert_called_once_with("Не удалось открыть страницу выпуска")
        with (
            patch(
                "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
                side_effect=GLib.Error("open failed"),
            ),
            patch.object(self.window, "toast") as toast,
        ):
            self.window._open_update_release()
        self.assertIn("open failed", toast.call_args.args[0])

    def test_log_folder_button_opens_the_directory_and_reports_failures(self) -> None:
        expected = logsetup.log_directory().as_uri()
        with patch(
            "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
            return_value=True,
        ) as launch:
            self.window._open_log_directory()
        launch.assert_called_once_with(expected, None)

        with (
            patch(
                "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
                return_value=False,
            ),
            patch.object(self.window, "toast") as toast,
        ):
            self.window._open_log_directory()
        toast.assert_called_once_with("Не удалось открыть папку журнала")

        with (
            patch(
                "keyswitch.ui.Gio.AppInfo.launch_default_for_uri",
                side_effect=GLib.Error("no file manager"),
            ),
            patch.object(self.window, "toast") as toast,
        ):
            self.window._open_log_directory()
        self.assertIn("no file manager", toast.call_args.args[0])

    def test_history_rendering_time_plural_and_clear_list(self) -> None:
        self.history.clear()
        self.window.refresh_history()
        self.assertEqual(self.window.history_total_label.get_label(), "0 записей")
        self.history.append(HistoryEntry("invalid", "a", "b", "Editor", 1.0))
        self.history.append(HistoryEntry.create("c", "d", "Editor", 2.0))
        self.window.refresh_history()
        self.assertEqual(self.window.history_total_label.get_label(), "2 записи")
        dashboard = self.window.dashboard_history
        del self.window.dashboard_history
        self.window.refresh_history()
        self.window.dashboard_history = dashboard
        self.assertEqual(self.window._format_time("broken"), "broken")
        self.assertIn(".", self.window._format_time("2026-08-26T10:00:00+00:00"))
        self.assertEqual(
            [self.window._plural_entries(value) for value in (1, 2, 5, 11, 12, 24)],
            ["1 запись", "2 записи", "5 записей", "11 записей", "12 записей", "24 записи"],
        )
        listbox = Gtk.ListBox()
        listbox.append(Gtk.Label(label="one"))
        listbox.append(Gtk.Label(label="two"))
        self.window._clear_list(listbox)
        self.assertIsNone(listbox.get_first_child())

    def test_application_catalog_exclusion_manual_and_active_capture(self) -> None:
        self.assertIs(self.window._application_catalog(), self.window._application_catalog())
        self.window._application_choices = None
        with patch("keyswitch.ui.installed_application_choices", return_value=self.choices) as installed:
            self.assertEqual(self.window._application_catalog(), self.choices)
            self.window._application_catalog()
        installed.assert_called_once_with()

        with patch.object(self.window, "toast") as toast:
            self.window._add_application_exclusion("  ")
            toast.assert_called_with("Не удалось определить имя приложения")
            self.window._add_application_exclusion("code", "Visual Studio Code")
            self.window._add_application_exclusion("CODE", "Visual Studio Code")
            toast.assert_called_with(
                "Visual Studio Code уже находится в исключениях"
            )
        self.window._add_application_exclusion("firefox", "Firefox")
        self.assertIn(
            "firefox", self.settings.get("exclusions.applications", list[str]())
        )
        self.window._remove_application_exclusion("FIREFOX")
        self.assertNotIn(
            "firefox", self.settings.get("exclusions.applications", list[str]())
        )

        self.window.manual_app_entry.set_text("telegram-desktop")
        self.window._add_manual_application()
        self.assertEqual(self.window.manual_app_entry.get_text(), "")
        self.window.manual_app_entry.set_text(" ")
        self.window._add_manual_application()

        with patch.object(self.window, "set_visible") as visible, patch("keyswitch.ui.GLib.timeout_add") as timeout:
            self.window._start_active_application_capture()
        visible.assert_called_once_with(False)
        timeout.assert_called_once_with(2500, self.window._finish_active_application_capture)
        with patch.object(self.window, "present"):
            for name in ("", "keyswitch"):
                self.engine.backend.application = name
                self.assertFalse(self.window._finish_active_application_capture())
            self.engine.backend.application = "ExternalEditor"
            self.assertFalse(self.window._finish_active_application_capture())
        self.assertIn(
            "ExternalEditor",
            self.settings.get("exclusions.applications", list[str]()),
        )

        self.settings.set("exclusions.applications", [])
        self.window._refresh_application_exclusions()
        self.assertEqual(len(self.window._application_rows), 1)
        self.settings.set("exclusions.applications", ["code", "unknown"])
        self.window._refresh_application_exclusions()
        self.assertEqual(len(self.window._application_rows), 2)

    def test_picker_filters_activates_choice_and_releases_dialog(self) -> None:
        self.settings.set("exclusions.applications", [])
        self.window._application_choices = self.choices
        self.window._show_application_picker()
        dialog = self.window._app_picker_dialog
        self.assertIsNotNone(dialog)
        assert dialog is not None
        widgets = list(descendants(dialog))
        search = next(widget for widget in widgets if isinstance(widget, Gtk.SearchEntry))
        applications = next(widget for widget in widgets if isinstance(widget, Gtk.ListBox))
        applications.emit("row-activated", Gtk.ListBoxRow())
        search.set_text("terminal")
        search.emit("search-changed")
        applications.invalidate_filter()
        row = applications.get_row_at_index(1)
        applications.emit("row-activated", row)
        self.assertIn(
            "terminal", self.settings.get("exclusions.applications", list[str]())
        )
        GLib.MainContext.default().iteration(False)
        if self.window._app_picker_dialog is not None:
            # Libadwaita 1.5 emits ``closed`` after its transition, while 1.9
            # commonly completes it in the first main-context iteration.
            dialog.emit("closed")
        self.assertIsNone(self.window._app_picker_dialog)

        self.window._show_application_picker()
        dialog = self.window._app_picker_dialog
        self.assertIsNotNone(dialog)
        assert dialog is not None
        close = next(
            widget
            for widget in descendants(dialog)
            if isinstance(widget, Gtk.Button)
            and widget.get_tooltip_text() == "Закрыть"
        )
        close.emit("clicked")
        GLib.MainContext.default().iteration(False)
        if self.window._app_picker_dialog is not None:
            dialog.emit("closed")
        self.assertIsNone(self.window._app_picker_dialog)

    def test_autostart_theme_clear_reset_diagnostics_and_close(self) -> None:
        row = Adw.SwitchRow()
        with patch.object(self.window, "toast") as toast:
            row.set_active(True)
            self.window._autostart_toggled(row, None)
            self.assertEqual(self.autostart.calls[-1], (True, True))
            self.autostart.error = OSError("readonly")
            self.window._autostart_toggled(row, None)
            self.assertFalse(row.get_active())
            toast.assert_called_with("Не удалось изменить автозапуск: readonly")
        self.autostart.error = None

        manager = Mock()
        with patch("keyswitch.ui.Adw.StyleManager.get_default", return_value=manager):
            for theme in ("system", "light", "dark", "invalid"):
                self.window._set_theme(theme)
        self.assertEqual(manager.set_color_scheme.call_count, 4)

        self.window._clear_history_response("cancel")
        self.window._clear_history_response("clear")
        self.assertEqual(self.history.read(), [])
        self.window._clear_learning_response("cancel")
        self.engine.learning.record_manual(0, "word", 1)
        self.window._clear_learning_response("clear")
        self.assertEqual(self.engine.learning.counts(), (0, 0))
        self.window._reset_response("cancel")
        self.settings.set("enabled", False)
        self.window._reset_response("reset")
        self.assertTrue(self.settings.get("enabled"))

        dialog = Mock()
        with patch("keyswitch.ui.Adw.AlertDialog", return_value=dialog):
            self.window._confirm_clear_history()
            self.window._confirm_clear_learning()
            self.window._confirm_reset()
        self.assertEqual(dialog.present.call_count, 3)
        callbacks = [
            item.args[1]
            for item in dialog.connect.call_args_list
            if item.args[0] == "response"
        ]
        self.assertEqual(len(callbacks), 3)
        for callback in callbacks:
            callback(dialog, "cancel")

        clipboard = Mock()
        display = Mock()
        display.get_clipboard.return_value = clipboard
        with patch("keyswitch.ui.Gdk.Display.get_default", return_value=display):
            self.window._copy_diagnostics(self.engine.backend.probe())
        diagnostics = clipboard.set.call_args.args[0]
        self.assertIn("KeySwitch", diagnostics)
        self.assertIn("Technical logging:", diagnostics)
        self.assertIn("keyswitch.log", diagnostics)
        with (
            patch("keyswitch.ui.Gdk.Display.get_default", return_value=None),
            patch.object(self.window, "toast") as toast,
        ):
            self.window._copy_diagnostics(self.engine.backend.probe())
        toast.assert_called_once_with("Буфер обмена недоступен")
        with patch.object(Path, "read_text", return_value='PRETTY_NAME="Test OS"\nBROKEN'):
            self.assertEqual(self.window._os_description(), "Test OS")
        with patch.object(Path, "read_text", side_effect=OSError), patch("keyswitch.ui.platform.platform", return_value="Fallback OS"):
            self.assertEqual(self.window._os_description(), "Fallback OS")

        self.window.toast("Visible toast")
        close = Mock(return_value=True)
        self.window._close_handler = close
        self.assertTrue(self.window._on_close_request(self.window))
        close.assert_called_once_with()

    def test_unavailable_backend_renders_diagnostics_warning(self) -> None:
        failed_engine = FakeEngine(self.root, backend_available=False)
        with (
            patch("keyswitch.ui.AutostartManager", return_value=self.autostart),
            patch("keyswitch.ui.installed_application_choices", return_value=[]),
        ):
            second = MainWindow(
                self.application,
                self.settings,
                self.history,
                failed_engine,
                self.updates,
                lambda: True,
            )
        self.assertFalse(failed_engine.backend.probe().available)
        second.destroy()


if __name__ == "__main__":
    unittest.main()

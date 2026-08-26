"""Application entry point."""

from __future__ import annotations

import argparse
import json
import logging
import signal
import sys
from pathlib import Path
from typing import Protocol

import gi

gi.require_version("Adw", "1")
gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")

from gi.repository import Adw, Gdk, Gio, GLib  # noqa: E402

from . import __version__
from .config import SettingsStore
from .engine import CorrectionPlan, EngineSnapshot, KeySwitchEngine
from .history import HistoryStore, data_dir
from .system import APP_ID, AutostartManager
from .tray import StatusNotifierItem
from .ui import MainWindow, RESOURCE_DIR
from .x11_backend import X11Backend


LOGGER = logging.getLogger(__name__)


class _WindowController(Protocol):
    def present(self) -> None: ...

    def toast(self, message: str) -> None: ...

    def set_visible(self, visible: bool) -> None: ...

    def get_visible(self) -> bool: ...

    def show_page(self, page_name: str) -> bool: ...


class _TrayController(Protocol):
    def set_indicator_style(self, style: object) -> None: ...

    def set_layout(self, group: int) -> None: ...

    def set_enabled(self, enabled: bool) -> None: ...

    def set_sound_enabled(self, enabled: bool) -> None: ...

    def set_notifications_enabled(self, enabled: bool) -> None: ...

    def close(self) -> None: ...


def configure_logging() -> None:
    directory = data_dir()
    directory.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        handlers=[logging.FileHandler(directory / "keyswitch.log", encoding="utf-8")],
    )


class KeySwitchApplication(Adw.Application):
    def __init__(self, *, hidden: bool = False, no_engine: bool = False) -> None:
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.DEFAULT_FLAGS)
        self.hidden = hidden
        self.no_engine = no_engine
        self.settings = SettingsStore()
        self.autostart = AutostartManager()
        self.history = HistoryStore(limit=int(self.settings.get("history.limit", 200)))
        self.engine = KeySwitchEngine(self.settings, self.history)
        self.window: _WindowController | None = None
        self.tray: _TrayController | None = None
        self._initialized = False
        self._held = False

    def do_startup(self) -> None:
        Adw.Application.do_startup(self)
        for name, callback in (
            ("show", lambda *_args: self.show_window()),
            ("toggle", lambda *_args: self.toggle_engine()),
            ("quit", lambda *_args: self.quit_application()),
        ):
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", callback)
            self.add_action(action)
        self._apply_theme(str(self.settings.get("appearance.theme", "system")))
        self._sync_autostart()
        self.settings.subscribe(self._settings_changed)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, self._signal_quit)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, self._signal_quit)

    def do_activate(self) -> None:
        if self._initialized:
            self.show_window()
            return
        self._initialized = True
        if not self._held:
            self.hold()
            self._held = True
        self.window = MainWindow(
            self,
            self.settings,
            self.history,
            self.engine,
            self._window_close_requested,
        )
        self.engine.subscribe_corrections(self._correction_from_thread)
        engine_error = ""
        if not self.no_engine:
            try:
                self.engine.start()
            except Exception as error:
                engine_error = str(error)
                LOGGER.exception("Не удалось запустить движок")
        self._sync_tray()
        self.engine.subscribe(self._engine_snapshot_from_thread)
        should_hide = self.hidden
        if not should_hide or engine_error:
            self.window.present()
        if engine_error:
            self.window.toast(f"Глобальная автокоррекция не запущена: {engine_error}")

    def show_window(self) -> bool:
        if self.window is not None:
            self.window.present()
        return GLib.SOURCE_REMOVE

    def _show_page(self, page_name: str) -> bool:
        if self.window is not None:
            self.window.show_page(page_name)
            self.window.present()
        return GLib.SOURCE_REMOVE

    def show_history(self) -> bool:
        return self._show_page("history")

    def show_exceptions(self) -> bool:
        return self._show_page("exceptions")

    def show_about(self) -> bool:
        return self._show_page("diagnostics")

    def toggle_engine(self) -> bool:
        self.settings.set("enabled", not bool(self.settings.get("enabled", True)))
        return GLib.SOURCE_REMOVE

    def toggle_sound(self) -> bool:
        self.settings.set(
            "general.sound", not bool(self.settings.get("general.sound", False))
        )
        return GLib.SOURCE_REMOVE

    def toggle_notifications(self) -> bool:
        self.settings.set(
            "general.notifications",
            not bool(self.settings.get("general.notifications", True)),
        )
        return GLib.SOURCE_REMOVE

    def _window_close_requested(self) -> bool:
        if bool(self.settings.get("general.close_to_tray", True)) and self.tray is not None:
            assert self.window is not None
            self.window.set_visible(False)
            return True
        self.quit_application()
        return True

    def _sync_tray(self) -> None:
        requested = bool(self.settings.get("appearance.show_indicator", True))
        if requested and self.tray is None:
            try:
                self.tray = StatusNotifierItem(
                    self.show_window,
                    self.toggle_engine,
                    RESOURCE_DIR,
                    on_sound_toggle=self.toggle_sound,
                    on_notifications_toggle=self.toggle_notifications,
                    on_history=self.show_history,
                    on_exceptions=self.show_exceptions,
                    on_about=self.show_about,
                    on_quit=self.quit_application,
                )
                self.tray.set_indicator_style(
                    self.settings.get("appearance.indicator_style", "letters")
                )
                self.tray.set_layout(self.engine.snapshot.current_group)
                self.tray.set_enabled(self.engine.snapshot.enabled)
                self.tray.set_sound_enabled(
                    bool(self.settings.get("general.sound", False))
                )
                self.tray.set_notifications_enabled(
                    bool(self.settings.get("general.notifications", True))
                )
            except Exception as error:
                LOGGER.warning("Системный индикатор недоступен: %s", error)
                self.tray = None
                if self.window is not None:
                    self.window.toast(f"Системный индикатор недоступен: {error}")
        elif not requested and self.tray is not None:
            self.tray.close()
            self.tray = None

    def _settings_changed(self, path: str, value: object) -> None:
        GLib.idle_add(self._apply_setting, path, value)

    def _apply_setting(self, path: str, value: object) -> bool:
        if path == "enabled" and self.tray is not None:
            self.tray.set_enabled(bool(value))
        elif path == "general.sound" and self.tray is not None:
            self.tray.set_sound_enabled(bool(value))
        elif path == "general.notifications" and self.tray is not None:
            self.tray.set_notifications_enabled(bool(value))
        elif path == "appearance.show_indicator":
            self._sync_tray()
        elif path == "appearance.indicator_style" and self.tray is not None:
            self.tray.set_indicator_style(value)
        elif path == "appearance.theme":
            self._apply_theme(str(value))
        elif path in {"general.autostart", "general.start_hidden"}:
            self._sync_autostart()
        return GLib.SOURCE_REMOVE

    def _engine_snapshot_from_thread(self, snapshot: EngineSnapshot) -> None:
        GLib.idle_add(self._apply_engine_snapshot, snapshot)

    def _apply_engine_snapshot(self, snapshot: EngineSnapshot) -> bool:
        if self.tray is not None:
            self.tray.set_layout(snapshot.current_group)
            self.tray.set_enabled(snapshot.enabled)
        return GLib.SOURCE_REMOVE

    def _sync_autostart(self) -> None:
        try:
            self.autostart.set_enabled(
                bool(self.settings.get("general.autostart", True)),
                start_hidden=bool(self.settings.get("general.start_hidden", True)),
            )
        except OSError as error:
            LOGGER.warning("Не удалось синхронизировать XDG Autostart: %s", error)

    @staticmethod
    def _apply_theme(theme: str) -> None:
        schemes = {
            "system": Adw.ColorScheme.DEFAULT,
            "light": Adw.ColorScheme.FORCE_LIGHT,
            "dark": Adw.ColorScheme.FORCE_DARK,
        }
        Adw.StyleManager.get_default().set_color_scheme(
            schemes.get(theme, Adw.ColorScheme.DEFAULT)
        )

    def _correction_from_thread(self, plan: CorrectionPlan) -> None:
        GLib.idle_add(self._announce_correction, plan)

    def _announce_correction(self, plan: CorrectionPlan) -> bool:
        if bool(self.settings.get("general.notifications", True)):
            notification = Gio.Notification.new("Раскладка исправлена")
            notification.set_body(f"{plan.original}  →  {plan.replacement}")
            notification.set_icon(Gio.ThemedIcon.new("keyswitch"))
            self.send_notification(None, notification)
        if bool(self.settings.get("general.sound", False)):
            display = Gdk.Display.get_default()
            if display is not None:
                display.beep()
        if self.window is not None and self.window.get_visible():
            self.window.toast(f"Исправлено: {plan.original} → {plan.replacement}")
        return GLib.SOURCE_REMOVE

    def quit_application(self) -> bool:
        self.quit()
        return GLib.SOURCE_REMOVE

    def _signal_quit(self) -> bool:
        self.quit()
        return GLib.SOURCE_REMOVE

    def do_shutdown(self) -> None:
        try:
            self.engine.stop()
        except Exception:
            LOGGER.exception("Ошибка остановки движка")
        if self.tray is not None:
            self.tray.close()
            self.tray = None
        if self._held:
            self.release()
            self._held = False
        Adw.Application.do_shutdown(self)


def diagnose() -> int:
    backend = X11Backend()
    probe = backend.probe()
    payload = {
        "keyswitch": __version__,
        "available": probe.available,
        "session_type": probe.session_type,
        "display": probe.display,
        "record_version": probe.record_version,
        "xtest_version": probe.xtest_version,
        "xkb_version": probe.xkb_version,
        "current_group": probe.current_group,
        "error": probe.error,
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    return 0 if probe.available else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="keyswitch", description="Автоматическое исправление раскладки Ubuntu X11")
    parser.add_argument("--hidden", action="store_true", help="не показывать окно при запуске")
    parser.add_argument("--diagnose", action="store_true", help="проверить X11 backend и завершиться")
    parser.add_argument("--no-engine", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--version", action="version", version=f"KeySwitch {__version__}")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = build_parser().parse_args(argv)
    GLib.set_prgname("keyswitch")
    GLib.set_application_name("KeySwitch")
    configure_logging()
    if arguments.diagnose:
        return diagnose()
    application = KeySwitchApplication(hidden=arguments.hidden, no_engine=arguments.no_engine)
    return application.run([sys.argv[0]])


if __name__ == "__main__":
    raise SystemExit(main())

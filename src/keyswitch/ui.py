"""GTK 4 / Libadwaita settings window."""

from __future__ import annotations

import os
import platform
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Protocol

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
gi.require_version("Gdk", "4.0")
gi.require_version("Gio", "2.0")
gi.require_version("Pango", "1.0")

from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

from . import __version__
from .config import SettingsStore
from .engine import EngineSnapshot
from .history import HistoryStore
from .logsetup import log_directory, log_path, log_status, rotation_summary
from .intent_model import IntentModelStatus
from .learning import LearningStore
from .system import AutostartManager
from .backend import BackendProbe
from .updates import UpdateController, UpdatePhase, UpdateSnapshot


RESOURCE_DIR = Path(__file__).resolve().parent / "resources"


@dataclass(frozen=True)
class ApplicationChoice:
    name: str
    identifier: str
    desktop_id: str
    icon: Gio.Icon | None

    @property
    def search_text(self) -> str:
        return f"{self.name} {self.identifier} {self.desktop_id}".casefold()


class _NavigationRow(Gtk.ListBoxRow):
    def __init__(self, page_name: str) -> None:
        super().__init__()
        self.page_name = page_name


class _ApplicationRow(Adw.ActionRow):
    def __init__(self, choice: ApplicationChoice) -> None:
        super().__init__(
            title=choice.name,
            subtitle=f"{choice.identifier} · {choice.desktop_id}",
        )
        self.choice = choice


class _UiLanguageModel(Protocol):
    @property
    def frequencies(self) -> dict[str, int]: ...

    @property
    def source(self) -> str: ...


class _UiBackend(Protocol):
    def active_application(self) -> str: ...

    def probe(self) -> BackendProbe: ...


class _UiEngine(Protocol):
    @property
    def backend(self) -> _UiBackend: ...

    @property
    def learning(self) -> LearningStore: ...

    @property
    def models(self) -> Mapping[int, _UiLanguageModel]: ...

    @property
    def intent_model_status(self) -> IntentModelStatus: ...

    def subscribe(self, callback: Callable[[EngineSnapshot], None]) -> None: ...


def installed_application_choices() -> list[ApplicationChoice]:
    """Return visible desktop applications with the best WM_CLASS candidate."""

    choices: dict[str, ApplicationChoice] = {}
    for application in Gio.AppInfo.get_all():
        try:
            if not application.should_show():
                continue
            desktop_id = (application.get_id() or "").removesuffix(".desktop")
            startup_class = ""
            if hasattr(application, "get_string"):
                startup_class = application.get_string("StartupWMClass") or ""
            executable = Path(application.get_executable() or "").name
            identifier = (startup_class or executable or desktop_id).strip()
            name = (application.get_display_name() or application.get_name() or identifier).strip()
            if not identifier or not name:
                continue
            key = identifier.casefold()
            choices.setdefault(
                key,
                ApplicationChoice(name, identifier, desktop_id, application.get_icon()),
            )
        except (GLib.Error, OSError, ValueError):
            continue
    return sorted(choices.values(), key=lambda item: (item.name.casefold(), item.identifier.casefold()))


class MainWindow(Adw.ApplicationWindow):
    NAVIGATION = (
        ("dashboard", "Обзор", "view-grid-symbolic"),
        ("automation", "Автокоррекция", "preferences-system-symbolic"),
        ("languages", "Языки", "input-keyboard-symbolic"),
        ("hotkeys", "Горячие клавиши", "preferences-desktop-keyboard-shortcuts-symbolic"),
        ("exceptions", "Исключения", "action-unavailable-symbolic"),
        ("system", "Внешний вид и система", "preferences-desktop-theme-symbolic"),
        ("updates", "Обновления", "software-update-available-symbolic"),
        ("history", "История", "document-open-recent-symbolic"),
        ("diagnostics", "О программе", "help-about-symbolic"),
    )

    def __init__(
        self,
        application: Adw.Application,
        settings: SettingsStore,
        history: HistoryStore,
        engine: _UiEngine,
        updates: UpdateController,
        on_close_request: Callable[[], bool],
    ) -> None:
        super().__init__(application=application, title="KeySwitch")
        self.settings = settings
        self.history = history
        self.engine = engine
        self.updates = updates
        self.autostart = AutostartManager()
        self._close_handler = on_close_request
        self._text_save_sources: dict[str, int] = {}
        self._settings_controls: dict[str, object] = {}
        self._application_choices: list[ApplicationChoice] | None = None
        self._application_rows: list[Adw.ActionRow] = []
        self._app_picker_dialog: Adw.Dialog | None = None
        self.set_default_size(1040, 720)
        self.set_size_request(850, 600)
        self.add_css_class("keyswitch-window")
        self._install_css()
        self._build()
        self.connect("close-request", self._on_close_request)
        self.engine.subscribe(self._engine_update_from_thread)
        self.history.subscribe(self._history_changed)
        self.settings.subscribe(self._setting_update_from_thread)
        self.updates.subscribe(self._update_from_thread)

    def _history_changed(self) -> None:
        GLib.idle_add(self.refresh_history)

    def _install_css(self) -> None:
        provider = Gtk.CssProvider()
        provider.load_from_path(str(RESOURCE_DIR / "style.css"))
        display = Gdk.Display.get_default()
        if display:
            Gtk.StyleContext.add_provider_for_display(
                display, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
            )

    def _build(self) -> None:
        self.toast_overlay = Adw.ToastOverlay()
        toolbar_view = Adw.ToolbarView()
        header = Adw.HeaderBar()
        header.set_show_title(False)
        header_title = Gtk.Label(label="Настройки KeySwitch")
        header_title.add_css_class("heading")
        header.set_title_widget(header_title)
        menu = Gtk.MenuButton(icon_name="open-menu-symbolic", tooltip_text="Меню")
        menu.set_menu_model(self._header_menu())
        header.pack_end(menu)
        toolbar_view.add_top_bar(header)

        body = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        sidebar = self._build_sidebar()
        body.append(sidebar)
        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
        self.stack.set_hexpand(True)
        self.stack.set_vexpand(True)
        body.append(self.stack)
        toolbar_view.set_content(body)
        self.toast_overlay.set_child(toolbar_view)
        self.set_content(self.toast_overlay)

        self._add_dashboard_page()
        self._add_automation_page()
        self._add_languages_page()
        self._add_hotkeys_page()
        self._add_exceptions_page()
        self._add_system_page()
        self._add_updates_page()
        self._add_history_page()
        self._add_diagnostics_page()
        self.nav_list.select_row(self.nav_list.get_row_at_index(0))

    def _header_menu(self) -> Gio.Menu:
        menu = Gio.Menu()
        menu.append("Показать обзор", "app.show")
        menu.append("Приостановить / продолжить", "app.toggle")
        menu.append("Завершить KeySwitch", "app.quit")
        return menu

    def _build_sidebar(self) -> Gtk.Widget:
        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        sidebar.set_size_request(250, -1)
        sidebar.add_css_class("keyswitch-sidebar")

        brand = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        brand.set_margin_top(20)
        brand.set_margin_bottom(12)
        brand.set_margin_start(20)
        brand.set_margin_end(16)
        icon = Gtk.Image.new_from_file(str(RESOURCE_DIR / "keyswitch.svg"))
        icon.set_pixel_size(44)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
        title = Gtk.Label(label="KeySwitch", xalign=0)
        title.add_css_class("brand-title")
        caption = Gtk.Label(label="Умная раскладка", xalign=0)
        caption.add_css_class("brand-caption")
        text.append(title)
        text.append(caption)
        brand.append(icon)
        brand.append(text)
        sidebar.append(brand)

        status = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        status.add_css_class("sidebar-status")
        status_text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
        self.sidebar_status_title = Gtk.Label(label="Автокоррекция", xalign=0)
        self.sidebar_status_title.add_css_class("heading")
        self.sidebar_status_detail = Gtk.Label(label="Запуск…", xalign=0)
        self.sidebar_status_detail.add_css_class("muted")
        self.sidebar_status_detail.set_ellipsize(Pango.EllipsizeMode.END)
        status_text.append(self.sidebar_status_title)
        status_text.append(self.sidebar_status_detail)
        status_text.set_hexpand(True)
        self.service_switch = Gtk.Switch(
            active=bool(self.settings.get("enabled", True)), valign=Gtk.Align.CENTER
        )
        self.service_switch.connect("notify::active", self._service_toggled)
        status.append(status_text)
        status.append(self.service_switch)
        sidebar.append(status)

        self.nav_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.nav_list.add_css_class("navigation-list")
        self.nav_list.connect("row-selected", self._navigation_selected)
        for name, label, icon_name in self.NAVIGATION:
            row = _NavigationRow(name)
            content = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            content.add_css_class("navigation-row")
            content.append(Gtk.Image(icon_name=icon_name))
            text_label = Gtk.Label(label=label, xalign=0)
            text_label.set_hexpand(True)
            content.append(text_label)
            row.set_child(content)
            self.nav_list.append(row)
        sidebar.append(self.nav_list)

        spacer = Gtk.Box()
        spacer.set_vexpand(True)
        sidebar.append(spacer)
        privacy = Gtk.Label(label="Локальная обработка\nБез облака и кейлогов", xalign=0)
        privacy.add_css_class("muted")
        privacy.set_margin_start(22)
        privacy.set_margin_bottom(18)
        sidebar.append(privacy)
        return sidebar

    def _new_page(self, name: str, title: str, subtitle: str) -> Gtk.Box:
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        content.add_css_class("page-content")
        heading = Gtk.Label(label=title, xalign=0)
        heading.add_css_class("page-title")
        description = Gtk.Label(label=subtitle, xalign=0, wrap=True)
        description.add_css_class("page-subtitle")
        content.append(heading)
        content.append(description)
        scroll = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER)
        scroll.set_child(content)
        self.stack.add_named(scroll, name)
        return content

    def _add_dashboard_page(self) -> None:
        page = self._new_page(
            "dashboard",
            "Обзор",
            "Состояние фоновой автокоррекции и быстрая проверка прямо в приложении.",
        )
        hero = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        hero.add_css_class("hero-card")
        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        hero_copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
        self.hero_title = Gtk.Label(label="KeySwitch готов", xalign=0)
        self.hero_title.add_css_class("hero-title")
        self.hero_subtitle = Gtk.Label(
            label="Печатайте как обычно — раскладка исправится после слова.",
            xalign=0,
            wrap=True,
        )
        self.hero_subtitle.add_css_class("hero-subtitle")
        hero_copy.append(self.hero_title)
        hero_copy.append(self.hero_subtitle)
        hero_copy.set_hexpand(True)
        self.hero_pill = Gtk.Label(label="АКТИВНО")
        self.hero_pill.add_css_class("status-pill")
        self.hero_pill.set_valign(Gtk.Align.START)
        top.append(hero_copy)
        top.append(self.hero_pill)
        hero.append(top)
        self.hero_action = Gtk.Label(label="Ожидание ввода", xalign=0)
        self.hero_action.add_css_class("hero-word")
        self.hero_action.set_ellipsize(Pango.EllipsizeMode.END)
        hero.append(self.hero_action)
        page.append(hero)

        stats = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        self.stat_corrections = self._stat_card(stats, "0", "исправлений")
        self.stat_layout = self._stat_card(stats, "—", "текущая раскладка")
        self.stat_backend = self._stat_card(stats, "…", "движок ввода")
        page.append(stats)

        check_group = Adw.PreferencesGroup(
            title="Проверить сейчас",
            description="Выберите EN и напечатайте ghbdtn: после паузы появится «привет». Можно также нажать пробел. Затем попробуйте руддщ в RU.",
        )
        test_row = Adw.ActionRow(title="Тестовое поле", subtitle="Глобальный перехват работает и внутри этого окна")
        self.test_entry = Gtk.Entry(placeholder_text="Начните печатать здесь…")
        self.test_entry.set_hexpand(True)
        self.test_entry.set_width_chars(30)
        self.test_entry.set_valign(Gtk.Align.CENTER)
        test_row.add_suffix(self.test_entry)
        check_group.add(test_row)
        page.append(check_group)

        recent_heading = Gtk.Label(label="Последние исправления", xalign=0)
        recent_heading.add_css_class("section-heading")
        page.append(recent_heading)
        self.dashboard_history = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.dashboard_history.add_css_class("boxed-list")
        page.append(self.dashboard_history)
        self.refresh_history()

    def _stat_card(self, parent: Gtk.Box, value: str, caption: str) -> Gtk.Label:
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        card.add_css_class("stat-card")
        card.set_hexpand(True)
        label = Gtk.Label(label=value, xalign=0)
        label.add_css_class("stat-value")
        description = Gtk.Label(label=caption, xalign=0)
        description.add_css_class("muted")
        card.append(label)
        card.append(description)
        parent.append(card)
        return label

    def _add_automation_page(self) -> None:
        page = self._new_page(
            "automation",
            "Автокоррекция",
            "Настройте момент срабатывания и насколько осторожно KeySwitch принимает решение.",
        )
        behavior = Adw.PreferencesGroup(title="Распознавание")
        behavior.add(self._switch_row("enabled", "Автоматически исправлять раскладку", "Главный выключатель фонового движка"))
        behavior.add(
            self._switch_row(
                "detection.respect_manual_layout",
                "Доверять ручной смене раскладки",
                "Не исправлять первое завершённое слово после переключения раскладки пользователем",
            )
        )
        minimum = Adw.SpinRow.new_with_range(2, 12, 1)
        minimum.set_title("Минимальная длина слова")
        minimum.set_subtitle(
            "Короткие фрагменты обычно не меняются; проверенный список частотных служебных слов обрабатывается отдельно"
        )
        minimum.set_value(float(self.settings.get("detection.minimum_length", 3)))
        minimum.connect("notify::value", lambda row, _p: self.settings.set("detection.minimum_length", int(row.get_value())))
        self._settings_controls["detection.minimum_length"] = minimum
        behavior.add(minimum)
        confidence = Adw.SpinRow.new_with_range(0.5, 8.0, 0.5)
        confidence.set_title("Порог уверенности")
        confidence.set_subtitle("Выше — меньше исправлений и меньше ложных срабатываний")
        confidence.set_digits(1)
        confidence.set_value(float(self.settings.get("detection.confidence", 2.0)))
        confidence.connect("notify::value", lambda row, _p: self.settings.set("detection.confidence", float(row.get_value())))
        self._settings_controls["detection.confidence"] = confidence
        behavior.add(confidence)
        behavior.add(self._switch_row("detection.aggressive", "Агрессивное распознавание", "Разрешить исправлять незнакомые слова по характерным сочетаниям букв"))
        behavior.add(self._switch_row("detection.context_aware", "Учитывать контекст", "Предыдущее слово и язык в текущем приложении помогают разрешать сомнения; контекст хранится только в памяти"))
        behavior.add(self._switch_row("detection.protect_code", "Защищать код и сокращения", "Не трогать URL, пути, слова с цифрами, ALL-CAPS и camelCase"))
        behavior.add(self._switch_row("detection.intent_model_enabled", "Локальная линейная модель", "Собственная символьная n-граммная модель проверяет сомнительные решения; введённый текст не покидает компьютер"))
        behavior.add(
            self._switch_row(
                "detection.early_switch",
                "Ранняя смена раскладки",
                "Переключать раскладку и переписывать начало слова, как только префикс невозможен в текущем языке и явно продолжается в другом",
            )
        )
        early_length = Adw.SpinRow.new_with_range(3, 8, 1)
        early_length.set_title("Букв до ранней смены")
        early_length.set_subtitle("Меньше — быстрее, но чаще ложные переключения на сокращениях и технических словах")
        early_length.set_value(float(self.settings.get("detection.early_switch_min_length", 4)))
        early_length.connect(
            "notify::value",
            lambda row, _parameter: self.settings.set(
                "detection.early_switch_min_length", int(row.get_value())
            ),
        )
        self._settings_controls["detection.early_switch_min_length"] = early_length
        behavior.add(early_length)
        page.append(behavior)

        learning = Adw.PreferencesGroup(
            title="Самообучение",
            description="KeySwitch сохраняет только слова, которые вы преобразовали вручную или вернули после ложного исправления.",
        )
        learning.add(self._switch_row("detection.learning", "Учиться на моих действиях", "После Pause/Break предложить правило: Enter подтверждает, Esc отклоняет"))
        confirmations = Adw.SpinRow.new_with_range(1, 5, 1)
        confirmations.set_title("Подтверждений для нового правила")
        confirmations.set_subtitle("Порог повторов действует, если предложение не подтверждено клавишей Enter")
        confirmations.set_value(float(self.settings.get("detection.learning_confirmations", 2)))
        confirmations.connect(
            "notify::value",
            lambda row, _parameter: self.settings.set(
                "detection.learning_confirmations", int(row.get_value())
            ),
        )
        self._settings_controls["detection.learning_confirmations"] = confirmations
        learning.add(confirmations)
        rules, rejections = self.engine.learning.counts()
        self.learning_status_row = Adw.ActionRow(
            title="Локальная модель пользователя",
            subtitle=self._learning_summary(rules, rejections),
        )
        clear_learning = Gtk.Button(label="Очистить", valign=Gtk.Align.CENTER)
        clear_learning.add_css_class("destructive-action")
        clear_learning.connect("clicked", lambda _button: self._confirm_clear_learning())
        self.learning_status_row.add_suffix(clear_learning)
        learning.add(self.learning_status_row)
        page.append(learning)

        triggers = Adw.PreferencesGroup(title="Исправлять после")
        triggers.add(
            self._switch_row(
                "detection.correct_on_pause",
                "Паузы в наборе",
                "Проверять текущее слово после паузы без ввода",
            )
        )
        pause_delay = Adw.SpinRow.new_with_range(0.3, 5.0, 0.1)
        pause_delay.set_title("Длительность паузы, с")
        pause_delay.set_subtitle("Сколько ждать без ввода, прежде чем проверить незавершённое слово")
        pause_delay.set_digits(1)
        pause_delay.set_value(float(self.settings.get("detection.pause_delay_seconds", 1.5)))
        pause_delay.connect(
            "notify::value",
            lambda row, _parameter: self.settings.set(
                "detection.pause_delay_seconds", round(float(row.get_value()), 1)
            ),
        )
        self._settings_controls["detection.pause_delay_seconds"] = pause_delay
        triggers.add(pause_delay)
        triggers.add(self._switch_row("detection.correct_on_space", "Пробела", "Основной и самый предсказуемый триггер"))
        for path, title in (("detection.correct_on_enter", "Enter"), ("detection.correct_on_tab", "Tab")):
            row = self._switch_row(path, title, "Замена после отправки или перехода небезопасна. Используйте пробел, паузу или Pause заранее.")
            row.set_sensitive(False)
            triggers.add(row)
        triggers.add(self._switch_row("detection.correct_on_punctuation", "Знака препинания", "Точка, запятая, вопросительный знак и другие границы слова"))
        page.append(triggers)

        tip = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        tip.add_css_class("privacy-card")
        tip.append(Gtk.Image(icon_name="dialog-information-symbolic"))
        label = Gtk.Label(
            label="Автокоррекция срабатывает консервативно: целевая форма должна быть явно лучше исходной. Ручное преобразование доступно горячей клавишей Pause.",
            xalign=0,
            wrap=True,
        )
        label.set_hexpand(True)
        tip.append(label)
        page.append(tip)

    def _add_languages_page(self) -> None:
        page = self._new_page(
            "languages",
            "Языки и раскладки",
            "KeySwitch использует две первые XKB-группы текущего сеанса и сопоставляет физические клавиши напрямую.",
        )
        pair = Adw.PreferencesGroup(title="Активная пара")
        first = Adw.ActionRow(title="Группа 1", subtitle="Английский — системная раскладка us")
        first.add_prefix(Gtk.Label(label="EN"))
        first.add_suffix(Gtk.Image(icon_name="object-select-symbolic"))
        pair.add(first)
        second = Adw.ActionRow(title="Группа 2", subtitle="Русский — системная раскладка ru")
        second.add_prefix(Gtk.Label(label="RU"))
        pair.add(second)
        self.language_active_row = Adw.ActionRow(title="Сейчас активна", subtitle="Определяется через XKB")
        self.language_active_label = Gtk.Label(label="—")
        self.language_active_label.add_css_class("accent")
        self.language_active_row.add_suffix(self.language_active_label)
        pair.add(self.language_active_row)
        page.append(pair)

        models = Adw.PreferencesGroup(
            title="Локальные языковые модели",
            description="Текст не отправляется в сеть. Используются словари, установленные вместе с Ubuntu Onboard, плюс небольшой встроенный словарь.",
        )
        for group, model in self.engine.models.items():
            language = "Английская" if group == 0 else "Русская"
            row = Adw.ActionRow(
                title=language,
                subtitle=f"{len(model.frequencies):,} слов · {model.source}",
            )
            row.add_suffix(Gtk.Image(icon_name="emblem-ok-symbolic"))
            models.add(row)
        intent_status = self.engine.intent_model_status
        intent_row = Adw.ActionRow(
            title="Линейная модель намерения",
            subtitle=(
                f"Версия {intent_status.version or 'unknown'} · {(intent_status.checksum or 'unknown')[:12]}"
                if intent_status.available
                else f"Безопасный fallback · {intent_status.error}"
            ),
        )
        intent_row.add_suffix(
            Gtk.Image(
                icon_name=(
                    "emblem-ok-symbolic"
                    if intent_status.available
                    else "dialog-warning-symbolic"
                )
            )
        )
        models.add(intent_row)
        page.append(models)

        example = Adw.PreferencesGroup(title="Пример физического соответствия")
        row = Adw.ActionRow(title="ghbdtn  ⇄  привет", subtitle="Одинаковые физические клавиши в группах EN и RU")
        example.add(row)
        row2 = Adw.ActionRow(title="руддщ  ⇄  hello", subtitle="Исправление работает в обе стороны")
        example.add(row2)
        page.append(example)

    def _add_hotkeys_page(self) -> None:
        page = self._new_page(
            "hotkeys",
            "Горячие клавиши",
            "Комбинации распознаются глобально. Нажмите Enter после редактирования поля.",
        )
        group = Adw.PreferencesGroup(title="Команды")
        group.add(self._hotkey_row("hotkeys.toggle", "Пауза / продолжение", "Временно выключает автоматические исправления"))
        group.add(self._hotkey_row("hotkeys.convert_last", "Преобразовать последнее слово", "Меняет раскладку принудительно, без решения словаря"))
        group.add(self._hotkey_row("hotkeys.undo", "Отменить исправление", "Возвращает последнее исправленное слово в течение 10 секунд"))
        page.append(group)
        syntax = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        syntax.add_css_class("privacy-card")
        syntax.append(Gtk.Image(icon_name="input-keyboard-symbolic"))
        syntax.append(Gtk.Label(label="Формат: Ctrl+Alt+P, Ctrl+Shift+Space или одиночная клавиша Pause. Модификаторы должны совпасть точно.", xalign=0, wrap=True))
        page.append(syntax)

    def _hotkey_row(self, path: str, title: str, subtitle: str) -> Adw.ActionRow:
        row = Adw.ActionRow(title=title, subtitle=subtitle)
        entry = Gtk.Entry(text=str(self.settings.get(path, "")), width_chars=20)
        entry.set_valign(Gtk.Align.CENTER)
        entry.connect("activate", lambda widget: self._save_hotkey(path, widget))
        focus = Gtk.EventControllerFocus()
        focus.connect("leave", lambda _controller: self._save_hotkey(path, entry))
        entry.add_controller(focus)
        row.add_suffix(entry)
        self._settings_controls[path] = entry
        return row

    def _save_hotkey(self, path: str, entry: Gtk.Entry) -> None:
        value = entry.get_text().strip()
        if not value:
            self.toast("Комбинация не может быть пустой")
            entry.set_text(str(self.settings.get(path, "")))
            return
        self.settings.set(path, value)
        self.toast("Горячая клавиша сохранена")

    def _add_exceptions_page(self) -> None:
        page = self._new_page(
            "exceptions",
            "Исключения и приватность",
            "Отключите исправления для отдельных приложений или слов, где раскладка намеренно необычна.",
        )
        picker = Adw.PreferencesGroup(
            title="Добавить приложение",
            description="Выберите окно прицелом, найдите установленное приложение в каталоге или укажите WM_CLASS вручную.",
        )
        self.active_app_row = Adw.ActionRow(
            title="Выбор приложения",
            subtitle="При выборе окна KeySwitch скроется на 2,5 секунды — переключитесь в нужное приложение.",
        )
        catalog_button = Gtk.Button(label="Из списка…", valign=Gtk.Align.CENTER)
        catalog_button.connect("clicked", lambda _button: self._show_application_picker())
        capture_button = Gtk.Button(label="Выбрать окно", valign=Gtk.Align.CENTER)
        capture_button.add_css_class("suggested-action")
        capture_button.connect("clicked", lambda _button: self._start_active_application_capture())
        self.active_app_row.add_suffix(catalog_button)
        self.active_app_row.add_suffix(capture_button)
        picker.add(self.active_app_row)

        manual_row = Adw.ActionRow(
            title="Добавить вручную",
            subtitle="Имя WM_CLASS или executable, например code, firefox или telegram-desktop",
        )
        self.manual_app_entry = Gtk.Entry(placeholder_text="WM_CLASS", width_chars=22)
        self.manual_app_entry.set_valign(Gtk.Align.CENTER)
        self.manual_app_entry.connect("activate", lambda _entry: self._add_manual_application())
        manual_button = Gtk.Button(label="Добавить", valign=Gtk.Align.CENTER)
        manual_button.connect("clicked", lambda _button: self._add_manual_application())
        manual_row.add_suffix(self.manual_app_entry)
        manual_row.add_suffix(manual_button)
        picker.add(manual_row)
        page.append(picker)

        self.apps_group = Adw.PreferencesGroup(
            title="Приложения-исключения",
            description="KeySwitch не исправляет ввод, если WM_CLASS активного окна содержит одно из этих имён.",
        )
        page.append(self.apps_group)
        self._refresh_application_exclusions()

        words_group = Adw.PreferencesGroup(
            title="Игнорируемые слова",
            description="По одному слову на строку. Проверяется исходная, ошибочно выглядящая форма.",
        )
        self.words_view = self._text_editor("exclusions.words", 100)
        words_group.add(self._editor_row(self.words_view))
        page.append(words_group)

        privacy = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        privacy.add_css_class("privacy-card")
        privacy.append(Gtk.Image(icon_name="security-high-symbolic"))
        privacy_label = Gtk.Label(
            label="KeySwitch держит в памяти только текущее слово. Полный поток клавиш не записывается; в историю попадают только пары выполненных исправлений. Для менеджеров паролей предустановлены исключения.",
            xalign=0,
            wrap=True,
        )
        privacy_label.set_hexpand(True)
        privacy.append(privacy_label)
        page.append(privacy)

    def _text_editor(self, path: str, height: int) -> Gtk.TextView:
        view = Gtk.TextView(wrap_mode=Gtk.WrapMode.WORD_CHAR, top_margin=10, bottom_margin=10, left_margin=10, right_margin=10)
        view.set_size_request(-1, height)
        values: list[str] = self.settings.get(path, [])
        view.get_buffer().set_text("\n".join(values))
        view.get_buffer().connect("changed", lambda buffer: self._debounce_text_save(path, buffer))
        return view

    @staticmethod
    def _editor_row(view: Gtk.TextView) -> Adw.ActionRow:
        row = Adw.ActionRow()
        scroll = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER, min_content_height=100)
        scroll.set_child(view)
        scroll.set_hexpand(True)
        row.set_child(scroll)
        return row

    def _debounce_text_save(self, path: str, buffer: Gtk.TextBuffer) -> None:
        old_source = self._text_save_sources.pop(path, 0)
        if old_source:
            GLib.source_remove(old_source)

        def save() -> bool:
            text = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
            values = [line.strip() for line in text.splitlines() if line.strip()]
            self.settings.set(path, values)
            self._text_save_sources.pop(path, None)
            return GLib.SOURCE_REMOVE

        self._text_save_sources[path] = GLib.timeout_add(450, save)

    def _add_system_page(self) -> None:
        page = self._new_page(
            "system",
            "Внешний вид и система",
            "Поведение окна, уведомлений и запуск вместе с рабочим столом.",
        )
        appearance = Adw.PreferencesGroup(title="Оформление")
        theme = Adw.ComboRow(title="Тема", subtitle="Можно следовать системной теме или зафиксировать вариант")
        theme.set_model(Gtk.StringList.new(["Системная", "Светлая", "Тёмная"]))
        keys = ["system", "light", "dark"]
        current = str(self.settings.get("appearance.theme", "system"))
        theme.set_selected(keys.index(current) if current in keys else 0)
        theme.connect("notify::selected", lambda row, _p: self._set_theme(keys[row.get_selected()]))
        appearance.add(theme)
        appearance.add(self._switch_row("appearance.show_indicator", "Значок в системной панели", "Щелчок открывает меню быстрых действий, средняя кнопка ставит движок на паузу"))
        indicator_style = Adw.ComboRow(
            title="Вид индикатора раскладки",
            subtitle="Показывать EN/RU или флаги США и России, как в Punto Switcher",
        )
        indicator_style.set_model(Gtk.StringList.new(["EN / RU", "Флаги стран"]))
        indicator_keys = ["letters", "flags"]
        current_indicator = str(self.settings.get("appearance.indicator_style", "letters"))
        indicator_style.set_selected(
            indicator_keys.index(current_indicator) if current_indicator in indicator_keys else 0
        )
        indicator_style.connect(
            "notify::selected",
            lambda row, _parameter: self.settings.set(
                "appearance.indicator_style", indicator_keys[row.get_selected()]
            ),
        )
        self._settings_controls["appearance.indicator_style"] = indicator_style
        appearance.add(indicator_style)
        appearance.add(self._switch_row("general.close_to_tray", "Сворачивать при закрытии окна", "Фоновая коррекция продолжит работать"))
        page.append(appearance)

        feedback = Adw.PreferencesGroup(title="Обратная связь")
        feedback.add(self._switch_row("general.notifications", "Уведомлять об исправлении", "Показывать исходное и исправленное слово"))
        feedback.add(self._switch_row("general.sound", "Звуковой сигнал", "Короткий системный сигнал после замены"))
        feedback.add(self._switch_row("general.keep_history", "Сохранять историю исправлений", "Только пары слов; полный ввод никогда не сохраняется"))
        page.append(feedback)

        startup = Adw.PreferencesGroup(title="Запуск")
        startup.add(self._autostart_row())
        startup.add(self._switch_row("general.start_hidden", "Запускать свёрнутым", "Окно не показывается, пока не нажать значок в панели"))
        page.append(startup)

        maintenance = Adw.PreferencesGroup(title="Обслуживание")
        reset_row = Adw.ActionRow(title="Вернуть настройки по умолчанию", subtitle="История исправлений при этом не удаляется")
        reset_button = Gtk.Button(label="Сбросить", valign=Gtk.Align.CENTER)
        reset_button.add_css_class("destructive-action")
        reset_button.connect("clicked", lambda _b: self._confirm_reset())
        reset_row.add_suffix(reset_button)
        maintenance.add(reset_row)
        page.append(maintenance)

    def _autostart_row(self) -> Adw.SwitchRow:
        row = Adw.SwitchRow(
            title="Запускать после входа в систему",
            subtitle="После перезагрузки KeySwitch запустится при следующем входе в рабочий стол",
        )
        row.set_active(self.autostart.enabled())
        row.connect("notify::active", self._autostart_toggled)
        return row

    def _add_updates_page(self) -> None:
        page = self._new_page(
            "updates",
            "Обновления",
            "Проверка стабильных выпусков KeySwitch и безопасный переход к системной установке.",
        )
        state = Adw.PreferencesGroup(title="Состояние")
        state.add(Adw.ActionRow(title="Установленная версия", subtitle=__version__))
        self.update_status_row = Adw.ActionRow(
            title="Проверка обновлений",
            subtitle=self.updates.snapshot.message,
        )
        state.add(self.update_status_row)
        self.update_version_row = Adw.ActionRow(
            title="Новая версия",
            subtitle="Новых выпусков пока не найдено",
        )
        state.add(self.update_version_row)
        page.append(state)

        policy = Adw.PreferencesGroup(title="Автоматическая проверка")
        policy.add(
            self._switch_row(
                "updates.check_automatically",
                "Проверять автоматически",
                "Первая проверка через 30 секунд после запуска, затем каждые шесть часов",
            )
        )
        policy.add(
            Adw.ActionRow(
                title="Установка в Ubuntu",
                subtitle=(
                    "KeySwitch уведомит о выпуске и откроет его страницу. Системный DEB "
                    "обновляется с подтверждением через APT; фоновая установка без прав "
                    "администратора отключена."
                ),
            )
        )
        page.append(policy)

        actions = Adw.PreferencesGroup(title="Действия")
        action_row = Adw.ActionRow(
            title="GitHub Releases",
            subtitle="Метаданные и SHA-256 загружаются только из официального репозитория",
        )
        buttons = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self.update_check_button = Gtk.Button(label="Проверить сейчас")
        self.update_check_button.connect("clicked", lambda _button: self._check_updates())
        self.update_open_button = Gtk.Button(label="Открыть выпуск")
        self.update_open_button.set_sensitive(bool(self.updates.snapshot.release_url))
        self.update_open_button.connect(
            "clicked", lambda _button: self._open_update_release()
        )
        buttons.append(self.update_check_button)
        buttons.append(self.update_open_button)
        action_row.add_suffix(buttons)
        actions.add(action_row)
        page.append(actions)

    def _update_from_thread(self, snapshot: UpdateSnapshot) -> None:
        GLib.idle_add(self._apply_update_snapshot, snapshot)

    def _apply_update_snapshot(self, snapshot: UpdateSnapshot) -> bool:
        self.update_status_row.set_subtitle(snapshot.message)
        self.update_version_row.set_subtitle(
            f"Доступна {snapshot.available_version}"
            if snapshot.available_version
            else "Новых выпусков пока не найдено"
        )
        self.update_check_button.set_sensitive(
            snapshot.phase not in {UpdatePhase.CHECKING, UpdatePhase.DOWNLOADING}
        )
        self.update_open_button.set_sensitive(bool(snapshot.release_url))
        return GLib.SOURCE_REMOVE

    def _check_updates(self) -> None:
        if not self.updates.check(automatic=False, install_automatically=False):
            self.toast("Проверка обновлений уже выполняется")

    def _open_update_release(self) -> None:
        release_url = self.updates.snapshot.release_url
        if not release_url:
            self.toast("Сначала проверьте наличие новой версии")
            return
        try:
            launched = Gio.AppInfo.launch_default_for_uri(release_url, None)
        except GLib.Error as error:
            self.toast(f"Не удалось открыть выпуск: {error.message}")
            return
        if not launched:
            self.toast("Не удалось открыть страницу выпуска")

    def _open_log_directory(self) -> None:
        directory = log_directory()
        try:
            launched = Gio.AppInfo.launch_default_for_uri(directory.as_uri(), None)
        except GLib.Error as error:
            self.toast(f"Не удалось открыть папку журнала: {error.message}")
            return
        if not launched:
            self.toast("Не удалось открыть папку журнала")

    def _add_history_page(self) -> None:
        page = self._new_page(
            "history",
            "История исправлений",
            "Здесь хранятся только реально заменённые пары слов, приложение и время.",
        )
        controls = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.history_total_label = Gtk.Label(label="0 записей", xalign=0)
        self.history_total_label.add_css_class("section-heading")
        self.history_total_label.set_hexpand(True)
        clear = Gtk.Button(label="Очистить историю", icon_name="user-trash-symbolic")
        clear.add_css_class("destructive-action")
        clear.connect("clicked", lambda _b: self._confirm_clear_history())
        controls.append(self.history_total_label)
        controls.append(clear)
        page.append(controls)
        self.history_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.history_list.add_css_class("boxed-list")
        page.append(self.history_list)
        self.refresh_history()

    def _add_diagnostics_page(self) -> None:
        page = self._new_page(
            "diagnostics",
            "О программе",
            "Версия, состояние системных расширений и пути к локальным данным.",
        )
        about = Adw.PreferencesGroup(title="KeySwitch")
        about.add(Adw.ActionRow(title="Версия", subtitle=__version__))
        about.add(Adw.ActionRow(title="Назначение", subtitle="Автоматическое исправление раскладки для Ubuntu X11"))
        page.append(about)

        probe = self.engine.backend.probe()
        diagnostics = Adw.PreferencesGroup(title="Диагностика backend")
        diagnostics.add(Adw.ActionRow(title="Сеанс", subtitle=f"{probe.session_type} · DISPLAY {probe.display or '—'}"))
        diagnostics.add(Adw.ActionRow(title="XRecord", subtitle=probe.record_version if probe.available else probe.error))
        diagnostics.add(Adw.ActionRow(title="XTEST", subtitle=probe.xtest_version))
        diagnostics.add(Adw.ActionRow(title="XKB", subtitle=probe.xkb_version))
        diagnostics.add(Adw.ActionRow(title="XKB-группа", subtitle=str(probe.current_group + 1) if probe.current_group >= 0 else "—"))
        diagnostics.add(Adw.ActionRow(title="Операционная система", subtitle=self._os_description()))
        page.append(diagnostics)

        locations = Adw.PreferencesGroup(title="Локальные данные")
        locations.add(Adw.ActionRow(title="Настройки", subtitle=str(self.settings.path)))
        locations.add(Adw.ActionRow(title="История", subtitle=str(self.history.path)))
        locations.add(Adw.ActionRow(title="Самообучение", subtitle=str(self.engine.learning.path)))
        page.append(locations)

        technical_log = Adw.PreferencesGroup(
            title="Технический журнал",
            description=(
                "Включайте на время воспроизведения проблемы. Журнал содержит решения модели, "
                "названия приложений и может содержать введённые слова; текст из приложений-исключений скрывается."
            ),
        )
        technical_log.add(
            self._switch_row(
                "diagnostics.technical_logging",
                "Записывать подробную диагностику распознавания",
                (
                    "По умолчанию выключено; включение начинает новый файл, "
                    f"журнал хранится только локально и ротируется по {rotation_summary(True)}"
                ),
            )
        )
        log_row = Adw.ActionRow(title="Файл журнала", subtitle=str(log_path()))
        open_log = Gtk.Button(
            icon_name="folder-symbolic",
            tooltip_text="Открыть папку журнала",
            valign=Gtk.Align.CENTER,
        )
        open_log.add_css_class("flat")
        open_log.connect("clicked", lambda _b: self._open_log_directory())
        log_row.add_suffix(open_log)
        log_row.set_activatable_widget(open_log)
        technical_log.add(log_row)
        page.append(technical_log)

        copy_button = Gtk.Button(label="Скопировать диагностику", icon_name="edit-copy-symbolic", halign=Gtk.Align.START)
        copy_button.connect("clicked", lambda _b: self._copy_diagnostics(probe))
        page.append(copy_button)

        if not probe.available:
            warning = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            warning.add_css_class("error-card")
            warning.append(Gtk.Image(icon_name="dialog-error-symbolic"))
            warning.append(Gtk.Label(label=probe.error, xalign=0, wrap=True))
            page.append(warning)

    def _switch_row(self, path: str, title: str, subtitle: str) -> Adw.SwitchRow:
        row = Adw.SwitchRow(title=title, subtitle=subtitle)
        row.set_active(bool(self.settings.get(path, False)))
        row.connect("notify::active", lambda widget, _p: self.settings.set(path, widget.get_active()))
        self._settings_controls[path] = row
        return row

    def _service_toggled(self, switch: Gtk.Switch, _parameter: object) -> None:
        self.settings.set("enabled", switch.get_active())

    def show_page(self, page_name: str) -> bool:
        for index in range(len(self.NAVIGATION)):
            row = self.nav_list.get_row_at_index(index)
            if isinstance(row, _NavigationRow) and row.page_name == page_name:
                self.nav_list.select_row(row)
                return True
        return False

    def _navigation_selected(self, _listbox: Gtk.ListBox, row: Gtk.ListBoxRow | None) -> None:
        if isinstance(row, _NavigationRow):
            self.stack.set_visible_child_name(row.page_name)
            if row.page_name == "history":
                self.refresh_history()
            elif row.page_name == "automation" and hasattr(self, "learning_status_row"):
                self._refresh_learning_status()

    def _engine_update_from_thread(self, snapshot: EngineSnapshot) -> None:
        GLib.idle_add(self._apply_engine_snapshot, snapshot)

    def _apply_engine_snapshot(self, snapshot: EngineSnapshot) -> bool:
        if self.service_switch.get_active() != snapshot.enabled:
            self.service_switch.set_active(snapshot.enabled)
        detail = "Работает в фоне" if snapshot.running and snapshot.enabled else "На паузе" if snapshot.running else "Backend недоступен"
        self.sidebar_status_detail.set_label(detail)
        self.hero_title.set_label("Автокоррекция активна" if snapshot.running and snapshot.enabled else "Автокоррекция приостановлена" if snapshot.running else "Нужна проверка backend")
        self.hero_pill.set_label("АКТИВНО" if snapshot.running and snapshot.enabled else "ПАУЗА" if snapshot.running else "ОШИБКА")
        self.hero_action.set_label(snapshot.current_word or snapshot.last_action)
        self.stat_corrections.set_label(str(snapshot.correction_count))
        self.stat_layout.set_label("EN" if snapshot.current_group == 0 else "RU" if snapshot.current_group == 1 else "—")
        self.stat_backend.set_label("X11" if snapshot.running else "—")
        self.language_active_label.set_label("EN · группа 1" if snapshot.current_group == 0 else "RU · группа 2" if snapshot.current_group == 1 else "—")
        if snapshot.last_error:
            self.hero_subtitle.set_label(snapshot.last_error)
        else:
            self.hero_subtitle.set_label("Печатайте как обычно — раскладка исправится после слова.")
        return GLib.SOURCE_REMOVE

    def _setting_update_from_thread(self, path: str, value: object) -> None:
        GLib.idle_add(self._apply_setting_update, path, value)

    def _apply_setting_update(self, path: str, value: object) -> bool:
        control = self._settings_controls.get(path)
        if isinstance(control, Adw.SwitchRow) and control.get_active() != bool(value):
            control.set_active(bool(value))
        if isinstance(control, Adw.SpinRow) and isinstance(value, (int, float)):
            if abs(control.get_value() - float(value)) > 1e-9:
                control.set_value(float(value))
        if path == "appearance.indicator_style" and isinstance(control, Adw.ComboRow):
            selected = 1 if value == "flags" else 0
            if control.get_selected() != selected:
                control.set_selected(selected)
        if path == "exclusions.applications" and hasattr(self, "apps_group"):
            self._refresh_application_exclusions()
        if path == "enabled" and self.service_switch.get_active() != bool(value):
            self.service_switch.set_active(bool(value))
        return GLib.SOURCE_REMOVE

    def refresh_history(self) -> bool:
        entries = self.history.read(200)
        if hasattr(self, "history_list"):
            self._clear_list(self.history_list)
            if not entries:
                self.history_list.append(Adw.ActionRow(title="История пока пуста", subtitle="Первое автоматическое исправление появится здесь"))
            for entry in reversed(entries):
                when = self._format_time(entry.timestamp)
                row = Adw.ActionRow(
                    title=f"{entry.original}  →  {entry.replacement}",
                    subtitle=f"{when} · {entry.application or 'неизвестное приложение'} · уверенность {entry.confidence:g}",
                )
                row.add_prefix(Gtk.Image(icon_name="emblem-ok-symbolic"))
                self.history_list.append(row)
            self.history_total_label.set_label(self._plural_entries(len(entries)))
        if hasattr(self, "dashboard_history"):
            self._clear_list(self.dashboard_history)
            if not entries:
                self.dashboard_history.append(Adw.ActionRow(title="Пока нет исправлений", subtitle="Попробуйте тестовое поле выше"))
            for entry in reversed(entries[-4:]):
                row = Adw.ActionRow(title=f"{entry.original}  →  {entry.replacement}", subtitle=self._format_time(entry.timestamp))
                self.dashboard_history.append(row)
        return GLib.SOURCE_REMOVE

    @staticmethod
    def _clear_list(listbox: Gtk.ListBox) -> None:
        while child := listbox.get_first_child():
            listbox.remove(child)

    @staticmethod
    def _format_time(value: str) -> str:
        try:
            moment = datetime.fromisoformat(value).astimezone()
            return moment.strftime("%d.%m.%Y · %H:%M")
        except ValueError:
            return value

    @staticmethod
    def _plural_entries(count: int) -> str:
        if count % 10 == 1 and count % 100 != 11:
            word = "запись"
        elif count % 10 in (2, 3, 4) and count % 100 not in (12, 13, 14):
            word = "записи"
        else:
            word = "записей"
        return f"{count} {word}"

    def _application_catalog(self) -> list[ApplicationChoice]:
        if self._application_choices is None:
            self._application_choices = installed_application_choices()
        return self._application_choices

    def _refresh_application_exclusions(self) -> None:
        for row in self._application_rows:
            self.apps_group.remove(row)
        self._application_rows.clear()
        catalog = {item.identifier.casefold(): item for item in self._application_catalog()}
        applications: list[str] = self.settings.get(
            "exclusions.applications", []
        )
        if not applications:
            row = Adw.ActionRow(
                title="Исключений пока нет",
                subtitle="Добавьте приложение одним из способов выше.",
            )
            row.add_prefix(Gtk.Image(icon_name="emblem-ok-symbolic"))
            self.apps_group.add(row)
            self._application_rows.append(row)
            return
        for identifier in applications:
            choice = catalog.get(str(identifier).casefold())
            row = Adw.ActionRow(
                title=choice.name if choice else str(identifier),
                subtitle=f"WM_CLASS / executable: {identifier}",
            )
            if choice and choice.icon:
                row.add_prefix(Gtk.Image.new_from_gicon(choice.icon))
            else:
                row.add_prefix(Gtk.Image(icon_name="application-x-executable-symbolic"))
            remove = Gtk.Button(
                icon_name="list-remove-symbolic",
                tooltip_text=f"Удалить {identifier} из исключений",
                valign=Gtk.Align.CENTER,
            )
            remove.connect(
                "clicked", lambda _button, value=str(identifier): self._remove_application_exclusion(value)
            )
            row.add_suffix(remove)
            self.apps_group.add(row)
            self._application_rows.append(row)

    def _add_application_exclusion(self, identifier: str, display_name: str = "") -> None:
        value = identifier.strip()
        if not value:
            self.toast("Не удалось определить имя приложения")
            return
        current: list[str] = self.settings.get("exclusions.applications", [])
        applications = [str(item) for item in current]
        if any(item.casefold() == value.casefold() for item in applications):
            self.toast(f"{display_name or value} уже находится в исключениях")
            return
        applications.append(value)
        self.settings.set("exclusions.applications", applications)
        self.toast(f"Добавлено в исключения: {display_name or value}")

    def _remove_application_exclusion(self, identifier: str) -> None:
        current: list[str] = self.settings.get("exclusions.applications", [])
        applications = [
            str(item)
            for item in current
            if str(item).casefold() != identifier.casefold()
        ]
        self.settings.set("exclusions.applications", applications)
        self.toast(f"Удалено из исключений: {identifier}")

    def _add_manual_application(self) -> None:
        value = self.manual_app_entry.get_text().strip()
        self._add_application_exclusion(value)
        if value:
            self.manual_app_entry.set_text("")

    def _start_active_application_capture(self) -> None:
        self.active_app_row.set_subtitle("Переключитесь в нужное приложение — захват через 2,5 секунды…")
        self.toast("KeySwitch скрыт: активируйте окно, которое нужно исключить")
        self.set_visible(False)
        GLib.timeout_add(2500, self._finish_active_application_capture)

    def _finish_active_application_capture(self) -> bool:
        application = self.engine.backend.active_application().strip()
        self.present()
        if not application or application.casefold() == "keyswitch":
            self.active_app_row.set_subtitle("Окно не выбрано. Повторите и переключитесь в другое приложение.")
            self.toast("Не удалось выбрать внешнее приложение")
            return GLib.SOURCE_REMOVE
        self.active_app_row.set_subtitle(f"Выбрано окно: {application}")
        self._add_application_exclusion(application)
        return GLib.SOURCE_REMOVE

    def _show_application_picker(self) -> None:
        dialog = Adw.Dialog()
        dialog.set_content_width(620)
        dialog.set_content_height(620)
        toolbar = Adw.ToolbarView()
        header = Adw.HeaderBar()
        title = Gtk.Label(label="Выберите приложение")
        title.add_css_class("heading")
        header.set_title_widget(title)
        close = Gtk.Button(icon_name="window-close-symbolic", tooltip_text="Закрыть")
        close.connect("clicked", lambda _button: dialog.close())
        header.pack_start(close)
        toolbar.add_top_bar(header)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        content.set_margin_top(12)
        content.set_margin_bottom(12)
        content.set_margin_start(12)
        content.set_margin_end(12)
        search = Gtk.SearchEntry(placeholder_text="Поиск по названию, WM_CLASS или executable")
        content.append(search)
        applications = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        applications.add_css_class("boxed-list")
        for choice in self._application_catalog():
            row = _ApplicationRow(choice)
            if choice.icon:
                row.add_prefix(Gtk.Image.new_from_gicon(choice.icon))
            applications.append(row)

        def matches_search(row: Gtk.ListBoxRow) -> bool:
            return isinstance(row, _ApplicationRow) and (
                search.get_text().strip().casefold() in row.choice.search_text
            )

        applications.set_filter_func(matches_search)
        search.connect("search-changed", lambda _entry: applications.invalidate_filter())

        def selected(_listbox: Gtk.ListBox, row: Gtk.ListBoxRow) -> None:
            if not isinstance(row, _ApplicationRow):
                return
            choice = row.choice
            self._add_application_exclusion(choice.identifier, choice.name)
            dialog.close()

        applications.connect("row-activated", selected)
        scroll = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER)
        scroll.set_vexpand(True)
        scroll.set_child(applications)
        content.append(scroll)
        toolbar.set_content(content)
        dialog.set_child(toolbar)
        dialog.connect("closed", lambda _dialog: setattr(self, "_app_picker_dialog", None))
        self._app_picker_dialog = dialog
        dialog.present(self)
        search.grab_focus()

    def _autostart_toggled(self, row: Adw.SwitchRow, _parameter: object) -> None:
        try:
            self.autostart.set_enabled(
                row.get_active(),
                start_hidden=bool(self.settings.get("general.start_hidden", True)),
            )
            self.settings.set("general.autostart", row.get_active())
            self.toast("Автозапуск включён" if row.get_active() else "Автозапуск выключен")
        except OSError as error:
            row.set_active(not row.get_active())
            self.toast(f"Не удалось изменить автозапуск: {error}")

    def _set_theme(self, theme: str) -> None:
        self.settings.set("appearance.theme", theme)
        manager = Adw.StyleManager.get_default()
        schemes = {
            "system": Adw.ColorScheme.DEFAULT,
            "light": Adw.ColorScheme.FORCE_LIGHT,
            "dark": Adw.ColorScheme.FORCE_DARK,
        }
        manager.set_color_scheme(schemes.get(theme, Adw.ColorScheme.DEFAULT))

    def _confirm_clear_history(self) -> None:
        dialog = Adw.AlertDialog(
            heading="Очистить историю?",
            body="Будут удалены только пары исправленных слов. Настройки не изменятся.",
        )
        dialog.add_response("cancel", "Отмена")
        dialog.add_response("clear", "Очистить")
        dialog.set_response_appearance("clear", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect("response", lambda _d, response: self._clear_history_response(response))
        dialog.present(self)

    def _clear_history_response(self, response: str) -> None:
        if response == "clear":
            self.history.clear()
            self.toast("История очищена")

    @staticmethod
    def _learning_summary(rules: int, rejections: int) -> str:
        return f"Подтверждённых правил: {rules} · запретов после отмены: {rejections}"

    def _refresh_learning_status(self) -> None:
        rules, rejections = self.engine.learning.counts()
        self.learning_status_row.set_subtitle(
            self._learning_summary(rules, rejections)
        )

    def _confirm_clear_learning(self) -> None:
        dialog = Adw.AlertDialog(
            heading="Очистить самообучение?",
            body="Будут удалены выученные правила и запомненные ложные срабатывания. Обычные настройки и история не изменятся.",
        )
        dialog.add_response("cancel", "Отмена")
        dialog.add_response("clear", "Очистить")
        dialog.set_response_appearance("clear", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect(
            "response", lambda _dialog, response: self._clear_learning_response(response)
        )
        dialog.present(self)

    def _clear_learning_response(self, response: str) -> None:
        if response != "clear":
            return
        self.engine.learning.clear()
        self._refresh_learning_status()
        self.toast("Самообучение очищено")

    def _confirm_reset(self) -> None:
        dialog = Adw.AlertDialog(
            heading="Сбросить настройки?",
            body="Автокоррекция, горячие клавиши и исключения вернутся к значениям по умолчанию.",
        )
        dialog.add_response("cancel", "Отмена")
        dialog.add_response("reset", "Сбросить")
        dialog.set_response_appearance("reset", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.connect("response", lambda _d, response: self._reset_response(response))
        dialog.present(self)

    def _reset_response(self, response: str) -> None:
        if response == "reset":
            self.settings.reset()
            self.autostart.set_enabled(
                bool(self.settings.get("general.autostart", True)),
                start_hidden=bool(self.settings.get("general.start_hidden", True)),
            )
            self.toast("Настройки сброшены; перезапустите окно для обновления всех полей")

    def _copy_diagnostics(self, probe: BackendProbe) -> None:
        text = "\n".join(
            (
                f"KeySwitch {__version__}",
                f"OS: {self._os_description()}",
                f"Session: {probe.session_type}",
                f"DISPLAY: {probe.display}",
                f"XRecord: {probe.record_version}",
                f"XTEST: {probe.xtest_version}",
                f"XKB: {probe.xkb_version}",
                f"XKB group: {probe.current_group}",
                f"Intent model: {self.engine.intent_model_status.summary}",
                f"Technical logging: {bool(self.settings.get('diagnostics.technical_logging', False))}",
                f"Technical log: {log_path()}",
                f"Log state: {log_status()}",
                f"Backend error: {probe.error or 'none'}",
            )
        )
        display = Gdk.Display.get_default()
        if display is None:
            self.toast("Буфер обмена недоступен")
            return
        clipboard = display.get_clipboard()
        clipboard.set(text)
        self.toast("Диагностика скопирована")

    @staticmethod
    def _os_description() -> str:
        try:
            fields = {}
            for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
                key, separator, value = line.partition("=")
                if separator:
                    fields[key] = value.strip('"')
            return fields.get("PRETTY_NAME", platform.platform())
        except OSError:
            return platform.platform()

    def toast(self, message: str) -> None:
        self.toast_overlay.add_toast(Adw.Toast.new(message))

    def _on_close_request(self, _window: Gtk.Window) -> bool:
        return self._close_handler()

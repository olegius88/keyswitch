"""GTK 4 / Libadwaita settings window."""

from __future__ import annotations

import os
import platform
from datetime import datetime
from pathlib import Path
from typing import Callable

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
gi.require_version("Gdk", "4.0")

from gi.repository import Adw, Gdk, GLib, Gtk  # noqa: E402

from . import __version__
from .config import SettingsStore
from .engine import EngineSnapshot, KeySwitchEngine
from .history import HistoryStore
from .system import AutostartManager


RESOURCE_DIR = Path(__file__).resolve().parent / "resources"


class MainWindow(Adw.ApplicationWindow):
    NAVIGATION = (
        ("dashboard", "Обзор", "view-grid-symbolic"),
        ("automation", "Автокоррекция", "preferences-system-symbolic"),
        ("languages", "Языки", "input-keyboard-symbolic"),
        ("hotkeys", "Горячие клавиши", "preferences-desktop-keyboard-shortcuts-symbolic"),
        ("exceptions", "Исключения", "action-unavailable-symbolic"),
        ("system", "Внешний вид и система", "preferences-desktop-theme-symbolic"),
        ("history", "История", "document-open-recent-symbolic"),
        ("diagnostics", "О программе", "help-about-symbolic"),
    )

    def __init__(
        self,
        application: Adw.Application,
        settings: SettingsStore,
        history: HistoryStore,
        engine: KeySwitchEngine,
        on_close_request: Callable[[], bool],
    ) -> None:
        super().__init__(application=application, title="KeySwitch")
        self.settings = settings
        self.history = history
        self.engine = engine
        self.autostart = AutostartManager()
        self._close_handler = on_close_request
        self._text_save_sources: dict[str, int] = {}
        self._settings_controls: dict[str, object] = {}
        self.set_default_size(1040, 720)
        self.set_size_request(850, 600)
        self.add_css_class("keyswitch-window")
        self._install_css()
        self._build()
        self.connect("close-request", self._on_close_request)
        self.engine.subscribe(self._engine_update_from_thread)
        self.history.subscribe(lambda: GLib.idle_add(self.refresh_history))
        self.settings.subscribe(self._setting_update_from_thread)

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
        self._add_history_page()
        self._add_diagnostics_page()
        self.nav_list.select_row(self.nav_list.get_row_at_index(0))

    def _header_menu(self):
        from gi.repository import Gio

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
        self.sidebar_status_detail.set_ellipsize(3)
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
            row = Gtk.ListBoxRow()
            row.page_name = name
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
        self.hero_action.set_ellipsize(3)
        hero.append(self.hero_action)
        page.append(hero)

        stats = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        self.stat_corrections = self._stat_card(stats, "0", "исправлений")
        self.stat_layout = self._stat_card(stats, "—", "текущая раскладка")
        self.stat_backend = self._stat_card(stats, "…", "движок ввода")
        page.append(stats)

        check_group = Adw.PreferencesGroup(
            title="Проверить сейчас",
            description="Выберите EN, напечатайте ghbdtn и нажмите пробел — должно появиться «привет ». Затем попробуйте руддщ в RU.",
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
        minimum = Adw.SpinRow.new_with_range(2, 12, 1)
        minimum.set_title("Минимальная длина слова")
        minimum.set_subtitle("Короткие фрагменты чаще бывают командами и сокращениями")
        minimum.set_value(float(self.settings.get("detection.minimum_length", 3)))
        minimum.connect("notify::value", lambda row, _p: self.settings.set("detection.minimum_length", int(row.get_value())))
        behavior.add(minimum)
        confidence = Adw.SpinRow.new_with_range(0.5, 8.0, 0.5)
        confidence.set_title("Порог уверенности")
        confidence.set_subtitle("Выше — меньше исправлений и меньше ложных срабатываний")
        confidence.set_digits(1)
        confidence.set_value(float(self.settings.get("detection.confidence", 2.0)))
        confidence.connect("notify::value", lambda row, _p: self.settings.set("detection.confidence", float(row.get_value())))
        behavior.add(confidence)
        behavior.add(self._switch_row("detection.aggressive", "Агрессивное распознавание", "Разрешить исправлять незнакомые слова по характерным сочетаниям букв"))
        page.append(behavior)

        triggers = Adw.PreferencesGroup(title="Исправлять после")
        triggers.add(self._switch_row("detection.correct_on_space", "Пробела", "Основной и самый предсказуемый триггер"))
        triggers.add(self._switch_row("detection.correct_on_enter", "Enter", "Работает в обычных многострочных полях"))
        triggers.add(self._switch_row("detection.correct_on_tab", "Tab", "Удобно при заполнении форм"))
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
        active = Adw.PreferencesGroup(title="Текущее приложение")
        self.active_app_row = Adw.ActionRow(title="Окно с фокусом", subtitle="Нажмите кнопку, чтобы перечитать WM_CLASS")
        active_button = Gtk.Button(label="Определить", valign=Gtk.Align.CENTER)
        active_button.connect("clicked", lambda _b: self._detect_active_application())
        self.active_app_row.add_suffix(active_button)
        active.add(self.active_app_row)
        page.append(active)

        apps_group = Adw.PreferencesGroup(
            title="Не исправлять в приложениях",
            description="По одному имени WM_CLASS на строку; совпадение без учёта регистра и допускает часть имени.",
        )
        self.apps_view = self._text_editor("exclusions.applications", 110)
        apps_group.add(self._editor_row(self.apps_view))
        page.append(apps_group)

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
        values = self.settings.get(path, [])
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
        appearance.add(self._switch_row("appearance.show_indicator", "Значок в системной панели", "Щелчок открывает окно, средняя кнопка ставит движок на паузу"))
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
        row = Adw.SwitchRow(title="Запускать вместе с системой", subtitle="Создаёт запись XDG Autostart только для текущего пользователя")
        row.set_active(self.autostart.enabled())
        row.connect("notify::active", self._autostart_toggled)
        return row

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
        page.append(locations)

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

    def _navigation_selected(self, _listbox: Gtk.ListBox, row: Gtk.ListBoxRow | None) -> None:
        if row is not None:
            self.stack.set_visible_child_name(row.page_name)
            if row.page_name == "history":
                self.refresh_history()

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

    def _detect_active_application(self) -> None:
        application = self.engine.backend.active_application() or "Не удалось определить"
        self.active_app_row.set_subtitle(application)

    def _autostart_toggled(self, row: Adw.SwitchRow, _parameter: object) -> None:
        try:
            self.autostart.set_enabled(row.get_active())
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
            self.autostart.set_enabled(False)
            self.settings.reset()
            self.toast("Настройки сброшены; перезапустите окно для обновления всех полей")

    def _copy_diagnostics(self, probe) -> None:
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
                f"Backend error: {probe.error or 'none'}",
            )
        )
        clipboard = Gdk.Display.get_default().get_clipboard()
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

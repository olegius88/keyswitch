"""Native Windows settings window and application composition."""

from __future__ import annotations

import json
import logging
import queue
import webbrowser
import winsound
from collections.abc import Callable, Iterable
from functools import partial
from pathlib import Path
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from . import __version__
from .backend import ScreenAnchor
from .config import SettingsStore
from .engine import (
    CorrectionPlan,
    EngineSnapshot,
    KeySwitchEngine,
    LearningPrompt,
)
from .history import HistoryEntry, HistoryStore, data_dir
from .indicator import layout_label
from .windows_backend import WindowsBackend
from .windows_system import (
    WindowsApplication as CatalogApplication,
    WindowsApplicationCatalog,
    WindowsAutostartManager,
)
from .windows_tray import WindowsTray, WindowsTrayActions
from .windows_ui_model import (
    AUTOCORRECTION_SETTINGS,
    HOTKEY_SETTINGS,
    SYSTEM_SETTINGS,
    TRIGGER_SETTINGS,
    UPDATE_SETTINGS,
    SettingSpec,
)
from .updates import (
    UpdateManager,
    UpdatePhase,
    UpdateSnapshot,
    launch_windows_installer,
)


LOGGER = logging.getLogger(__name__)
PAGE_NAMES = (
    ("overview", "Обзор"),
    ("autocorrection", "Автокоррекция"),
    ("layouts", "Раскладки"),
    ("hotkeys", "Горячие клавиши"),
    ("exclusions", "Исключения"),
    ("appearance", "Внешний вид и система"),
    ("history", "История"),
    ("updates", "Обновления"),
    ("maintenance", "Обслуживание"),
    ("about", "О программе"),
)
WINDOWS_LEARNING_PROMPT_DELAY_MS = 200
WINDOWS_UPDATE_INITIAL_DELAY_MS = 30_000
WINDOWS_UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000


class WindowsLearningPrompt:
    """Keyboard-focused learning prompt positioned above the Win32 caret."""

    def __init__(
        self,
        root: tk.Tk,
        backend: WindowsBackend,
        confirm: Callable[[LearningPrompt], bool],
        dismiss: Callable[[LearningPrompt], bool],
    ) -> None:
        self.root = root
        self.backend = backend
        self.confirm = confirm
        self.dismiss = dismiss
        self.prompt: LearningPrompt | None = None
        self.anchor: ScreenAnchor | None = None
        self.window = tk.Toplevel(root, class_="KeySwitchLearningPrompt")
        self.window.withdraw()
        self.window.overrideredirect(True)
        self.window.attributes("-topmost", True)
        self.window.configure(background="#7657ff", borderwidth=1)

        card = tk.Frame(
            self.window,
            background="#171a21",
            padx=16,
            pady=12,
        )
        card.pack(fill="both", expand=True)
        tk.Label(
            card,
            text="Добавить слово в правила переключения?",
            background="#171a21",
            foreground="#ffffff",
            font=("Segoe UI Semibold", 10),
            anchor="w",
        ).pack(fill="x")
        self.word = tk.Label(
            card,
            background="#171a21",
            foreground="#9a84ff",
            font=("Segoe UI Semibold", 11),
            anchor="w",
        )
        self.word.pack(fill="x", pady=(5, 2))
        tk.Label(
            card,
            text="Enter - ДА    Esc - НЕТ",
            background="#171a21",
            foreground="#b7bdc9",
            font=("Segoe UI", 9),
            anchor="w",
        ).pack(fill="x")
        self.window.bind("<Return>", self._confirm_event)
        self.window.bind("<Escape>", self._dismiss_event)

    def show_prompt(
        self, prompt: LearningPrompt, anchor: ScreenAnchor | None
    ) -> None:
        self.prompt = prompt
        self.anchor = anchor
        self.word.configure(text=f"{prompt.original}  →  {prompt.replacement}")
        self.window.update_idletasks()
        width = max(390, self.window.winfo_reqwidth())
        height = self.window.winfo_reqheight()
        if anchor is None:
            pointer_x, pointer_y = self.root.winfo_pointerxy()
            anchor = ScreenAnchor(pointer_x, pointer_y)
            self.anchor = anchor
        x = anchor.x - width // 2
        y = anchor.y - height - 12
        virtual_x = self.root.winfo_vrootx()
        virtual_y = self.root.winfo_vrooty()
        virtual_width = self.root.winfo_vrootwidth()
        virtual_height = self.root.winfo_vrootheight()
        x = max(virtual_x + 8, min(x, virtual_x + virtual_width - width - 8))
        y = max(virtual_y + 8, min(y, virtual_y + virtual_height - height - 8))
        self.window.geometry(f"{width}x{height}+{x}+{y}")
        self.window.deiconify()
        self.window.lift()
        self.window.focus_force()

    def hide_prompt(self) -> None:
        anchor = self.anchor
        self.prompt = None
        self.anchor = None
        self.window.withdraw()
        if anchor is not None and anchor.window is not None:
            self.root.after_idle(
                lambda: self.backend.restore_window(anchor.window)
            )

    def destroy(self) -> None:
        self.prompt = None
        self.anchor = None
        self.window.destroy()

    def _confirm_event(self, _event: tk.Event[tk.Misc]) -> str:
        prompt = self.prompt
        if prompt is not None:
            self.confirm(prompt)
        return "break"

    def _dismiss_event(self, _event: tk.Event[tk.Misc]) -> str:
        prompt = self.prompt
        if prompt is not None:
            self.dismiss(prompt)
        return "break"


class WindowsApplication:
    """Own Tk, the global engine, tray and per-user desktop integration."""

    def __init__(
        self,
        *,
        hidden: bool,
        no_engine: bool,
        enable_updates: bool = True,
    ) -> None:
        self.hidden = hidden
        self.no_engine = no_engine
        self.enable_updates = enable_updates
        self.settings = SettingsStore()
        self.history = HistoryStore(limit=int(self.settings.get("history.limit", 200)))
        self.backend = WindowsBackend()
        self.engine = KeySwitchEngine(
            self.settings,
            self.history,
            backend=self.backend,
            backend_label="Win32 hook + SendInput",
        )
        self.autostart = WindowsAutostartManager()
        self.catalog = WindowsApplicationCatalog()
        self.tray: WindowsTray | None = None
        self._closing = False
        self._events: queue.SimpleQueue[Callable[[], None]] = queue.SimpleQueue()
        self.updates = UpdateManager(
            __version__,
            data_dir() / "updates",
            install_request=self._install_request_from_thread,
        )
        self._pages: dict[str, ttk.Frame] = {}
        self._navigation: dict[str, tk.Button] = {}
        self._active_page = "overview"
        self._sidebar_background = "#18233a"
        self._navigation_accent = "#4f7cff"
        self._navigation_hover = "#293957"
        self._boolean_variables: dict[str, tk.BooleanVar] = {}
        self._string_variables: dict[str, tk.StringVar] = {}
        self._choice_labels: dict[str, dict[str, str]] = {}
        self._catalog_items: dict[str, CatalogApplication] = {}
        self._update_after_id: str | None = None
        self._last_update_notification = ""

        self.root = tk.Tk(className="KeySwitch")
        self.root.title("KeySwitch")
        self.root.geometry("1080x760")
        self.root.minsize(900, 640)
        self.root.protocol("WM_DELETE_WINDOW", self._window_close)
        # A Tk font description is a Tcl list; brace the Windows family name
        # so its embedded space is not parsed as a bogus size/style token.
        self.root.option_add("*Font", "{Segoe UI} 10")
        self.learning_prompt = WindowsLearningPrompt(
            self.root,
            self.backend,
            self.engine.confirm_learning_prompt,
            self.engine.dismiss_learning_prompt,
        )

        self.status_text = tk.StringVar(master=self.root, value="Подготовка движка…")
        self.layout_text = tk.StringVar(master=self.root, value="Раскладка: —")
        self.count_text = tk.StringVar(master=self.root, value="Исправлений: 0")
        self.last_action_text = tk.StringVar(master=self.root, value="Ожидание ввода")
        self.error_text = tk.StringVar(master=self.root, value="")
        self.catalog_selection = tk.StringVar(master=self.root, value="")
        self.manual_application = tk.StringVar(master=self.root, value="")
        self.manual_word = tk.StringVar(master=self.root, value="")
        self.learning_text = tk.StringVar(master=self.root, value="")
        self.update_status_text = tk.StringVar(
            master=self.root,
            value=self.updates.snapshot.message,
        )
        self.update_version_text = tk.StringVar(
            master=self.root,
            value=f"Установлена версия {__version__}",
        )
        self.update_progress_text = tk.StringVar(master=self.root, value="")

        self.application_list: tk.Listbox
        self.word_list: tk.Listbox
        self.catalog_combo: ttk.Combobox
        self.history_tree: ttk.Treeview
        self.diagnostics_text: tk.Text
        self.test_entry: ttk.Entry
        self.update_check_button: ttk.Button
        self.update_install_button: ttk.Button
        self.update_release_button: ttk.Button
        self._build_window()
        self._apply_theme(str(self.settings.get("appearance.theme", "system")))

        self.settings.subscribe(self._setting_from_thread)
        self.history.subscribe(lambda: self._post(self._refresh_history))
        self.engine.subscribe(self._snapshot_from_thread)
        self.engine.subscribe_corrections(self._correction_from_thread)
        self.engine.subscribe_learning_prompts(self._learning_prompt_from_thread)
        self.updates.subscribe(self._update_from_thread)

    def _build_window(self) -> None:
        shell = ttk.Frame(self.root, style="App.TFrame")
        shell.pack(fill="both", expand=True)

        sidebar = ttk.Frame(shell, width=225, padding=(16, 18), style="Sidebar.TFrame")
        sidebar.pack(side="left", fill="y")
        sidebar.pack_propagate(False)
        ttk.Label(sidebar, text="KeySwitch", style="Brand.TLabel").pack(
            anchor="w", padx=8, pady=(0, 2)
        )
        ttk.Label(
            sidebar,
            text="умная раскладка EN / RU",
            style="MutedSidebar.TLabel",
        ).pack(anchor="w", padx=8, pady=(0, 18))
        for page_name, title in PAGE_NAMES:
            # Native ttk themes are allowed to ignore custom button colours.
            # A classic Tk button keeps the dark sidebar readable on every
            # supported Windows theme while the rest of the UI remains ttk.
            button = tk.Button(
                sidebar,
                text=title,
                command=partial(self.show_page, page_name),
                anchor="w",
                background=self._sidebar_background,
                foreground="white",
                activebackground=self._navigation_hover,
                activeforeground="white",
                borderwidth=0,
                highlightthickness=0,
                relief="flat",
                cursor="hand2",
                padx=12,
                pady=8,
            )
            button.pack(fill="x", pady=2)
            self._navigation[page_name] = button
        ttk.Separator(sidebar).pack(fill="x", pady=16)
        ttk.Label(sidebar, textvariable=self.layout_text, style="Sidebar.TLabel").pack(
            anchor="w", padx=8
        )
        ttk.Label(sidebar, textvariable=self.status_text, style="MutedSidebar.TLabel").pack(
            anchor="w", padx=8, pady=(4, 0)
        )

        content_shell = ttk.Frame(shell, style="Content.TFrame")
        content_shell.pack(side="left", fill="both", expand=True)
        content_shell.rowconfigure(0, weight=1)
        content_shell.columnconfigure(0, weight=1)

        self._build_overview(content_shell)
        self._build_autocorrection(content_shell)
        self._build_layouts(content_shell)
        self._build_hotkeys(content_shell)
        self._build_exclusions(content_shell)
        self._build_appearance(content_shell)
        self._build_history(content_shell)
        self._build_updates(content_shell)
        self._build_maintenance(content_shell)
        self._build_about(content_shell)
        self.show_page("overview")

    def _new_page(
        self,
        parent: ttk.Frame,
        name: str,
        title: str,
        subtitle: str,
    ) -> ttk.Frame:
        page = ttk.Frame(parent, padding=(30, 24), style="Content.TFrame")
        page.grid(row=0, column=0, sticky="nsew")
        page.columnconfigure(0, weight=1)
        ttk.Label(page, text=title, style="PageTitle.TLabel").grid(
            row=0, column=0, sticky="w"
        )
        ttk.Label(page, text=subtitle, style="PageSubtitle.TLabel", wraplength=760).grid(
            row=1, column=0, sticky="ew", pady=(3, 18)
        )
        self._pages[name] = page
        return page

    @staticmethod
    def _section(parent: ttk.Frame, title: str, row: int) -> ttk.LabelFrame:
        section = ttk.LabelFrame(parent, text=title, padding=(18, 14))
        section.grid(row=row, column=0, sticky="ew", pady=(0, 14))
        section.columnconfigure(0, weight=1)
        return section

    def _build_overview(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "overview",
            "KeySwitch работает рядом с вами",
            "Автоматическая замена раскладки выполняется локально после завершения слова.",
        )
        status = self._section(page, "Состояние", 2)
        ttk.Label(status, textvariable=self.status_text, style="CardTitle.TLabel").grid(
            row=0, column=0, sticky="w"
        )
        ttk.Label(status, textvariable=self.layout_text).grid(row=1, column=0, sticky="w", pady=(6, 0))
        ttk.Label(status, textvariable=self.count_text).grid(row=2, column=0, sticky="w", pady=(4, 0))
        ttk.Label(status, textvariable=self.last_action_text, wraplength=720).grid(
            row=3, column=0, sticky="w", pady=(4, 0)
        )
        ttk.Label(status, textvariable=self.error_text, style="Error.TLabel", wraplength=720).grid(
            row=4, column=0, sticky="w", pady=(4, 0)
        )
        self._add_setting(status, AUTOCORRECTION_SETTINGS[0], 5)

        test = self._section(page, "Проверка в реальном поле", 3)
        ttk.Label(
            test,
            text="Выберите EN и напечатайте ghbdtn: после паузы появится «привет». Можно также нажать пробел. Потом проверьте обратное направление.",
            wraplength=720,
        ).grid(row=0, column=0, sticky="w")
        self.test_entry = ttk.Entry(test, font=("Segoe UI", 14))
        self.test_entry.grid(row=1, column=0, sticky="ew", pady=(12, 2))
        ttk.Label(
            test,
            text="Поле обрабатывается тем же глобальным Win32 backend, что и другие приложения.",
            style="Muted.TLabel",
        ).grid(row=2, column=0, sticky="w")

    def _build_autocorrection(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "autocorrection",
            "Автокоррекция",
            "Настройте точность распознавания, обучение и события завершения слова.",
        )
        columns = ttk.Frame(page, style="Content.TFrame")
        columns.grid(row=2, column=0, sticky="nsew")
        columns.columnconfigure(0, weight=3)
        columns.columnconfigure(1, weight=2)
        behavior = ttk.LabelFrame(columns, text="Распознавание", padding=(18, 14))
        behavior.grid(row=0, column=0, sticky="nsew", padx=(0, 7))
        behavior.columnconfigure(0, weight=1)
        for row, spec in enumerate(AUTOCORRECTION_SETTINGS):
            self._add_setting(behavior, spec, row, description_wrap=310)
        triggers = ttk.LabelFrame(columns, text="Когда проверять слово", padding=(18, 14))
        triggers.grid(row=0, column=1, sticky="nsew", padx=(7, 0))
        triggers.columnconfigure(0, weight=1)
        for row, spec in enumerate(TRIGGER_SETTINGS):
            self._add_setting(triggers, spec, row, description_wrap=310)

    def _build_layouts(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "layouts",
            "Раскладки EN / RU",
            "KeySwitch находит установленные английскую и русскую HKL и сохраняет выбранный пользователем язык.",
        )
        section = self._section(page, "Системная пара", 2)
        ttk.Label(
            section,
            text="Английская раскладка",
            style="CardTitle.TLabel",
        ).grid(row=0, column=0, sticky="w")
        ttk.Label(section, text="EN — primary language 0x09").grid(row=1, column=0, sticky="w", pady=(2, 10))
        ttk.Label(section, text="Русская раскладка", style="CardTitle.TLabel").grid(row=2, column=0, sticky="w")
        ttk.Label(section, text="RU — primary language 0x19").grid(row=3, column=0, sticky="w", pady=(2, 10))
        ttk.Button(section, text="Обновить диагностику", command=self._refresh_diagnostics).grid(
            row=4, column=0, sticky="w"
        )
        behavior = self._section(page, "Ручное переключение", 3)
        manual_spec = next(
            spec
            for spec in AUTOCORRECTION_SETTINGS
            if spec.path == "detection.respect_manual_layout"
        )
        self._add_setting(behavior, manual_spec, 0)
        ttk.Label(
            behavior,
            text="Когда вы сами меняете язык перед вводом, первое завершённое слово остаётся в выбранной раскладке. Следующие слова снова анализируются автоматически.",
            wraplength=720,
        ).grid(row=1, column=0, columnspan=2, sticky="w", pady=(8, 0))

    def _build_hotkeys(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "hotkeys",
            "Горячие клавиши",
            "Комбинации работают глобально. Используйте названия Ctrl, Alt, Shift, Super, Pause и букв.",
        )
        section = self._section(page, "Глобальные команды", 2)
        for row, spec in enumerate(HOTKEY_SETTINGS):
            self._add_setting(section, spec, row)

    def _build_exclusions(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "exclusions",
            "Исключения",
            "В исключённых программах KeySwitch наблюдает раскладку, но не заменяет введённые слова.",
        )
        applications = self._section(page, "Программы", 2)
        applications.columnconfigure(0, weight=1)
        self.application_list = tk.Listbox(applications, height=6, exportselection=False)
        self.application_list.grid(row=0, column=0, columnspan=4, sticky="ew")
        ttk.Button(applications, text="Активное окно", command=self._add_active_application).grid(
            row=1, column=0, sticky="w", pady=(10, 8)
        )
        ttk.Button(applications, text="Выбрать .exe…", command=self._pick_executable).grid(
            row=1, column=1, sticky="w", padx=(8, 0), pady=(10, 8)
        )
        ttk.Button(applications, text="Удалить", command=self._remove_application).grid(
            row=1, column=3, sticky="e", pady=(10, 8)
        )
        self.catalog_combo = ttk.Combobox(
            applications,
            textvariable=self.catalog_selection,
            state="readonly",
        )
        self.catalog_combo.grid(row=2, column=0, columnspan=2, sticky="ew")
        ttk.Button(applications, text="Обновить список", command=self._load_catalog).grid(
            row=2, column=2, padx=(8, 0)
        )
        ttk.Button(applications, text="Добавить из списка", command=self._add_catalog_application).grid(
            row=2, column=3, padx=(8, 0)
        )
        manual_entry = ttk.Entry(applications, textvariable=self.manual_application)
        manual_entry.grid(row=3, column=0, columnspan=3, sticky="ew", pady=(8, 0))
        ttk.Button(applications, text="Добавить вручную", command=self._add_manual_application).grid(
            row=3, column=3, padx=(8, 0), pady=(8, 0)
        )

        words = self._section(page, "Слова", 3)
        words.columnconfigure(0, weight=1)
        self.word_list = tk.Listbox(words, height=5, exportselection=False)
        self.word_list.grid(row=0, column=0, columnspan=2, sticky="ew")
        ttk.Entry(words, textvariable=self.manual_word).grid(row=1, column=0, sticky="ew", pady=(8, 0))
        buttons = ttk.Frame(words)
        buttons.grid(row=1, column=1, padx=(8, 0), pady=(8, 0))
        ttk.Button(buttons, text="Добавить", command=self._add_word).pack(side="left")
        ttk.Button(buttons, text="Удалить", command=self._remove_word).pack(side="left", padx=(8, 0))
        self._refresh_exclusion_lists()

    def _build_appearance(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "appearance",
            "Внешний вид и система",
            "Настройте автозагрузку после входа в Windows, трей, уведомления и локальную историю.",
        )
        section = self._section(page, "Интеграция с Windows", 2)
        for row, spec in enumerate(SYSTEM_SETTINGS):
            self._add_setting(section, spec, row)

    def _build_history(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "history",
            "История исправлений",
            "Сохраняются только пары слов, время, приложение и уверенность — не полный поток клавиатуры.",
        )
        toolbar = ttk.Frame(page, style="Content.TFrame")
        toolbar.grid(row=2, column=0, sticky="ew", pady=(0, 10))
        ttk.Button(toolbar, text="Обновить", command=self._refresh_history).pack(side="left")
        ttk.Button(toolbar, text="Очистить историю", command=self._clear_history).pack(side="right")
        columns = ("time", "original", "replacement", "application", "confidence")
        self.history_tree = ttk.Treeview(page, columns=columns, show="headings")
        headings = {
            "time": "Время UTC",
            "original": "Было",
            "replacement": "Стало",
            "application": "Приложение",
            "confidence": "Уверенность",
        }
        widths = {"time": 165, "original": 130, "replacement": 130, "application": 150, "confidence": 100}
        for column in columns:
            self.history_tree.heading(column, text=headings[column])
            self.history_tree.column(column, width=widths[column], anchor="w")
        self.history_tree.grid(row=3, column=0, sticky="nsew")
        page.rowconfigure(3, weight=1)
        self._refresh_history()

    def _build_updates(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "updates",
            "Обновления",
            "KeySwitch проверяет стабильные выпуски, сверяет SHA-256 и может тихо обновиться с автоматическим перезапуском.",
        )
        state = self._section(page, "Состояние", 2)
        ttk.Label(
            state,
            textvariable=self.update_version_text,
            style="CardTitle.TLabel",
        ).grid(row=0, column=0, sticky="w")
        ttk.Label(
            state,
            textvariable=self.update_status_text,
            wraplength=720,
        ).grid(row=1, column=0, sticky="w", pady=(6, 0))
        ttk.Label(
            state,
            textvariable=self.update_progress_text,
            style="Muted.TLabel",
        ).grid(row=2, column=0, sticky="w", pady=(4, 0))

        policy = self._section(page, "Политика", 3)
        for row, spec in enumerate(UPDATE_SETTINGS):
            self._add_setting(policy, spec, row)

        actions = self._section(page, "Действия", 4)
        controls = ttk.Frame(actions)
        controls.grid(row=0, column=0, sticky="w")
        self.update_check_button = ttk.Button(
            controls,
            text="Проверить сейчас",
            command=self._check_updates,
        )
        self.update_check_button.pack(side="left")
        self.update_install_button = ttk.Button(
            controls,
            text="Установить сейчас",
            command=self._install_available_update,
            state="disabled",
        )
        self.update_install_button.pack(side="left", padx=(8, 0))
        self.update_release_button = ttk.Button(
            controls,
            text="Открыть выпуск",
            command=self._open_update_release,
            state="disabled",
        )
        self.update_release_button.pack(side="left", padx=(8, 0))
        ttk.Label(
            actions,
            text=(
                "Автоустановка работает для Setup EXE текущего пользователя и не требует "
                "прав администратора. При обновлении окно закроется и KeySwitch запустится снова."
            ),
            wraplength=720,
            style="Muted.TLabel",
        ).grid(row=1, column=0, sticky="w", pady=(10, 0))

    def _update_from_thread(self, snapshot: UpdateSnapshot) -> None:
        self._post(partial(self._apply_update_snapshot, snapshot))

    def _apply_update_snapshot(self, snapshot: UpdateSnapshot) -> None:
        self.update_status_text.set(snapshot.message)
        self.update_version_text.set(
            f"Доступна версия {snapshot.available_version}"
            if snapshot.available_version
            else f"Установлена версия {snapshot.current_version}"
        )
        self.update_progress_text.set(
            f"Загрузка: {snapshot.progress}%"
            if snapshot.phase is UpdatePhase.DOWNLOADING
            else ""
        )
        busy = snapshot.phase in {UpdatePhase.CHECKING, UpdatePhase.DOWNLOADING}
        self.update_check_button.state(["disabled"] if busy else ["!disabled"])
        can_install = snapshot.phase is UpdatePhase.AVAILABLE
        self.update_install_button.state(
            ["!disabled"] if can_install else ["disabled"]
        )
        self.update_release_button.state(
            ["!disabled"] if snapshot.release_url else ["disabled"]
        )
        if (
            can_install
            and snapshot.available_version != self._last_update_notification
        ):
            self._last_update_notification = snapshot.available_version
            if (
                bool(self.settings.get("general.notifications", True))
                and self.tray is not None
            ):
                self.tray.notify("Доступно обновление KeySwitch", snapshot.message)

    def _check_updates(self) -> None:
        if not self.updates.check(automatic=False, install_automatically=False):
            self.update_status_text.set("Проверка обновлений уже выполняется")

    def _install_available_update(self) -> None:
        if not self.updates.install_available():
            self.update_status_text.set("Сначала проверьте наличие новой версии")

    def _open_update_release(self) -> None:
        release_url = self.updates.snapshot.release_url
        if not release_url:
            self.update_status_text.set("Сначала проверьте наличие новой версии")
            return
        if not webbrowser.open(release_url):
            self.update_status_text.set("Не удалось открыть страницу выпуска")

    def _install_request_from_thread(self, path: Path) -> None:
        self._post(partial(self._install_update, path))

    def _install_update(self, path: Path) -> None:
        try:
            launch_windows_installer(path)
        except Exception as error:
            LOGGER.exception("Не удалось запустить установщик KeySwitch")
            self.updates.installation_failed(error)
            return
        self.shutdown()

    def _build_maintenance(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "maintenance",
            "Обслуживание",
            "Управляйте локальными правилами, настройками и расположением данных KeySwitch.",
        )
        learning = self._section(page, "Локальное обучение", 2)
        ttk.Label(learning, textvariable=self.learning_text).grid(
            row=0,
            column=0,
            sticky="w",
        )
        ttk.Button(
            learning,
            text="Очистить правила и запреты",
            command=self._clear_learning,
        ).grid(row=1, column=0, sticky="w", pady=(10, 0))

        settings = self._section(page, "Настройки", 3)
        ttk.Label(
            settings,
            text="Вернуть все параметры по умолчанию. История и выученные правила сохранятся.",
            wraplength=720,
        ).grid(row=0, column=0, sticky="w")
        ttk.Button(
            settings,
            text="Сбросить настройки",
            command=self._reset_settings,
        ).grid(row=1, column=0, sticky="w", pady=(10, 0))

        locations = self._section(page, "Локальные файлы", 4)
        ttk.Label(
            locations,
            text=(
                f"Настройки: {self.settings.path}\n"
                f"История: {self.history.path}\n"
                f"Самообучение: {self.engine.learning.path}\n"
                f"Журнал: {data_dir() / 'keyswitch.log'}"
            ),
            wraplength=720,
        ).grid(row=0, column=0, sticky="w")
        self._refresh_learning()

    def _build_about(self, parent: ttk.Frame) -> None:
        page = self._new_page(
            parent,
            "about",
            f"KeySwitch {__version__}",
            "Локальное приложение автоматического исправления раскладки для Windows и Linux.",
        )
        details = self._section(page, "Диагностика Win32 backend", 2)
        self.diagnostics_text = tk.Text(details, height=15, wrap="word", borderwidth=0)
        self.diagnostics_text.grid(row=0, column=0, sticky="nsew")
        buttons = ttk.Frame(details)
        buttons.grid(row=1, column=0, sticky="w", pady=(10, 0))
        ttk.Button(buttons, text="Обновить", command=self._refresh_diagnostics).pack(side="left")
        ttk.Button(buttons, text="Копировать", command=self._copy_diagnostics).pack(side="left", padx=(8, 0))
        ttk.Label(
            page,
            text="Лицензия: GNU GPL-3.0-or-later · https://github.com/olegius88/keyswitch",
            style="Muted.TLabel",
        ).grid(row=3, column=0, sticky="w")
        self._refresh_diagnostics()

    def _add_setting(
        self,
        parent: ttk.LabelFrame,
        spec: SettingSpec,
        row: int,
        *,
        description_wrap: int = 620,
    ) -> None:
        cell = ttk.Frame(parent)
        cell.grid(row=row, column=0, sticky="ew", pady=5)
        cell.columnconfigure(0, weight=1)
        if spec.kind == "bool":
            variable = self._boolean_variables.get(spec.path)
            if variable is None:
                variable = tk.BooleanVar(
                    master=self.root,
                    value=bool(self.settings.get(spec.path, False)),
                )
                self._boolean_variables[spec.path] = variable
            ttk.Checkbutton(
                cell,
                text=spec.title,
                variable=variable,
                command=partial(self._save_boolean, spec.path, variable),
            ).grid(row=0, column=0, columnspan=2, sticky="w")
        else:
            ttk.Label(cell, text=spec.title, style="CardTitle.TLabel").grid(
                row=0, column=0, sticky="w"
            )
            control = self._setting_control(cell, spec)
            control.grid(row=0, column=1, sticky="e", padx=(16, 0))
        ttk.Label(
            cell,
            text=spec.description,
            style="Muted.TLabel",
            wraplength=description_wrap,
            justify="left",
        ).grid(
            row=1,
            column=0,
            columnspan=2,
            sticky="w",
            padx=(24 if spec.kind == "bool" else 0, 0),
        )

    def _setting_control(self, parent: ttk.Frame, spec: SettingSpec) -> ttk.Widget:
        stored = self.settings.get(spec.path, "")
        if spec.kind == "choice":
            labels = dict(spec.choices)
            self._choice_labels[spec.path] = labels
            variable = self._string_variables.setdefault(
                spec.path,
                tk.StringVar(master=self.root, value=labels.get(str(stored), str(stored))),
            )
            choice_control = ttk.Combobox(
                parent,
                width=22,
                state="readonly",
                values=tuple(labels.values()),
                textvariable=variable,
            )
            choice_control.bind(
                "<<ComboboxSelected>>",
                partial(self._save_choice_event, spec.path, variable),
            )
            return choice_control
        variable = self._string_variables.setdefault(
            spec.path,
            tk.StringVar(master=self.root, value=str(stored)),
        )
        if spec.kind in {"int", "float"}:
            number_control = ttk.Spinbox(
                parent,
                width=12,
                from_=spec.minimum,
                to=spec.maximum,
                increment=spec.step,
                textvariable=variable,
                command=partial(self._save_number, spec, variable),
            )
            number_control.bind(
                "<FocusOut>",
                partial(self._save_number_event, spec, variable),
            )
            return number_control
        text_control = ttk.Entry(parent, width=25, textvariable=variable)
        text_control.bind(
            "<FocusOut>",
            partial(self._save_text_event, spec.path, variable),
        )
        text_control.bind(
            "<Return>",
            partial(self._save_text_event, spec.path, variable),
        )
        return text_control

    def _save_boolean(self, path: str, variable: tk.BooleanVar) -> None:
        self.settings.set(path, variable.get())

    def _save_text(self, path: str, variable: tk.StringVar) -> None:
        value = variable.get().strip()
        if value:
            self.settings.set(path, value)

    def _save_text_event(
        self,
        path: str,
        variable: tk.StringVar,
        _event: tk.Event[tk.Misc],
    ) -> None:
        self._save_text(path, variable)

    def _save_choice(self, path: str, variable: tk.StringVar) -> None:
        selected = variable.get()
        values = self._choice_labels[path]
        internal = next((key for key, label in values.items() if label == selected), selected)
        self.settings.set(path, internal)

    def _save_choice_event(
        self,
        path: str,
        variable: tk.StringVar,
        _event: tk.Event[tk.Misc],
    ) -> None:
        self._save_choice(path, variable)

    def _save_number(self, spec: SettingSpec, variable: tk.StringVar) -> None:
        try:
            numeric = float(variable.get().replace(",", "."))
        except ValueError:
            variable.set(str(self.settings.get(spec.path, spec.minimum)))
            return
        numeric = max(spec.minimum, min(spec.maximum, numeric))
        value: int | float = int(round(numeric)) if spec.kind == "int" else round(numeric, 2)
        variable.set(str(value))
        self.settings.set(spec.path, value)

    def _save_number_event(
        self,
        spec: SettingSpec,
        variable: tk.StringVar,
        _event: tk.Event[tk.Misc],
    ) -> None:
        self._save_number(spec, variable)

    def show_page(self, page_name: str) -> None:
        page = self._pages.get(page_name)
        if page is None:
            return
        page.tkraise()
        self._active_page = page_name
        self._refresh_navigation_style()
        if page_name == "history":
            self._refresh_history()
        elif page_name == "exclusions" and not self._catalog_items:
            self._load_catalog()
        elif page_name == "maintenance":
            self._refresh_learning()
        elif page_name == "about":
            self._refresh_diagnostics()

    def _refresh_navigation_style(self) -> None:
        for name, button in self._navigation.items():
            selected = name == self._active_page
            button.configure(
                background=(
                    self._navigation_accent
                    if selected
                    else self._sidebar_background
                ),
                activebackground=(
                    self._navigation_accent
                    if selected
                    else self._navigation_hover
                ),
            )

    def present(self) -> None:
        self.root.deiconify()
        self.root.lift()
        self.root.focus_force()

    def _window_close(self) -> None:
        if bool(self.settings.get("general.close_to_tray", True)) and self.tray is not None:
            self.root.withdraw()
            return
        self.shutdown()

    def _post(self, action: Callable[[], None]) -> None:
        if not self._closing:
            self._events.put(action)

    def _drain_events(self) -> None:
        if self._closing:
            return
        while True:
            try:
                action = self._events.get_nowait()
            except queue.Empty:
                break
            try:
                action()
            except Exception:
                LOGGER.exception("Ошибка действия Windows UI")
        self.root.after(50, self._drain_events)

    def _setting_from_thread(self, path: str, value: object) -> None:
        self._post(partial(self._apply_setting, path, value))

    def _apply_setting(self, path: str, value: object) -> None:
        if path == "*":
            self._refresh_setting_controls()
            return
        boolean = self._boolean_variables.get(path)
        if boolean is not None:
            boolean.set(bool(value))
        string = self._string_variables.get(path)
        if string is not None:
            labels = self._choice_labels.get(path, {})
            string.set(labels.get(str(value), str(value)))
        if path == "appearance.theme":
            self._apply_theme(str(value))
        elif path in {"general.autostart", "general.start_hidden"}:
            self._sync_autostart()
        elif path == "appearance.show_indicator":
            self._sync_tray()
        elif path == "appearance.indicator_style" and self.tray is not None:
            self.tray.set_indicator_style(value)
        elif path == "enabled" and self.tray is not None:
            self.tray.set_enabled(bool(value))
        elif path == "general.sound" and self.tray is not None:
            self.tray.set_sound_enabled(bool(value))
        elif path == "general.notifications" and self.tray is not None:
            self.tray.set_notifications_enabled(bool(value))
        elif path == "history.limit":
            if isinstance(value, (int, float, str)):
                self.history.limit = max(1, int(value))
        elif path in {"exclusions.applications", "exclusions.words"}:
            self._refresh_exclusion_lists()
        elif path == "updates.check_automatically":
            self._sync_update_schedule(initial=True)

    def _refresh_setting_controls(self) -> None:
        for path, boolean_variable in self._boolean_variables.items():
            boolean_variable.set(bool(self.settings.get(path, False)))
        for path, string_variable in self._string_variables.items():
            value = self.settings.get(path, "")
            labels = self._choice_labels.get(path, {})
            string_variable.set(labels.get(str(value), str(value)))
        self.history.limit = max(1, int(self.settings.get("history.limit", 200)))
        self._apply_theme(str(self.settings.get("appearance.theme", "system")))
        self._refresh_exclusion_lists()
        self._sync_autostart()
        self._sync_tray()
        if self.tray is not None:
            self.tray.set_enabled(bool(self.settings.get("enabled", True)))
            self.tray.set_sound_enabled(
                bool(self.settings.get("general.sound", False))
            )
            self.tray.set_notifications_enabled(
                bool(self.settings.get("general.notifications", True))
            )
            self.tray.set_indicator_style(
                self.settings.get("appearance.indicator_style", "letters")
            )
        self._sync_update_schedule(initial=True)

    def _snapshot_from_thread(self, snapshot: EngineSnapshot) -> None:
        self._post(partial(self._apply_snapshot, snapshot))

    def _apply_snapshot(self, snapshot: EngineSnapshot) -> None:
        self.status_text.set(
            "Автокоррекция включена"
            if snapshot.enabled and snapshot.running
            else "Автокоррекция на паузе"
            if not snapshot.enabled
            else "Движок остановлен"
        )
        self.layout_text.set(f"Раскладка: {layout_label(snapshot.current_group)}")
        self.count_text.set(f"Исправлений: {snapshot.correction_count}")
        self.last_action_text.set(snapshot.last_action)
        self.error_text.set(snapshot.last_error)
        if self.tray is not None:
            self.tray.set_layout(snapshot.current_group)
            self.tray.set_enabled(snapshot.enabled)

    def _correction_from_thread(self, plan: CorrectionPlan) -> None:
        self._post(partial(self._announce_correction, plan))

    def _announce_correction(self, plan: CorrectionPlan) -> None:
        if bool(self.settings.get("general.notifications", True)) and self.tray is not None:
            self.tray.notify("Раскладка исправлена", f"{plan.original}  →  {plan.replacement}")
        if bool(self.settings.get("general.sound", False)):
            winsound.MessageBeep(winsound.MB_OK)

    def _learning_prompt_from_thread(
        self, prompt: LearningPrompt | None
    ) -> None:
        anchor = self.backend.input_anchor() if prompt is not None else None
        if prompt is None:
            self._post(partial(self._apply_learning_prompt, prompt, anchor))
            return
        self._post(partial(self._schedule_learning_prompt, prompt, anchor))

    def _schedule_learning_prompt(
        self,
        prompt: LearningPrompt,
        anchor: ScreenAnchor | None,
    ) -> None:
        self.root.after(
            WINDOWS_LEARNING_PROMPT_DELAY_MS,
            self._apply_learning_prompt,
            prompt,
            anchor,
        )

    def _apply_learning_prompt(
        self,
        prompt: LearningPrompt | None,
        anchor: ScreenAnchor | None,
    ) -> None:
        if prompt is None:
            self.learning_prompt.hide_prompt()
            return
        if self.engine.learning_prompt != prompt:
            return
        self.learning_prompt.show_prompt(prompt, anchor)

    def _sync_autostart(self) -> None:
        try:
            self.autostart.set_enabled(
                bool(self.settings.get("general.autostart", True)),
                start_hidden=bool(self.settings.get("general.start_hidden", True)),
            )
        except OSError as error:
            LOGGER.warning("Не удалось синхронизировать автозагрузку Windows: %s", error)
            self.error_text.set(f"Автозагрузка: {error}")

    def _sync_update_schedule(self, *, initial: bool) -> None:
        requested = (
            self.enable_updates
            and not self._closing
            and bool(self.settings.get("updates.check_automatically", True))
        )
        if self._update_after_id is not None:
            try:
                self.root.after_cancel(self._update_after_id)
            except tk.TclError:
                pass
            self._update_after_id = None
        if requested:
            delay = (
                WINDOWS_UPDATE_INITIAL_DELAY_MS
                if initial
                else WINDOWS_UPDATE_INTERVAL_MS
            )
            self._update_after_id = self.root.after(
                delay,
                self._automatic_update_check,
            )

    def _automatic_update_check(self) -> None:
        self._update_after_id = None
        self.updates.check(
            automatic=True,
            install_automatically=bool(
                self.settings.get("updates.install_automatically", True)
            ),
        )
        self._sync_update_schedule(initial=False)

    def _tray_action(self, action: Callable[[], None]) -> None:
        self._post(action)

    def _sync_tray(self) -> None:
        requested = bool(self.settings.get("appearance.show_indicator", True))
        if requested and self.tray is None:
            try:
                actions = WindowsTrayActions(
                    lambda: self._tray_action(self.present),
                    lambda: self._tray_action(self._toggle_engine),
                    lambda: self._tray_action(self._toggle_sound),
                    lambda: self._tray_action(self._toggle_notifications),
                    lambda: self._tray_action(partial(self._show_page_and_present, "history")),
                    lambda: self._tray_action(partial(self._show_page_and_present, "exclusions")),
                    lambda: self._tray_action(partial(self._show_page_and_present, "about")),
                    lambda: self._tray_action(self.shutdown),
                )
                self.tray = WindowsTray(actions)
                self.tray.set_layout(self.engine.snapshot.current_group)
                self.tray.set_enabled(self.engine.snapshot.enabled)
                self.tray.set_sound_enabled(bool(self.settings.get("general.sound", False)))
                self.tray.set_notifications_enabled(
                    bool(self.settings.get("general.notifications", True))
                )
                self.tray.set_indicator_style(
                    self.settings.get("appearance.indicator_style", "letters")
                )
            except Exception as error:
                LOGGER.exception("Не удалось создать системный индикатор")
                self.error_text.set(f"Системный индикатор недоступен: {error}")
                self.tray = None
        elif not requested and self.tray is not None:
            self.tray.close()
            self.tray = None
            if self.root.state() == "withdrawn":
                self.present()

    def _show_page_and_present(self, page_name: str) -> None:
        self.show_page(page_name)
        self.present()

    def _toggle_engine(self) -> None:
        self.settings.set("enabled", not bool(self.settings.get("enabled", True)))

    def _toggle_sound(self) -> None:
        self.settings.set(
            "general.sound",
            not bool(self.settings.get("general.sound", False)),
        )

    def _toggle_notifications(self) -> None:
        self.settings.set(
            "general.notifications",
            not bool(self.settings.get("general.notifications", True)),
        )

    def _string_list(self, path: str) -> list[str]:
        value = self.settings.get(path)
        if not isinstance(value, list):
            return []
        return [str(item) for item in value if str(item).strip()]

    @staticmethod
    def _fill_listbox(widget: tk.Listbox, values: Iterable[str]) -> None:
        widget.delete(0, tk.END)
        for value in values:
            widget.insert(tk.END, value)

    def _refresh_exclusion_lists(self) -> None:
        self._fill_listbox(
            self.application_list,
            sorted(self._string_list("exclusions.applications"), key=str.casefold),
        )
        self._fill_listbox(
            self.word_list,
            sorted(self._string_list("exclusions.words"), key=str.casefold),
        )

    def _add_application(self, identifier: str) -> None:
        normalized = identifier.strip().casefold()
        if not normalized:
            return
        applications = self._string_list("exclusions.applications")
        if normalized not in {item.casefold() for item in applications}:
            applications.append(normalized)
            self.settings.set("exclusions.applications", applications)

    def _add_active_application(self) -> None:
        messagebox.showinfo(
            "Выбор окна",
            "После закрытия подсказки переключитесь на нужное приложение. KeySwitch определит его через 3 секунды.",
            parent=self.root,
        )
        self.status_text.set("Выберите приложение-исключение…")
        self.root.withdraw()
        self.root.after(3000, self._capture_active_application)

    def _capture_active_application(self) -> None:
        identifier = self.backend.active_application()
        if identifier:
            self._add_application(identifier)
            self.status_text.set(f"Добавлено исключение: {identifier}")
        else:
            messagebox.showinfo(
                "KeySwitch",
                "Не удалось определить выбранное приложение.",
                parent=self.root,
            )
        self.show_page("exclusions")
        self.present()

    def _pick_executable(self) -> None:
        selected = filedialog.askopenfilename(
            parent=self.root,
            title="Выберите приложение-исключение",
            filetypes=(("Приложения Windows", "*.exe"), ("Все файлы", "*.*")),
        )
        application = self.catalog.from_executable(selected)
        if application is not None:
            self._add_application(application.identifier)

    def _load_catalog(self) -> None:
        try:
            applications = self.catalog.installed()
        except OSError as error:
            self.error_text.set(f"Каталог приложений: {error}")
            return
        self._catalog_items = {
            f"{application.name} — {application.identifier}": application
            for application in applications
        }
        labels = tuple(self._catalog_items)
        self.catalog_combo.configure(values=labels)
        self.catalog_selection.set(labels[0] if labels else "")

    def _add_catalog_application(self) -> None:
        selected = self._catalog_items.get(self.catalog_selection.get())
        if selected is not None:
            self._add_application(selected.identifier)

    def _add_manual_application(self) -> None:
        application = self.catalog.from_executable(self.manual_application.get())
        if application is not None:
            self._add_application(application.identifier)
            self.manual_application.set("")

    def _remove_application(self) -> None:
        selected: tuple[int, ...] = self.application_list.curselection()  # type: ignore[no-untyped-call]
        if not selected:
            return
        value = str(self.application_list.get(selected[0]))
        applications = [
            item
            for item in self._string_list("exclusions.applications")
            if item.casefold() != value.casefold()
        ]
        self.settings.set("exclusions.applications", applications)

    def _add_word(self) -> None:
        word = self.manual_word.get().strip()
        if not word:
            return
        words = self._string_list("exclusions.words")
        if word.casefold() not in {item.casefold() for item in words}:
            words.append(word)
            self.settings.set("exclusions.words", words)
        self.manual_word.set("")

    def _remove_word(self) -> None:
        selected: tuple[int, ...] = self.word_list.curselection()  # type: ignore[no-untyped-call]
        if not selected:
            return
        value = str(self.word_list.get(selected[0]))
        words = [
            item
            for item in self._string_list("exclusions.words")
            if item.casefold() != value.casefold()
        ]
        self.settings.set("exclusions.words", words)

    def _refresh_history(self) -> None:
        for item in self.history_tree.get_children():
            self.history_tree.delete(item)
        for entry in reversed(self.history.read()):
            self._insert_history(entry)

    def _insert_history(self, entry: HistoryEntry) -> None:
        self.history_tree.insert(
            "",
            tk.END,
            values=(
                entry.timestamp,
                entry.original,
                entry.replacement,
                entry.application,
                f"{entry.confidence:.2f}",
            ),
        )

    def _clear_history(self) -> None:
        if messagebox.askyesno(
            "Очистить историю",
            "Удалить все сохранённые пары исправлений?",
            parent=self.root,
        ):
            self.history.clear()

    def _refresh_learning(self) -> None:
        rules, rejections = self.engine.learning.counts()
        self.learning_text.set(
            f"Выученных правил: {rules} · запретов после отмены: {rejections}"
        )

    def _clear_learning(self) -> None:
        if messagebox.askyesno(
            "Очистить самообучение",
            "Удалить все выученные правила и сохранённые запреты?",
            parent=self.root,
        ):
            self.engine.learning.clear()
            self._refresh_learning()

    def _reset_settings(self) -> None:
        if messagebox.askyesno(
            "Сбросить настройки",
            "Вернуть все параметры KeySwitch по умолчанию?",
            parent=self.root,
        ):
            self.settings.reset()

    def _diagnostics(self) -> str:
        probe = self.backend.probe()
        payload = {
            "keyswitch": __version__,
            "available": probe.available,
            "session_type": probe.session_type,
            "desktop": probe.display,
            "keyboard_hook": probe.record_version,
            "injection": probe.xtest_version,
            "layouts": probe.xkb_version,
            "current_group": probe.current_group,
            "current_layout": layout_label(probe.current_group),
            "settings": str(self.settings.path),
            "data": str(data_dir()),
            "error": probe.error,
        }
        return json.dumps(payload, ensure_ascii=False, indent=2)

    def _refresh_diagnostics(self) -> None:
        diagnostics = self._diagnostics()
        self.diagnostics_text.configure(state="normal")
        self.diagnostics_text.delete("1.0", tk.END)
        self.diagnostics_text.insert("1.0", diagnostics)
        self.diagnostics_text.configure(state="disabled")

    def _copy_diagnostics(self) -> None:
        self.root.clipboard_clear()
        self.root.clipboard_append(self._diagnostics())

    def _apply_theme(self, theme: str) -> None:
        style = ttk.Style(self.root)
        available = style.theme_names()
        style.theme_use("vista" if "vista" in available else "clam")
        dark = theme == "dark"
        background = "#181a1f" if dark else "#f4f6f9"
        content = "#20232a" if dark else "#ffffff"
        sidebar = "#111318" if dark else "#18233a"
        foreground = "#f4f6fb" if dark else "#182033"
        muted = "#aeb5c4" if dark else "#667085"
        accent = "#4f7cff"
        self._sidebar_background = sidebar
        self._navigation_accent = accent
        self._navigation_hover = "#293957"
        self.root.configure(background=background)
        style.configure("App.TFrame", background=background)
        style.configure("Content.TFrame", background=content)
        style.configure("Sidebar.TFrame", background=sidebar)
        style.configure("TLabel", background=content, foreground=foreground)
        style.configure("PageTitle.TLabel", background=content, foreground=foreground, font=("Segoe UI Semibold", 22))
        style.configure("PageSubtitle.TLabel", background=content, foreground=muted)
        style.configure("CardTitle.TLabel", foreground=foreground, font=("Segoe UI Semibold", 10))
        style.configure("Muted.TLabel", foreground=muted)
        style.configure("Error.TLabel", foreground="#e5484d")
        style.configure("Brand.TLabel", background=sidebar, foreground="white", font=("Segoe UI Semibold", 20))
        style.configure("Sidebar.TLabel", background=sidebar, foreground="white")
        style.configure("MutedSidebar.TLabel", background=sidebar, foreground="#aebbd1")
        self._refresh_navigation_style()
        style.configure("TLabelframe", background=content, foreground=foreground)
        style.configure("TLabelframe.Label", background=content, foreground=foreground, font=("Segoe UI Semibold", 11))

    def run(self) -> int:
        self._sync_autostart()
        self._sync_tray()
        self._sync_update_schedule(initial=True)
        engine_error = ""
        if not self.no_engine:
            try:
                self.engine.start()
            except Exception as error:
                engine_error = str(error)
                LOGGER.exception("Не удалось запустить Windows backend")
                self.error_text.set(engine_error)
        self.root.after(25, self._drain_events)
        if self.hidden and self.tray is not None and not engine_error:
            self.root.withdraw()
        else:
            self.present()
        self.root.mainloop()
        return 0

    def shutdown(self) -> None:
        if self._closing:
            return
        self._closing = True
        self._sync_update_schedule(initial=False)
        self.updates.close()
        try:
            self.engine.stop()
        except Exception:
            LOGGER.exception("Ошибка остановки Windows backend")
        if self.tray is not None:
            self.tray.close()
            self.tray = None
        self.learning_prompt.destroy()
        self.root.destroy()


def run_windows_application(
    *,
    hidden: bool,
    no_engine: bool,
    quit_after_ms: int | None = None,
) -> int:
    application = WindowsApplication(
        hidden=hidden,
        no_engine=no_engine,
        enable_updates=quit_after_ms is None,
    )
    if quit_after_ms is not None:
        application.root.after(quit_after_ms, application.shutdown)
    return application.run()

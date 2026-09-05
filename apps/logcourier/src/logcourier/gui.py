from __future__ import annotations

import copy
import os
import sys
from datetime import datetime
from pathlib import Path

from PySide6.QtCore import QLockFile, QObject, Qt, QThread, QTimer, Signal
from PySide6.QtGui import QAction, QColor, QIcon, QPainter, QPixmap
from PySide6.QtWidgets import (
    QApplication,
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMainWindow,
    QMenu,
    QMessageBox,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSystemTrayIcon,
    QTableWidget,
    QTableWidgetItem,
    QTabWidget,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

from . import __version__
from .catalog import current_catalog, verify_connection
from .config import Config, Source, data_directory, load_config, save_config
from .secrets import read_token, redact, store_token, token_bot_id
from .service import Service
from .store import Store
from .telegram import Telegram


def application_icon() -> QIcon:
    pixmap = QPixmap(64, 64)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setBrush(QColor("#2563eb"))
    painter.setPen(Qt.PenStyle.NoPen)
    painter.drawRoundedRect(2, 2, 60, 60, 14, 14)
    painter.setPen(QColor("white"))
    font = painter.font()
    font.setPixelSize(29)
    font.setBold(True)
    painter.setFont(font)
    painter.drawText(pixmap.rect(), Qt.AlignmentFlag.AlignCenter, "LC")
    painter.end()
    return QIcon(pixmap)


class Signals(QObject):
    status = Signal(str, dict)


class Task(QThread):
    result = Signal(object)
    error = Signal(str)

    def __init__(self, action, parent=None):
        super().__init__(parent)
        self.action = action

    def run(self):
        try:
            self.result.emit(self.action())
        except Exception as error:
            if isinstance(error, (ValueError, RuntimeError)):
                self.error.emit(redact(str(error)))
            else:
                self.error.emit(f"Операция не выполнена ({type(error).__name__}).")


class SourceDialog(QDialog):
    def __init__(self, path: str, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Добавить источник")
        layout = QFormLayout(self)
        self.path = QLineEdit(path)
        self.label = QLineEdit(Path(path).name)
        self.rotations = QSpinBox()
        self.rotations.setRange(0, 20)
        self.rotations.setValue(5)
        self.existing = QCheckBox("Включить уже существующие записи и ротации")
        note = QLabel(
            "Без этого флажка сбор начнётся с конца файлов при первом запуске сбора.\n"
            "Ротации: имя.log.1 … имя.log.N; архивы .gz не читаются."
        )
        note.setWordWrap(True)
        layout.addRow("Файл", self.path)
        layout.addRow("Название", self.label)
        layout.addRow("Ротаций", self.rotations)
        layout.addRow(self.existing)
        layout.addRow(note)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addRow(buttons)

    def source(self) -> Source:
        result = Source(
            self.path.text(), self.label.text(), self.rotations.value(), self.existing.isChecked()
        )
        result.validate()
        return result


class Window(QMainWindow):
    def __init__(self, root: Path, config: Config, start_service: bool = True):
        super().__init__()
        self.root, self.config = root, config
        self.sources = copy.deepcopy(config.sources)
        self.tasks: list[Task] = []
        self.exiting = False
        self.last_message = ""
        self.setWindowTitle(f"LogCourier {__version__} — сборщик логов")
        self.setWindowIcon(application_icon())
        self.resize(850, 640)
        self.setMinimumSize(540, 420)
        self.tabs = QTabWidget()
        self.setCentralWidget(self.tabs)
        self.make_connection_page()
        self.make_sources_page()
        self.make_delivery_page()
        self.make_status_page()
        self.statusBar().showMessage("Отправка только после вашего разрешения")
        token = os.environ.get("LOGCOURIER_BOT_TOKEN", "")
        if not token and config.bot_id:
            try:
                token = read_token(config.bot_id)
            except RuntimeError as error:
                self.events.append(str(error))
        self.token.setText(token)
        self.chat.textChanged.connect(lambda: self.consent.setChecked(False))
        self.token.textEdited.connect(lambda: self.consent.setChecked(False))
        self.signals = Signals()
        self.signals.status.connect(self.on_status)
        self.service = Service(root, config, token, self.signals.status.emit)
        self.tray = QSystemTrayIcon(self.windowIcon(), self)
        self.tray.setToolTip("LogCourier — личный сборщик логов")
        menu = QMenu(self)
        for title, callback in (
            ("Открыть настройки", self.reveal),
            ("Отправить сейчас", self.send_now),
            ("Приостановить автосбор", self.pause),
            ("Выход", self.shutdown),
        ):
            action = QAction(title, menu)
            action.triggered.connect(callback)
            menu.addAction(action)
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(self.activated)
        self.tray.show()
        self.exit_timer = QTimer(self)
        self.exit_timer.timeout.connect(self.finish_shutdown)
        if start_service:
            self.service.start()

    def page(self, title: str):
        widget = QWidget()
        layout = QVBoxLayout(widget)
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setWidget(widget)
        self.tabs.addTab(scroll, title)
        return layout

    def make_connection_page(self):
        layout = self.page("Telegram")
        intro = QLabel(
            "1. Создайте отдельного бота через @BotFather.\n"
            "2. Создайте закрытую группу и добавьте бота администратором\n"
            "   с правом закреплять сообщения.\n"
            "3. Введите токен здесь, найдите группу и проверьте подключение.\n\n"
            "Одна группа — один сборщик. Не закрепляйте в ней другие сообщения.\n"
            "Токен не нужно присылать в чат, записывать в файл или включать в установщик."
        )
        intro.setWordWrap(True)
        layout.addWidget(intro)
        form = QFormLayout()
        self.token = QLineEdit()
        self.token.setEchoMode(QLineEdit.EchoMode.Password)
        self.token.setPlaceholderText("Токен от BotFather")
        self.persist_token = QCheckBox("Сохранить токен в системном хранилище секретов")
        self.persist_token.setChecked(True)
        self.chat = QLineEdit(self.config.chat_id)
        self.chat.setPlaceholderText("Например, -1001234567890")
        self.device = QLineEdit(self.config.device_name)
        form.addRow("Токен", self.token)
        form.addRow(self.persist_token)
        form.addRow("ID группы (Chat ID)", self.chat)
        form.addRow("Имя устройства", self.device)
        layout.addLayout(form)
        buttons = QHBoxLayout()
        for title, callback in (
            ("Найти группы", self.find_groups),
            ("Проверить подключение", self.check_connection),
            ("Сохранить", self.save),
        ):
            button = QPushButton(title)
            button.clicked.connect(callback)
            buttons.addWidget(button)
        layout.addLayout(buttons)
        adopt = QPushButton("Подключить существующий каталог (перенос с другого компьютера)")
        adopt.clicked.connect(self.adopt_catalog)
        layout.addWidget(adopt)
        layout.addStretch()

    def make_sources_page(self):
        layout = self.page("Файлы")
        note = QLabel(
            "Читаются только выбранные файлы и их числовые ротации.\n"
            "Исходные файлы не изменяются и не удаляются. Удаление источника\n"
            "останавливает новый сбор, но уже собранные архивы остаются в очереди."
        )
        note.setWordWrap(True)
        layout.addWidget(note)
        self.table = QTableWidget(0, 4)
        self.table.setHorizontalHeaderLabels(["Название", "Файл", "Ротаций", "Старые записи"])
        self.table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.table.horizontalHeader().setStretchLastSection(True)
        layout.addWidget(self.table)
        buttons = QHBoxLayout()
        for title, callback in (
            ("Добавить файл", self.add_source),
            ("Удалить из списка", self.remove_source),
            ("Сохранить", self.save),
        ):
            button = QPushButton(title)
            button.clicked.connect(callback)
            buttons.addWidget(button)
        layout.addLayout(buttons)
        self.refresh_sources()

    def make_delivery_page(self):
        layout = self.page("Отправка")
        self.consent = QCheckBox(
            "Разрешаю отправлять содержимое выбранных логов в эту группу Telegram"
        )
        self.consent.setChecked(self.config.consent)
        self.auto = QCheckBox("Автоматически собирать и отправлять логи")
        self.auto.setChecked(self.config.auto_send)
        self.interval = QSpinBox()
        self.interval.setRange(1, 1440)
        self.interval.setValue(self.config.interval_minutes)
        self.interval.setSuffix(" мин")
        note = QLabel(
            "Логи могут содержать переписку, имена, пути и другие личные данные.\n"
            "Это передача полного содержимого, не обезличенная телеметрия.\n\n"
            "Сбор каждые 5 секунд; отправка по выбранному интервалу.\n"
            "Очередь на диске: до 128 МиБ. Фрагмент: до 2 МиБ до сжатия.\n"
            "При заполнении очереди сбор остановится. Ротация исходной программы\n"
            "может удалить ещё не прочитанные записи — увеличьте её срок хранения.\n\n"
            "При неопределённом ответе сети возможен повторный файл с тем же ID.\n"
            "Каталог и скачивание исключают повторы по идентификатору архива.\n"
            "Файлы в Telegram автоматически не удаляются."
        )
        note.setWordWrap(True)
        layout.addWidget(self.consent)
        layout.addWidget(self.auto)
        form = QFormLayout()
        form.addRow("Интервал отправки", self.interval)
        layout.addLayout(form)
        layout.addWidget(note)
        buttons = QHBoxLayout()
        for title, callback in (
            ("Сохранить", self.save),
            ("Отправить сейчас", self.send_now),
            ("Пауза", self.pause),
        ):
            button = QPushButton(title)
            button.clicked.connect(callback)
            buttons.addWidget(button)
        layout.addLayout(buttons)
        layout.addStretch()

    def make_status_page(self):
        layout = self.page("Состояние")
        self.summary = QLabel("Нет отправленных архивов.")
        self.summary.setWordWrap(True)
        layout.addWidget(self.summary)
        self.events = QTextEdit()
        self.events.setReadOnly(True)
        self.events.document().setMaximumBlockCount(150)
        layout.addWidget(self.events)
        info = QLabel(
            f"Локальные настройки и очередь: {self.root}\n"
            "Журнал состояния не содержит токенов и фрагментов исходных логов."
        )
        info.setWordWrap(True)
        layout.addWidget(info)

    def refresh_sources(self):
        self.table.setRowCount(len(self.sources))
        for row, source in enumerate(self.sources):
            for column, value in enumerate(
                (
                    source.label,
                    source.path,
                    source.rotations,
                    "Да" if source.include_existing else "Нет",
                )
            ):
                self.table.setItem(row, column, QTableWidgetItem(str(value)))
        self.table.resizeColumnsToContents()

    def add_source(self):
        path, _ = QFileDialog.getOpenFileName(self, "Выберите лог", "", "Все файлы (*)")
        if not path:
            return
        dialog = SourceDialog(path, self)
        if dialog.exec() == QDialog.DialogCode.Accepted:
            try:
                source = dialog.source()
                if any(
                    os.path.normcase(item.path) == os.path.normcase(source.path)
                    for item in self.sources
                ):
                    raise ValueError("Этот файл уже добавлен.")
                self.sources.append(source)
                self.refresh_sources()
            except ValueError as error:
                self.error(str(error))

    def remove_source(self):
        rows = sorted({index.row() for index in self.table.selectedIndexes()}, reverse=True)
        for row in rows:
            self.sources.pop(row)
        self.refresh_sources()

    def draft(self) -> tuple[Config, str]:
        token = self.token.text().strip()
        config = copy.deepcopy(self.config)
        config.bot_id = token_bot_id(token) if token else config.bot_id
        config.chat_id = self.chat.text().strip()
        config.device_name = self.device.text().strip()
        config.sources = copy.deepcopy(self.sources)
        config.consent = self.consent.isChecked()
        config.auto_send = self.auto.isChecked()
        config.interval_minutes = self.interval.value()
        config.validate()
        if config.auto_send and (not config.consent or not token or not config.chat_id):
            raise ValueError(
                "Для автоотправки нужны токен, группа и разрешение на передачу текста."
            )
        return config, token

    def save(self) -> bool:
        try:
            config, token = self.draft()
            if token and self.persist_token.isChecked():
                store_token(token)
            save_config(self.root, config)
            self.config = config
            self.service.update(config, token)
            self.statusBar().showMessage("Настройки сохранены.", 5000)
            return True
        except (ValueError, RuntimeError, OSError) as error:
            self.error(
                redact(str(error))
                if not isinstance(error, OSError)
                else "Не удалось сохранить настройки."
            )
            return False

    def pause(self):
        self.auto.setChecked(False)
        # Pausing never depends on availability of the system keyring.
        self.config.auto_send = False
        self.service.update(self.config, self.token.text().strip())
        try:
            save_config(self.root, self.config)
        except OSError:
            self.error("Сбор приостановлен, но настройка не сохранена на диск.")
        self.statusBar().showMessage(
            "Автосбор приостановлен. Текущий сетевой запрос может завершиться."
        )

    def send_now(self):
        if not self.consent.isChecked():
            self.error("Сначала разрешите передачу текста логов на вкладке «Отправка».")
            return
        if self.save():
            self.service.send_now()

    def task(self, action, callback):
        worker = Task(action, self)
        self.tasks.append(worker)
        worker.result.connect(callback)
        worker.error.connect(self.error)
        worker.finished.connect(lambda: self.tasks.remove(worker))
        worker.start()

    def find_groups(self):
        try:
            client = Telegram(self.token.text().strip())
        except ValueError as error:
            self.error(str(error))
            return
        self.task(client.groups, self.choose_group)

    def choose_group(self, groups):
        if not groups:
            self.error(
                "Группы не найдены. Добавьте бота и отправьте /start в группе, затем повторите. "
                "Либо введите Chat ID вручную."
            )
            return
        from PySide6.QtWidgets import QInputDialog

        options = [f"{item.get('title', 'Группа')} ({item['id']})" for item in groups]
        choice, accepted = QInputDialog.getItem(
            self, "Группа Telegram", "Выберите группу", options, editable=False
        )
        if accepted:
            self.chat.setText(str(groups[options.index(choice)]["id"]))

    def check_connection(self):
        try:
            client = Telegram(self.token.text().strip())
            chat = self.chat.text().strip()
            if not chat:
                raise ValueError("Сначала выберите группу.")
        except ValueError as error:
            self.error(str(error))
            return
        self.task(
            lambda: verify_connection(client, chat),
            lambda result: QMessageBox.information(self, "Подключение проверено", str(result)),
        )

    def adopt_catalog(self):
        try:
            client = Telegram(self.token.text().strip())
            chat = self.chat.text().strip()
            if not chat:
                raise ValueError("Укажите ID группы.")
        except ValueError as error:
            self.error(str(error))
            return

        def inspect():
            verify_connection(client, chat)
            return current_catalog(client, chat)

        self.task(inspect, lambda head: self.confirm_catalog(head, client.bot_id, chat))

    def confirm_catalog(self, head, bot_id, chat):
        try:
            current_bot = token_bot_id(self.token.text().strip())
        except ValueError:
            current_bot = ""
        if self.chat.text().strip() != chat or current_bot != bot_id:
            self.error("Настройки подключения изменились. Повторите проверку.")
            return
        if not head:
            self.error("В группе нет закреплённого каталога LogCourier.")
            return
        if head["index"]["device_id"] == self.config.device_id:
            QMessageBox.information(
                self, "LogCourier", "Этот каталог уже относится к вашему профилю."
            )
            return
        choice = QMessageBox.question(
            self,
            "Перенос каталога",
            "Сначала остановите LogCourier на прежнем компьютере.\n"
            "Продолжить его каталог на этом компьютере?\n"
            "Старые логи не скачиваются. Автоотправку потребуется включить отдельно.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        if choice != QMessageBox.StandardButton.Yes:
            return
        self.pause()
        try:
            store = Store(self.root)
            try:
                stats = store.stats(f"{bot_id}:{chat}")
                if stats["queued"] or stats["unindexed"]:
                    raise ValueError("Сначала завершите отправку локальной очереди в эту группу.")
            finally:
                store.close()
            old_id = self.config.device_id
            self.config.device_id = head["index"]["device_id"]
            self.consent.setChecked(False)
            if not self.save():
                self.config.device_id = old_id
        except (ValueError, RuntimeError) as error:
            self.error(str(error))

    def on_status(self, message: str, stats: dict):
        stats = {
            "last_success": None,
            "queued": 0,
            "unindexed": 0,
            "queue_bytes": 0,
            "other": 0,
            **stats,
        }
        last = stats["last_success"]
        stamp = (
            datetime.fromtimestamp(last).strftime("%d.%m.%Y %H:%M:%S") if last else "ещё не было"
        )
        self.summary.setText(
            f"{message}\nВ очереди: {stats['queued']}; ждут каталога: {stats['unindexed']}.\n"
            f"Объём очереди: {stats['queue_bytes'] / 1024 / 1024:.1f} МиБ.\n"
            f"Последняя успешная отправка: {stamp}.\n"
            f"Архивов для других ботов/групп: {stats['other']} (не отправляются сюда)."
        )
        if message != self.last_message:
            self.events.moveCursor(self.events.textCursor().MoveOperation.End)
            self.events.insertPlainText(f"{datetime.now():%H:%M:%S} {redact(message)}\n")
            self.last_message = message
        self.tray.setToolTip(("LogCourier — " + message)[:120])

    def error(self, message):
        QMessageBox.warning(self, "LogCourier", redact(str(message)))

    def reveal(self):
        self.showNormal()
        self.raise_()
        self.activateWindow()

    def activated(self, reason):
        if reason in (
            QSystemTrayIcon.ActivationReason.Trigger,
            QSystemTrayIcon.ActivationReason.DoubleClick,
        ):
            self.reveal()

    def closeEvent(self, event):
        if QSystemTrayIcon.isSystemTrayAvailable() and not self.exiting:
            event.ignore()
            self.hide()
        elif not self.exiting:
            event.ignore()
            self.shutdown()
        else:
            event.accept()

    def shutdown(self):
        self.exiting = True
        self.service.stop()
        self.setEnabled(False)
        self.statusBar().showMessage("Завершение текущего запроса…")
        self.exit_timer.start(100)

    def finish_shutdown(self):
        if not self.service.thread.is_alive() and not any(task.isRunning() for task in self.tasks):
            self.exit_timer.stop()
            self.tray.hide()
            QApplication.quit()


def run(minimized: bool = False) -> int:
    app = QApplication(sys.argv[:1])
    app.setApplicationName("LogCourier")
    app.setOrganizationName("LogCourier")
    app.setQuitOnLastWindowClosed(False)
    root = data_directory()
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock = QLockFile(str(root / "instance.lock"))
    lock.setStaleLockTime(0)
    if not lock.tryLock(0):
        QMessageBox.information(
            None, "LogCourier", "Сборщик уже запущен. Откройте его через значок в трее."
        )
        return 0
    try:
        config = load_config(root)
    except Exception:
        QMessageBox.critical(
            None,
            "LogCourier",
            f"Не удалось прочитать {root / 'config.json'}.\n"
            "Файл оставлен без изменений; автоматическая отправка не запущена.",
        )
        return 1
    window = Window(root, config)
    if not minimized or not QSystemTrayIcon.isSystemTrayAvailable() or not config.bot_id:
        window.show()
    return app.exec()

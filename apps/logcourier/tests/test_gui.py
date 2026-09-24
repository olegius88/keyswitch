import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from fixture_values.counts import FAKE_BOT_TOKEN_SECRET_CHARACTERS, FAKE_HEX_ID_LENGTH, TAB_COUNT
from PySide6.QtWidgets import (
    QApplication,
    QLabel,
    QLineEdit,
    QMessageBox,
    QScrollArea,
    QSystemTrayIcon,
)

from logcourier import autostart
from logcourier.config import Config, load_config
from logcourier.constants.files import BYTES_PER_MEBIBYTE, BYTES_PER_MEGABYTE, CHUNK_BYTES
from logcourier.constants.gui import WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH
from logcourier.constants.limits import MAX_QUEUE_BYTES
from logcourier.constants.telegram import GROUP_INTERVAL, MAX_DOWNLOAD
from logcourier.constants.timing import WAKE_POLL_SECONDS
from logcourier.gui import Window
from logcourier.russian_text import SECONDS, quantity, russian_number


def test_gui_safe_defaults_scroll_and_pause(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    window = Window(tmp_path, Config(), start_service=False)
    window.show()
    window.resize(WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT)
    app.processEvents()
    assert window.tabs.count() == TAB_COUNT
    assert all(isinstance(window.tabs.widget(i), QScrollArea) for i in range(TAB_COUNT))
    assert window.token.echoMode() == QLineEdit.EchoMode.Password
    assert not window.auto.isChecked() and not window.consent.isChecked()
    window.consent.setChecked(True)
    window.chat.setText("-100123")
    assert not window.consent.isChecked()
    window.on_status("Тест состояния", {})
    window.pause()
    assert not window.service._config.auto_send
    window.exiting = True
    window.tray.hide()
    window.close()
    app.processEvents()


def test_delivery_note_names_the_limits_the_code_uses(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    QApplication.instance() or QApplication([])
    window = Window(tmp_path, Config(), start_service=False)
    note = next(label.text() for label in window.findChildren(QLabel) if "Очередь:" in label.text())
    for fragment in (
        f"Сбор раз в {quantity(WAKE_POLL_SECONDS, SECONDS)};",
        f"Очередь: до {russian_number(MAX_QUEUE_BYTES / BYTES_PER_MEBIBYTE)} МиБ.",
        f"Локальный фрагмент: до {russian_number(CHUNK_BYTES / BYTES_PER_MEBIBYTE)} МиБ до сжатия.",
        f"в пакеты до {russian_number(MAX_DOWNLOAD / BYTES_PER_MEGABYTE)} МБ.",
        f"не чаще одного раза в {quantity(GROUP_INTERVAL, SECONDS)}.",
    ):
        assert fragment in note
    window.exiting = True
    window.tray.hide()
    window.close()


def test_save_bot_and_group_and_adopt_catalog(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    window = Window(tmp_path, Config(), start_service=False)
    token = "123456:" + "C" * FAKE_BOT_TOKEN_SECRET_CHARACTERS
    window.token.setText(token)
    window.chat.setText("-100123")
    window.persist_token.setChecked(False)
    assert window.save()
    saved = load_config(tmp_path)
    assert saved.bot_id == "123456" and saved.chat_id == "-100123"
    assert token not in (tmp_path / "config.json").read_text()
    monkeypatch.setattr(QMessageBox, "question", lambda *args: QMessageBox.StandardButton.Yes)
    window.confirm_catalog({"index": {"device_id": "a" * FAKE_HEX_ID_LENGTH}}, "123456", "-100123")
    assert load_config(tmp_path).device_id == "a" * FAKE_HEX_ID_LENGTH
    assert not window.consent.isChecked() and not window.auto.isChecked()
    errors = []
    monkeypatch.setattr(window, "error", errors.append)
    window.token.setText("invalid")
    window.confirm_catalog(None, "123456", "-100123")
    assert errors
    window.exiting = True
    window.tray.hide()
    window.close()
    app.processEvents()


def test_close_hides_and_keeps_worker_alive(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    monkeypatch.setattr(QSystemTrayIcon, "isSystemTrayAvailable", lambda: True)
    window = Window(tmp_path, Config(), start_service=False)
    window.show()
    app.processEvents()
    window.close()
    app.processEvents()
    assert not window.isVisible()
    assert not window.exiting and not window.service.stop_event.is_set()
    assert not app.quitOnLastWindowClosed()
    window.reveal()
    assert window.isVisible()
    window.shutdown()
    assert window.exiting and window.service.stop_event.is_set()
    window.exit_timer.stop()
    window.tray.hide()
    window.close()


def test_no_tray_does_not_terminate_or_strand_sender(tmp_path, monkeypatch):
    app = QApplication.instance() or QApplication([])
    monkeypatch.setattr(QSystemTrayIcon, "isSystemTrayAvailable", lambda: False)
    window = Window(tmp_path, Config(), start_service=False)
    window.show()
    window.close()
    app.processEvents()
    assert window.isVisible() and not window.exiting
    assert not window.service.stop_event.is_set()
    window.exiting = True
    window.tray.hide()
    window.close()


def test_autostart_toggle_applies_immediately_and_recovers_on_error(tmp_path, monkeypatch):
    app = QApplication.instance() or QApplication([])
    monkeypatch.setattr(autostart, "is_enabled", lambda: False)
    window = Window(tmp_path, Config(), start_service=False)
    calls = []
    monkeypatch.setattr(autostart, "set_enabled", calls.append)
    window.change_autostart(True)
    assert calls == [True]

    def denied(value):
        raise OSError("denied")

    monkeypatch.setattr(autostart, "set_enabled", denied)
    errors = []
    monkeypatch.setattr(window, "error", errors.append)
    window.autostart_box.setChecked(True)
    window.change_autostart(True)
    assert not window.autostart_box.isChecked() and errors
    window.exiting = True
    window.tray.hide()
    window.close()
    app.processEvents()

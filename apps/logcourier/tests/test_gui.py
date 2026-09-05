import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtWidgets import QApplication, QLineEdit, QMessageBox, QScrollArea

from logcourier.config import Config, load_config
from logcourier.gui import Window


def test_gui_safe_defaults_scroll_and_pause(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    window = Window(tmp_path, Config(), start_service=False)
    window.show()
    window.resize(540, 420)
    app.processEvents()
    assert window.tabs.count() == 4
    assert all(isinstance(window.tabs.widget(i), QScrollArea) for i in range(4))
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


def test_save_bot_and_group_and_adopt_catalog(tmp_path, monkeypatch):
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    window = Window(tmp_path, Config(), start_service=False)
    token = "123456:" + "C" * 30
    window.token.setText(token)
    window.chat.setText("-100123")
    window.persist_token.setChecked(False)
    assert window.save()
    saved = load_config(tmp_path)
    assert saved.bot_id == "123456" and saved.chat_id == "-100123"
    assert token not in (tmp_path / "config.json").read_text()
    monkeypatch.setattr(QMessageBox, "question", lambda *args: QMessageBox.StandardButton.Yes)
    window.confirm_catalog({"index": {"device_id": "a" * 32}}, "123456", "-100123")
    assert load_config(tmp_path).device_id == "a" * 32
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

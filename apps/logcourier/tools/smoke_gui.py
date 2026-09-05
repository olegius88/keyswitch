"""Render a fresh non-sending profile and exercise the real Qt event loop."""

import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QApplication

from logcourier.config import Config
from logcourier.gui import Window

ROOT = Path(__file__).resolve().parents[1] / ".local" / "smoke"
ROOT.mkdir(parents=True, exist_ok=True)
app = QApplication([])
window = Window(ROOT / "profile", Config(), start_service=True)
window.show()


def capture():
    for index, label in enumerate(("telegram", "files", "delivery", "status")):
        window.tabs.setCurrentIndex(index)
        app.processEvents()
        if not window.grab().save(str(ROOT / f"{label}.png")):
            raise RuntimeError("Screenshot failed")
    window.shutdown()


QTimer.singleShot(500, capture)
raise SystemExit(app.exec())

"""Report desktop startup failures even when Windows provides no stderr."""

import sys
import traceback

from .config import data_directory
from .secrets import redact


def report_error(error):
    detail = redact("".join(traceback.format_exception(error)))
    message = f"Не удалось запустить LogCourier: {type(error).__name__}\n{redact(str(error))}"
    try:
        root = data_directory()
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        path = root / "startup-error.txt"
        path.write_text(detail, encoding="utf-8")
        message += f"\n\nПодробности: {path}"
    except OSError:
        message += "\nНе удалось сохранить диагностический файл."
    if sys.platform == "win32":
        import ctypes

        ctypes.windll.user32.MessageBoxW(None, message, "LogCourier — ошибка запуска", 0x10)
    elif sys.stderr is not None:
        print(message, file=sys.stderr)

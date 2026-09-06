"""Opt-in per-user Windows startup. No administrator rights or scheduled tasks."""

import subprocess
import sys
from pathlib import Path

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
VALUE_NAME = "LogCourier"


def command() -> str:
    executable = Path(sys.executable)
    if getattr(sys, "frozen", False):
        args = [str(executable), "gui", "--minimized"]
    else:
        pythonw = executable.with_name("pythonw.exe")
        if not pythonw.is_file():
            raise RuntimeError("Для автозапуска из исходников нужен pythonw.exe.")
        args = [str(pythonw), "-m", "logcourier", "gui", "--minimized"]
    result = subprocess.list2cmdline(args)
    if len(result) > 260:
        raise ValueError(
            "Путь для автозапуска Windows слишком длинный. Переместите программу ближе к корню диска."
        )
    return result


def is_enabled() -> bool:
    if sys.platform != "win32":
        return False
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_READ) as key:
            value, kind = winreg.QueryValueEx(key, VALUE_NAME)
            return kind == winreg.REG_SZ and bool(value)
    except FileNotFoundError:
        return False


def set_enabled(enabled: bool) -> None:
    if sys.platform != "win32":
        raise RuntimeError("Эта настройка доступна только в Windows.")
    import winreg

    if enabled:
        value = command()
        with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
            winreg.SetValueEx(key, VALUE_NAME, 0, winreg.REG_SZ, value)
    else:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
                winreg.DeleteValue(key, VALUE_NAME)
        except FileNotFoundError:
            pass

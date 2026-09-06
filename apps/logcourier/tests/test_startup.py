from types import SimpleNamespace
from unittest.mock import MagicMock

from logcourier import __main__, startup


def test_gui_error_callback_receives_import_details(monkeypatch):
    monkeypatch.setattr(
        __main__, "load_config", MagicMock(side_effect=ImportError("missing module"))
    )
    errors = []
    assert __main__.main(["status"], error_handler=errors.append) == 1
    assert str(errors[0]) == "missing module"


def test_windows_error_dialog_without_stderr(tmp_path, monkeypatch):
    import ctypes

    dialog = MagicMock()
    monkeypatch.setattr(
        ctypes, "windll", SimpleNamespace(user32=SimpleNamespace(MessageBoxW=dialog)), raising=False
    )
    monkeypatch.setattr(startup, "sys", SimpleNamespace(platform="win32", stderr=None))
    monkeypatch.setattr(startup, "data_directory", lambda: tmp_path)
    token = "123456:" + "A" * 30
    startup.report_error(ImportError("missing DLL " + token))
    assert "missing DLL" in (tmp_path / "startup-error.txt").read_text()
    assert token not in (tmp_path / "startup-error.txt").read_text()
    assert token not in str(dialog.call_args)
    dialog.assert_called_once()

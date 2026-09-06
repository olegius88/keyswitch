import sys
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from logcourier import autostart


@pytest.fixture
def registry(monkeypatch):
    values = {"OtherProgram": "keep"}
    handle = MagicMock()
    handle.__enter__.return_value = handle
    module = SimpleNamespace(HKEY_CURRENT_USER=1, KEY_READ=2, KEY_SET_VALUE=3, REG_SZ=1)

    def open_key(hive, key, reserved, access):
        assert hive == 1 and key == autostart.RUN_KEY
        return handle

    def query(key, name):
        if name not in values:
            raise FileNotFoundError
        return values[name], 1

    def delete(key, name):
        if name not in values:
            raise FileNotFoundError
        del values[name]

    module.OpenKey = module.CreateKeyEx = open_key
    module.QueryValueEx = query
    module.SetValueEx = lambda key, name, reserved, kind, value: values.update({name: value})
    module.DeleteValue = delete
    monkeypatch.setitem(sys.modules, "winreg", module)
    monkeypatch.setattr(
        autostart,
        "sys",
        SimpleNamespace(
            platform="win32", frozen=True, executable="C:/Program Files/LogCourier/LogCourier.exe"
        ),
    )
    return module, values


def test_enable_disable_preserves_other_programs(registry):
    _, values = registry
    assert not autostart.is_enabled()
    autostart.set_enabled(True)
    assert autostart.is_enabled()
    executable = Path("C:/Program Files/LogCourier/LogCourier.exe")
    assert values["LogCourier"] == f'"{executable}" gui --minimized'
    autostart.set_enabled(False)
    autostart.set_enabled(False)
    assert values == {"OtherProgram": "keep"}


def test_registry_error_is_not_hidden(registry):
    module, _ = registry
    module.CreateKeyEx = MagicMock(side_effect=PermissionError("Denied"))
    with pytest.raises(PermissionError):
        autostart.set_enabled(True)


def test_long_path_rejected(registry, monkeypatch):
    monkeypatch.setattr(autostart.sys, "executable", "C:/" + "x" * 270 + "/LogCourier.exe")
    with pytest.raises(ValueError):
        autostart.set_enabled(True)


def test_source_uses_pythonw(registry, monkeypatch, tmp_path):
    monkeypatch.setattr(autostart.sys, "frozen", False)
    monkeypatch.setattr(autostart.sys, "executable", str(tmp_path / "python.exe"))
    with pytest.raises(RuntimeError):
        autostart.command()
    (tmp_path / "pythonw.exe").touch()
    assert "pythonw.exe" in autostart.command()
    assert "-m logcourier gui --minimized" in autostart.command()


def test_other_platform_never_writes_registry(monkeypatch):
    monkeypatch.setattr(autostart, "sys", SimpleNamespace(platform="linux"))
    assert not autostart.is_enabled()
    with pytest.raises(RuntimeError):
        autostart.set_enabled(True)

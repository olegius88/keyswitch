import runpy
import sys
from pathlib import Path
from types import SimpleNamespace


def test_windows_spec_has_gui_and_console_executables(monkeypatch):
    root = Path(__file__).resolve().parents[1]
    executables = []

    def exe(*args, **kwargs):
        executables.append(kwargs)
        return kwargs["name"]

    namespace = {
        "SPECPATH": str(root / "packaging"),
        "Analysis": lambda *args, **kwargs: SimpleNamespace(
            pure=[], scripts=[], binaries=[], datas=[]
        ),
        "PYZ": lambda *args: None,
        "EXE": exe,
        "COLLECT": lambda *args, **kwargs: None,
    }
    with monkeypatch.context() as patch:
        patch.setattr(sys, "platform", "win32")
        runpy.run_path(str(root / "packaging/logcourier.spec"), init_globals=namespace)
    assert [(item["name"], item["console"]) for item in executables] == [
        ("LogCourier", False),
        ("LogCourier-cli", True),
    ]

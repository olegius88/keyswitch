import hashlib
import runpy
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture
def builder(monkeypatch):
    module = runpy.run_path(str(ROOT / "tools/build_installer.py"))
    monkeypatch.setattr(module["platform"], "system", lambda: "Windows")
    monkeypatch.setattr(module["shutil"], "which", lambda name: "C:/Inno Setup 6/ISCC.exe")
    return module


@pytest.fixture
def payload(tmp_path):
    root = tmp_path / "Сборка с пробелами"
    source = root / "dist/LogCourier"
    source.mkdir(parents=True)
    for name in ("LogCourier.exe", "LogCourier-cli.exe", "LICENSE", "README.md"):
        (source / name).write_bytes(b"fixture")
    for name in ("_internal", "docs", "third-party-licenses"):
        (source / name).mkdir()
        (source / name / "fixture.md").write_text("fixture")
    (root / "pyproject.toml").write_text('[project]\nversion = "0.1.0"\n')
    return root


def test_build_installer_command_and_checksum(builder, payload, monkeypatch):
    commands = []

    def compile(args, **kwargs):
        commands.append((args, kwargs))
        output = Path(
            next(arg.removeprefix("/DOutputDir=") for arg in args if arg.startswith("/DOutputDir="))
        )
        (output / "LogCourier-Setup-0.1.0-x64.exe").write_bytes(b"new installer")

    monkeypatch.setattr(builder["subprocess"], "run", compile)
    result = builder["build"](payload)
    assert result.name == "LogCourier-Setup-0.1.0-x64.exe"
    assert result.read_bytes() == b"new installer"
    assert result.with_suffix(".exe.sha256").read_text() == (
        f"{hashlib.sha256(b'new installer').hexdigest()}  {result.name}\n"
    )
    args, kwargs = commands[0]
    assert args[0] == str(Path("C:/Inno Setup 6/ISCC.exe"))
    assert f"/DSourceDir={payload / 'dist/LogCourier'}" in args
    assert args[-1] == str(payload / "packaging/windows/LogCourier.iss")
    assert kwargs == {"cwd": payload, "check": True, "timeout": 600}


@pytest.mark.parametrize("missing", ["LogCourier-cli.exe", "_internal/fixture.md"])
def test_incomplete_payload_fails(builder, payload, missing):
    (payload / "dist/LogCourier" / missing).unlink()
    with pytest.raises(RuntimeError, match="Missing payload"):
        builder["build"](payload)


def test_compiler_failure_propagates(builder, payload, monkeypatch):
    def fail(*args, **kwargs):
        raise subprocess.CalledProcessError(1, args[0])

    monkeypatch.setattr(builder["subprocess"], "run", fail)
    with pytest.raises(subprocess.CalledProcessError):
        builder["build"](payload)


def test_stale_output_cannot_pass(builder, payload, monkeypatch):
    (payload / "dist/LogCourier-Setup-0.1.0-x64.exe").write_bytes(b"old")
    monkeypatch.setattr(builder["subprocess"], "run", lambda *args, **kwargs: None)
    with pytest.raises(RuntimeError, match="did not produce"):
        builder["build"](payload)


def test_compiler_discovery_and_missing(builder, tmp_path, monkeypatch):
    monkeypatch.setattr(builder["shutil"], "which", lambda name: None)
    monkeypatch.setenv("ProgramFiles(x86)", str(tmp_path))
    monkeypatch.delenv("ProgramFiles", raising=False)
    with pytest.raises(RuntimeError, match="Inno Setup 6"):
        builder["find_compiler"]()
    compiler = tmp_path / "Inno Setup 6/ISCC.exe"
    compiler.parent.mkdir()
    compiler.touch()
    assert builder["find_compiler"]() == compiler


def test_non_windows_build_fails(builder, payload, monkeypatch):
    monkeypatch.setattr(builder["platform"], "system", lambda: "Linux")
    with pytest.raises(RuntimeError, match="native Windows"):
        builder["build"](payload)


def test_installer_payload_and_user_safety_contract():
    spec = (ROOT / "packaging/windows/LogCourier.iss").read_text(encoding="utf-8")
    assert "PrivilegesRequired=lowest" in spec
    assert "DefaultDirName={localappdata}\\Programs\\LogCourier" in spec
    assert 'Source: "{#SourceDir}\\*"' not in spec
    assert "[UninstallDelete]" not in spec
    assert "[Registry]" not in spec  # Setup never enables autostart.
    assert "skipifsilent" in spec
    assert "CompareText(Command," in spec  # Own executable only on uninstall.
    assert "60EA65E9-FAEF-4FC9-A0D6-8F1DF258C8EC" in spec


def test_installer_lifecycle_rejects_non_ci(monkeypatch):
    module = runpy.run_path(str(ROOT / "tools/verify_installer.py"))
    monkeypatch.delenv("GITHUB_ACTIONS", raising=False)
    with pytest.raises(RuntimeError, match="disposable Windows CI"):
        module["main"]()

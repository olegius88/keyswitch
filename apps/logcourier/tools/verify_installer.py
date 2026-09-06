"""Native CI only: install, run GUI, upgrade, uninstall; preserve personal data."""

import os
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "{60EA65E9-FAEF-4FC9-A0D6-8F1DF258C8EC}_is1"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def main():
    if sys.platform != "win32" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Run installer lifecycle checks only on a disposable Windows CI runner.")
    import winreg

    def read_startup():
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
                return winreg.QueryValueEx(key, "LogCourier")[0]
        except FileNotFoundError:
            return None

    # Never overwrite an existing installation or personal profile.
    uninstall_key = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{APP_ID}"
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key):
            raise RuntimeError("Existing LogCourier installation: refusing lifecycle checks.")
    except FileNotFoundError:
        pass
    profile = Path(os.environ["LOCALAPPDATA"]) / "LogCourier"
    if profile.exists() or read_startup() is not None:
        raise RuntimeError("Existing LogCourier data or autostart: refusing lifecycle checks.")

    version = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))["project"][
        "version"
    ]
    installer = ROOT / "dist" / f"LogCourier-Setup-{version}-x64.exe"
    # Leave logs and test data on the disposable runner, including on failure.
    work = Path(tempfile.mkdtemp(prefix="logcourier-install-", dir=os.environ["RUNNER_TEMP"]))
    installed = work / "Проверка установки с пробелами" / "LogCourier"
    print(f"Installer verification evidence: {work}", flush=True)
    profile.mkdir()
    sentinels = {"config.json": b'{"test":"preserve"}', "queue.sqlite3": b"test queue"}
    for name, data in sentinels.items():
        (profile / name).write_bytes(data)

    def check_profile():
        for name, data in sentinels.items():
            assert (profile / name).read_bytes() == data, f"User data modified: {name}"

    def install(label):
        subprocess.run(
            [
                str(installer),
                "/SP-",
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART",
                "/NORESTARTAPPLICATIONS",
                f"/DIR={installed}",
                f"/LOG={work / (label + '.log')}",
            ],
            check=True,
            timeout=180,
        )
        assert (installed / "LogCourier.exe").is_file()
        check_profile()

    def uninstall(label):
        uninstallers = list(installed.glob("unins*.exe"))
        assert len(uninstallers) == 1, "Expected exactly one uninstaller"
        subprocess.run(
            [
                str(uninstallers[0]),
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART",
                f"/LOG={work / (label + '.log')}",
            ],
            check=True,
            timeout=180,
        )
        for _ in range(50):
            if not (installed / "LogCourier.exe").exists():
                break
            time.sleep(0.2)
        assert not (installed / "LogCourier.exe").exists(), "Uninstall left the GUI executable"
        assert not (installed / "LogCourier-cli.exe").exists(), "Uninstall left the CLI executable"
        check_profile()

    def set_startup(command):
        with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            winreg.SetValueEx(key, "LogCourier", 0, winreg.REG_SZ, command)

    install("install")
    assert read_startup() is None, "Installer unexpectedly enabled autostart"
    result = subprocess.run(
        [str(installed / "LogCourier-cli.exe"), "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.stdout.strip() == version
    subprocess.run(
        [sys.executable, str(ROOT / "tools/verify_frozen.py"), str(installed)],
        check=True,
        timeout=60,
    )
    startup = subprocess.list2cmdline([str(installed / "LogCourier.exe"), "gui", "--minimized"])
    set_startup(startup)
    install("upgrade")
    assert read_startup() == startup, "Upgrade changed the user's autostart setting"
    uninstall("uninstall")
    assert read_startup() is None, "Uninstall left its own autostart entry"

    install("reinstall")
    portable = subprocess.list2cmdline(
        [str(work / "Portable/LogCourier.exe"), "gui", "--minimized"]
    )
    set_startup(portable)
    try:
        uninstall("uninstall-portable-check")
        assert read_startup() == portable, "Uninstall removed another installation's autostart"
    finally:
        if read_startup() == portable:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
                winreg.DeleteValue(key, "LogCourier")
    print("Installed GUI, upgrade, uninstall, autostart ownership and data preservation: passed")


if __name__ == "__main__":
    main()

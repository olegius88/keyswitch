"""Desktop integration helpers."""

from __future__ import annotations

import os
import shlex
import shutil
import sys
from pathlib import Path


APP_ID = "io.github.olegius88.KeySwitch"
APP_NAME = "KeySwitch"


def source_root() -> Path:
    return Path(__file__).resolve().parents[2]


def launcher_command() -> str:
    installed = shutil.which("keyswitch")
    if installed:
        return shlex.quote(installed)
    launcher = source_root() / "run.sh"
    if launcher.is_file():
        return shlex.quote(str(launcher))
    return f"{shlex.quote(sys.executable)} -m keyswitch"


def autostart_path() -> Path:
    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return config_home / "autostart" / f"{APP_ID}.desktop"


class AutostartManager:
    def __init__(self, path: Path | None = None) -> None:
        self.path = path or autostart_path()

    def enabled(self) -> bool:
        if not self.path.is_file():
            return False
        try:
            return "Hidden=true" not in self.path.read_text(encoding="utf-8")
        except OSError:
            return False

    def set_enabled(self, enabled: bool) -> None:
        if enabled:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            self.path.write_text(
                "\n".join(
                    (
                        "[Desktop Entry]",
                        "Type=Application",
                        f"Name={APP_NAME}",
                        "Comment=Автоматическое исправление раскладки клавиатуры",
                        f"Exec={launcher_command()}",
                        "Icon=keyswitch",
                        "Terminal=false",
                        "StartupNotify=false",
                        "X-GNOME-Autostart-enabled=true",
                        "OnlyShowIn=GNOME;XFCE;Unity;MATE;Cinnamon;",
                        "",
                    )
                ),
                encoding="utf-8",
            )
        elif self.path.exists():
            self.path.unlink()

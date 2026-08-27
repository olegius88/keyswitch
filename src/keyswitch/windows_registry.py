"""Native ``winreg`` adapter isolated from platform-independent logic."""

from __future__ import annotations

import winreg


RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
APP_PATHS_KEY = r"Software\Microsoft\Windows\CurrentVersion\App Paths"
UNINSTALL_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"


class NativeWindowsRegistry:
    def read_autostart(self, name: str) -> str | None:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
                value, _value_type = winreg.QueryValueEx(key, name)
        except FileNotFoundError:
            return None
        return value if isinstance(value, str) else None

    def write_autostart(self, name: str, command: str) -> None:
        with winreg.CreateKeyEx(
            winreg.HKEY_CURRENT_USER,
            RUN_KEY,
            0,
            winreg.KEY_SET_VALUE,
        ) as key:
            winreg.SetValueEx(key, name, 0, winreg.REG_SZ, command)

    def delete_autostart(self, name: str) -> None:
        try:
            with winreg.OpenKey(
                winreg.HKEY_CURRENT_USER,
                RUN_KEY,
                0,
                winreg.KEY_SET_VALUE,
            ) as key:
                winreg.DeleteValue(key, name)
        except FileNotFoundError:
            return

    def application_paths(self) -> tuple[tuple[str, str], ...]:
        result: list[tuple[str, str]] = []
        views = (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY)
        roots = (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE)
        for root in roots:
            for view in views:
                result.extend(self._uninstall_paths_for(root, view))
                result.extend(self._application_paths_for(root, view))
        return tuple(result)

    @staticmethod
    def _application_paths_for(
        root: int,
        view: int,
    ) -> list[tuple[str, str]]:
        try:
            key = winreg.OpenKey(root, APP_PATHS_KEY, 0, winreg.KEY_READ | view)
        except (FileNotFoundError, PermissionError):
            return []
        result: list[tuple[str, str]] = []
        with key:
            index = 0
            while True:
                try:
                    subkey_name = winreg.EnumKey(key, index)
                except OSError:
                    break
                index += 1
                try:
                    with winreg.OpenKey(key, subkey_name) as subkey:
                        value, _value_type = winreg.QueryValueEx(subkey, "")
                except (FileNotFoundError, OSError):
                    continue
                if isinstance(value, str):
                    result.append((subkey_name, value))
        return result

    @staticmethod
    def _uninstall_paths_for(
        root: int,
        view: int,
    ) -> list[tuple[str, str]]:
        try:
            key = winreg.OpenKey(root, UNINSTALL_KEY, 0, winreg.KEY_READ | view)
        except (FileNotFoundError, PermissionError):
            return []
        result: list[tuple[str, str]] = []
        with key:
            index = 0
            while True:
                try:
                    subkey_name = winreg.EnumKey(key, index)
                except OSError:
                    break
                index += 1
                try:
                    with winreg.OpenKey(key, subkey_name) as subkey:
                        icon, _icon_type = winreg.QueryValueEx(subkey, "DisplayIcon")
                        try:
                            name, _name_type = winreg.QueryValueEx(
                                subkey,
                                "DisplayName",
                            )
                        except FileNotFoundError:
                            name = subkey_name
                except (FileNotFoundError, OSError):
                    continue
                if isinstance(icon, str) and isinstance(name, str):
                    result.append((name, icon))
        return result

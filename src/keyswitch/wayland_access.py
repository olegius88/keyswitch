"""Raw keyboard access for a Wayland session, given only on an administrator's explicit command.

In a Wayland session a program sees no keys of other windows, so KeySwitch reads the keyboards and types through
/dev/uinput itself (wayland_helper). The udev rule that lets the session's user do that lets every program of that
user do it as well: read every key typed, passwords included, and type keys of its own, as any program can on X11 and
Windows. The package installed the rule for everyone who installed or updated it, X11 users included (a code-scanning
review of 28.10.2026); the rule now ships inside the application and an administrator enables it once with
`sudo keyswitch --enable-wayland-input`, and takes it away with `--disable-wayland-input`.
"""

from __future__ import annotations

import glob
import os
import shutil
import subprocess
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Final, cast

from .constants.wayland import WAYLAND_ACCESS_FILE_MODE

ENABLE_COMMAND: Final = "sudo keyswitch --enable-wayland-input"
DISABLE_COMMAND: Final = "sudo keyswitch --disable-wayland-input"
# What a user who meets the missing access is told, in every place that reports it.
ACCESS_HINT: Final = (
    f"в сеансе Wayland KeySwitch читает клавиатуру на уровне системы; доступ включает администратор один раз "
    f"командой «{ENABLE_COMMAND}». После этого любая программа вашего сеанса сможет читать нажатия и печатать сама, "
    f"как в X11 и Windows; выключить — «{DISABLE_COMMAND}»"
)

RESOURCES: Final = Path(__file__).parent / "resources" / "wayland-input"
# The application's resource and where enabling puts it, relative to the root of the file system.
ACCESS_FILES: Final = (
    ("60-keyswitch.rules", "etc/udev/rules.d/60-keyswitch.rules"),
    ("keyswitch.modules-load.conf", "etc/modules-load.d/keyswitch.conf"),
)
RULE_DESTINATION: Final = ACCESS_FILES[0][1]
# The devices the rule gives access to: their ACLs stay until they are taken away (udev re-adds only tagged ones).
ACL_DEVICE_PATTERNS: Final = ("dev/uinput", "dev/input/event*")
RELOAD_COMMANDS: Final = (
    ("udevadm", "control", "--reload-rules"),
    ("udevadm", "trigger", "--subsystem-match=misc", "--sysname-match=uinput"),
    ("udevadm", "trigger", "--subsystem-match=input"),
)
LOAD_UINPUT: Final = ("modprobe", "uinput")
# udev and logind act on the triggered events after the command returns: the ACLs are stripped once they are done.
SETTLE: Final = ("udevadm", "settle")

Runner = Callable[[Sequence[str]], int]


def _run(command: Sequence[str]) -> int:
    """Run a system command; a missing program counts as a failure, not a crash."""

    try:
        return subprocess.run(list(command), check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode
    except OSError:
        return 1


def _effective_user() -> int:
    """The effective user id; os.geteuid is POSIX-only, and the Windows type check reads this module too."""

    return cast(Callable[[], int], getattr(os, "geteuid"))()


def access_enabled(root: Path = Path("/")) -> bool:
    """Whether an administrator has enabled the access (the rule stands in /etc/udev/rules.d)."""

    return (root / RULE_DESTINATION).exists()


def enable_wayland_input(
    *, root: Path = Path("/"), run: Runner = _run, euid: Callable[[], int] = _effective_user,
    out: Callable[[str], None] = print,
) -> int:
    """Install the rule and the module list, load uinput and apply the rule to the devices already present."""

    if euid() != 0:
        out(f"Нужны права администратора: {ENABLE_COMMAND}")
        return 1
    for name, destination in ACCESS_FILES:
        target = root / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(RESOURCES / name, target)
        target.chmod(WAYLAND_ACCESS_FILE_MODE)
    failed = [command for command in (LOAD_UINPUT, *RELOAD_COMMANDS) if run(command) != 0]
    out("Доступ KeySwitch к клавиатурам в сеансе Wayland включён. Теперь любая программа сеанса может читать "
        f"нажатия и печатать сама, как в X11 и Windows. Выключить: {DISABLE_COMMAND}")
    for command in failed:
        out(f"Команда не выполнилась: {' '.join(command)}; доступ появится после перезагрузки")
    return 0


def disable_wayland_input(
    *, root: Path = Path("/"), run: Runner = _run, euid: Callable[[], int] = _effective_user,
    out: Callable[[str], None] = print,
) -> int:
    """Remove the rule and the module list and take the access away from the devices at once.

    udev applies a removed rule to no device, and the ACLs it granted stay until they are stripped. Stripped before the
    devices are triggered, the ACL of /dev/uinput came back with the triggered event (Ubuntu 26.04, 29.10.2026), so the
    events are triggered and settled first; a device systemd grants on its own (a joystick) has its grant given back
    at the next event or login.
    """

    if euid() != 0:
        out(f"Нужны права администратора: {DISABLE_COMMAND}")
        return 1
    for _, destination in ACCESS_FILES:
        (root / destination).unlink(missing_ok=True)
    failed = [command for command in (*RELOAD_COMMANDS, SETTLE) if run(command) != 0]
    devices = sorted(path for pattern in ACL_DEVICE_PATTERNS for path in glob.glob(str(root / pattern)))
    stripped = bool(devices) and run(("setfacl", "-b", *devices)) == 0
    out("Доступ KeySwitch к клавиатурам в сеансе Wayland выключен.")
    if not stripped or failed:
        out("Права на устройства снимутся после перезагрузки.")
    return 0

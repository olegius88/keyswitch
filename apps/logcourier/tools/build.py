"""Native portable build. Never packages a user profile or accepts secrets."""

import hashlib
import importlib.metadata
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    from logcourier import __version__

    windows_options = ["--hide-console", "hide-early"] if sys.platform == "win32" else []
    # Optional task-local Ubuntu dependency; no system installation or network access here.
    extra_library = ROOT / ".local/qt-deps/unpacked/usr/lib/x86_64-linux-gnu/libxcb-cursor.so.0"
    native_options = ["--add-binary", f"{extra_library}:."] if extra_library.exists() else []
    subprocess.run(
        [
            sys.executable,
            "-m",
            "PyInstaller",
            "--noconfirm",
            "--onedir",
            "--name",
            "LogCourier",
            "--specpath",
            "build",
            "--distpath",
            "dist",
            "--workpath",
            "build/pyinstaller",
            *windows_options,
            *native_options,
            str(ROOT / "packaging/launcher.py"),
        ],
        cwd=ROOT,
        check=True,
    )
    folder = ROOT / "dist/LogCourier"
    shutil.copy2(ROOT / "README.md", folder / "README.md")
    shutil.copy2(ROOT / "LICENSE", folder / "LICENSE")
    for name in ("PySide6", "PySide6_Essentials", "PySide6_Addons", "shiboken6", "keyring"):
        distribution = importlib.metadata.distribution(name)
        for file in distribution.files or []:
            if "licenses" in file.parts and file.name:
                source = Path(distribution.locate_file(file))
                if source.is_file():
                    target = (
                        folder
                        / "third-party-licenses"
                        / name
                        / Path(*file.parts[file.parts.index("licenses") + 1 :])
                    )
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, target)
    (folder / "docs").mkdir(exist_ok=True)
    shutil.copy2(ROOT / "docs/VERIFICATION.md", folder / "docs/VERIFICATION.md")
    name = f"LogCourier-{__version__}-{platform.system().lower()}-{platform.machine().lower()}"
    archive = Path(
        shutil.make_archive(str(ROOT / "dist" / name), "zip", ROOT / "dist", "LogCourier")
    )
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (ROOT / "dist" / (name + ".sha256")).write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(archive)


if __name__ == "__main__":
    main()

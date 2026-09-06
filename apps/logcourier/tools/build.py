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

    subprocess.run(
        [
            sys.executable,
            "-m",
            "PyInstaller",
            "--noconfirm",
            "--distpath",
            "dist",
            "--workpath",
            "build/pyinstaller",
            str(ROOT / "packaging/logcourier.spec"),
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
    for document in (ROOT / "docs").glob("*.md"):
        shutil.copy2(document, folder / "docs" / document.name)
    name = f"LogCourier-{__version__}-{platform.system().lower()}-{platform.machine().lower()}"
    archive = Path(
        shutil.make_archive(str(ROOT / "dist" / name), "zip", ROOT / "dist", "LogCourier")
    )
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (ROOT / "dist" / (name + ".sha256")).write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(archive)


if __name__ == "__main__":
    main()

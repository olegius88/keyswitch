"""Compile the native Windows installer after build.py and verify_frozen.py."""

import hashlib
import os
import platform
import shutil
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def find_compiler() -> Path:
    found = shutil.which("ISCC.exe")
    if found:
        return Path(found)
    for name in ("ProgramFiles(x86)", "ProgramFiles"):
        if os.environ.get(name):
            candidate = Path(os.environ[name]) / "Inno Setup 6/ISCC.exe"
            if candidate.is_file():
                return candidate
    raise RuntimeError("Install Inno Setup 6 or add ISCC.exe to PATH before building.")


def build(root: Path = ROOT) -> Path:
    if platform.system() != "Windows":
        raise RuntimeError("The installer must be built and verified on native Windows.")
    source = root / "dist/LogCourier"
    for name in ("LogCourier.exe", "LogCourier-cli.exe", "LICENSE", "README.md"):
        if not (source / name).is_file():
            raise RuntimeError(f"Missing payload {name}; run tools/build.py first.")
    for name in ("_internal", "docs", "third-party-licenses"):
        folder = source / name
        if not folder.is_dir() or not any(path.is_file() for path in folder.rglob("*")):
            raise RuntimeError(f"Missing payload directory {name}; run tools/build.py first.")
    version = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))["project"][
        "version"
    ]
    output = root / "dist" / f"LogCourier-Setup-{version}-x64.exe"
    # A fresh directory prevents stale installers being mistaken for compiler output.
    import tempfile

    with tempfile.TemporaryDirectory(prefix="logcourier-setup-") as directory:
        subprocess.run(
            [
                str(find_compiler()),
                f"/DMyAppVersion={version}",
                f"/DSourceDir={source}",
                f"/DOutputDir={directory}",
                str(root / "packaging/windows/LogCourier.iss"),
            ],
            cwd=root,
            check=True,
            timeout=600,
        )
        compiled = Path(directory) / output.name
        if not compiled.is_file() or compiled.stat().st_size == 0:
            raise RuntimeError("Inno Setup did not produce the expected installer.")
        shutil.copy2(compiled, output)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(".exe.sha256").write_text(f"{digest}  {output.name}\n", encoding="ascii")
    print(f"{output} ({output.stat().st_size:,} bytes)")
    return output


if __name__ == "__main__":
    build()

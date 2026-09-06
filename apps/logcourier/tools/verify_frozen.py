"""Check the actual packaged executables, not just imports in the source environment."""

import json
import os
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
folder = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "dist/LogCourier"
windows = sys.platform == "win32"
gui = folder / ("LogCourier.exe" if windows else "LogCourier")
cli = folder / "LogCourier-cli.exe" if windows else gui
if windows:
    for path, subsystem in ((gui, 2), (cli, 3)):
        data = path.read_bytes()
        header = struct.unpack_from("<I", data, 0x3C)[0]
        assert data[header : header + 4] == b"PE\0\0"
        assert struct.unpack_from("<H", data, header + 24 + 68)[0] == subsystem
subprocess.run([str(cli), "--version"], check=True, timeout=20)
with tempfile.TemporaryDirectory(prefix="logcourier-frozen-check-") as directory:
    output = Path(directory) / "result.json"
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", LOGCOURIER_DATA_DIR=directory)
    env.pop("LOGCOURIER_BOT_TOKEN", None)
    subprocess.run(
        [str(gui), "self-test", "--output", str(output)], env=env, check=True, timeout=30
    )
    assert json.loads(output.read_text())["passed"] is True
print("Frozen GUI launch, close, restore and shutdown: passed")

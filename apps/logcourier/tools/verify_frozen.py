"""Check the actual packaged executables, not just imports in the source environment."""

import json
import os
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

from logcourier.constants.release import (
    CLI_VERSION_CHECK_TIMEOUT_SECONDS,
    GUI_SELF_TEST_TIMEOUT_SECONDS,
)
from logcourier.constants.windows import (
    DOS_HEADER_PE_OFFSET_FIELD,
    OPTIONAL_HEADER_SUBSYSTEM_OFFSET,
    PE_SIGNATURE_AND_COFF_HEADER_BYTES,
    PE_SIGNATURE_BYTES,
    WINDOWS_SUBSYSTEM_CUI,
    WINDOWS_SUBSYSTEM_GUI,
)

root = Path(__file__).resolve().parents[1]
folder = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "dist/LogCourier"
windows = sys.platform == "win32"
gui = folder / ("LogCourier.exe" if windows else "LogCourier")
cli = folder / "LogCourier-cli.exe" if windows else gui
if windows:
    for path, subsystem in ((gui, WINDOWS_SUBSYSTEM_GUI), (cli, WINDOWS_SUBSYSTEM_CUI)):
        data = path.read_bytes()
        header = struct.unpack_from("<I", data, DOS_HEADER_PE_OFFSET_FIELD)[0]
        assert data[header : header + PE_SIGNATURE_BYTES] == b"PE\0\0"
        assert (
            struct.unpack_from(
                "<H",
                data,
                header + PE_SIGNATURE_AND_COFF_HEADER_BYTES + OPTIONAL_HEADER_SUBSYSTEM_OFFSET,
            )[0]
            == subsystem
        )
subprocess.run([str(cli), "--version"], check=True, timeout=CLI_VERSION_CHECK_TIMEOUT_SECONDS)
with tempfile.TemporaryDirectory(prefix="logcourier-frozen-check-") as directory:
    output = Path(directory) / "result.json"
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", LOGCOURIER_DATA_DIR=directory)
    env.pop("LOGCOURIER_BOT_TOKEN", None)
    subprocess.run(
        [str(gui), "self-test", "--output", str(output)],
        env=env,
        check=True,
        timeout=GUI_SELF_TEST_TIMEOUT_SECONDS,
    )
    assert json.loads(output.read_text())["passed"] is True
print("Frozen GUI launch, close, restore and shutdown: passed")

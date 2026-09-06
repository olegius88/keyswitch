import sys
from pathlib import Path

from logcourier.__main__ import main
from logcourier.startup import report_error

if __name__ == "__main__":
    desktop = sys.platform == "win32" and not Path(sys.executable).stem.endswith("-cli")
    raise SystemExit(main(error_handler=report_error if desktop else None))

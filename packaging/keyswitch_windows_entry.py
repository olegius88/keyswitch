"""Build-only entry point for the native Windows executable."""

import logging
import os
import sys
import threading

from keyswitch.windows_app import main


def leave(status: int) -> None:
    """End the process without finalising the interpreter around the engine's loader.

    The engine is built on a daemon thread while the window comes up, so a window closed in the program's first seconds
    leaves that thread running. The installer check of 0.66.0 never finished on Windows on Arm, where such a quit is
    the rule, while the same check takes half a minute on x64 and did on Arm with 0.65.0, whose engine was built before
    the window. What finalising would have waited for and flushed is done here; the rest ends with the process.
    """

    for thread in threading.enumerate():
        if thread is not threading.current_thread() and not thread.daemon:
            thread.join()
    logging.shutdown()
    for stream in (sys.stdout, sys.stderr):
        # A windowed program may have no streams, and a redirected one may be closed already.
        if stream is not None:
            try:
                stream.flush()
            except (OSError, ValueError):
                pass
    os._exit(status)


if __name__ == "__main__":
    leave(main())

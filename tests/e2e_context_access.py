#!/usr/bin/env python3
"""Real GTK4/AT-SPI caret, selected text and password privacy contracts."""

from __future__ import annotations

import threading
import time
import os
import subprocess
import sys
import tempfile

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

from keyswitch.atspi_context import AtspiFieldReader
from keyswitch.input_context import FieldContext
from keyswitch.x11_backend import X11Backend
from fixture_values.clock import (
    ATSPI_UNAVAILABLE_PROBE_TIMEOUT_SECONDS,
    CONTEXT_ACCESS_E2E_INITIAL_PREPARE_DELAY_MS,
    CONTEXT_ACCESS_E2E_POLL_SECONDS,
    CONTEXT_ACCESS_E2E_PREPARE_SETTLE_DELAY_MS,
    CONTEXT_ACCESS_E2E_READ_DEADLINE_SECONDS,
    CONTEXT_ACCESS_E2E_START_READ_DELAY_MS,
    CONTEXT_ACCESS_E2E_TIMEOUT_SECONDS,
    CONTEXT_ACCESS_E2E_WORKER_JOIN_TIMEOUT_SECONDS,
)
from fixture_values.counts import (
    CONTEXT_ACCESS_E2E_PASSWORD_STAGE,
    CONTEXT_ACCESS_E2E_SELECTION_CHARACTERS,
    CONTEXT_ACCESS_E2E_STAGE_COUNT,
)


def verify_unavailable_bus() -> None:
    # Native g_error()/SIGABRT is not a Python exception. Exercise the actual
    # library in a fresh process, including a second reader after engine restart.
    script = """
from keyswitch.context_access import PlatformFieldReader
for _ in range(2):
    reader = PlatformFieldReader()
    assert reader.read('TestEditor', 1) is None
    assert reader.status == 'unavailable'
    reader.close()
print('AT_SPI_UNAVAILABLE_OK')
"""
    with tempfile.TemporaryDirectory(prefix="keyswitch-atspi-e2e-") as directory:
        result = subprocess.run(
            [sys.executable, "-c", script],
            env={**os.environ, "AT_SPI_BUS_ADDRESS": f"unix:path={directory}/missing-bus"},
            capture_output=True, text=True, timeout=ATSPI_UNAVAILABLE_PROBE_TIMEOUT_SECONDS, check=False,
        )
    assert result.returncode == 0, (result.returncode, result.stderr)
    assert "AT_SPI_UNAVAILABLE_OK" in result.stdout, result.stdout
    print("AT_SPI_UNAVAILABLE_OK")


def main() -> int:
    verify_unavailable_bus()
    Gtk.init()
    backend = X11Backend()
    backend._open()
    loop = GLib.MainLoop()
    window = Gtk.Window(title="KeySwitch accessibility E2E")
    entry = Gtk.Entry()
    window.set_child(entry)
    window.present()
    entry.grab_focus()
    prefix, suffix = "ранее вставлено ghbdtn", " после курсора"
    stage = 0
    success = False
    worker: threading.Thread | None = None

    def finish() -> bool:
        print("AT_SPI_TIMEOUT")
        loop.quit()
        return GLib.SOURCE_REMOVE

    def checked(snapshot: FieldContext | None, error: str) -> bool:
        nonlocal stage, success
        try:
            assert not error, error
            assert snapshot is not None, "No focused accessible text field"
            if stage == 0:
                assert (snapshot.before, snapshot.after) == (prefix, suffix), snapshot
            elif stage == 1:
                assert snapshot.selection and not snapshot.before and not snapshot.after, snapshot
            else:
                assert snapshot.sensitive and not snapshot.before and not snapshot.after, snapshot
            print(f"AT_SPI_STAGE_{stage}_OK")
        except AssertionError as failure:
            print(f"AT_SPI_FAILED: {failure}")
            print(f"GTK_SELECTION_NOW: {entry.get_selection_bounds()}")
            loop.quit()
            return GLib.SOURCE_REMOVE
        stage += 1
        if stage == CONTEXT_ACCESS_E2E_STAGE_COUNT:
            success = True
            loop.quit()
        else:
            GLib.idle_add(prepare)
        return GLib.SOURCE_REMOVE

    def read() -> None:
        snapshot: FieldContext | None = None
        error = ""
        try:
            reader = AtspiFieldReader(process_for_window=backend.window_process_id)
            deadline = time.monotonic() + CONTEXT_ACCESS_E2E_READ_DEADLINE_SECONDS
            while snapshot is None and time.monotonic() < deadline:
                focus = backend.focused_window()
                if focus is not None:
                    # Deliberately not the accessible app name: use the X11 PID.
                    snapshot = reader.read("DifferentWMClass", focus.window)
                # Provider notifications are asynchronous. Wait for the new
                # caret/selection/visibility state, not an older valid reply.
                ready = snapshot is not None and (
                    (stage == 0 and snapshot.before == prefix and snapshot.after == suffix)
                    or (stage == 1 and snapshot.selection)
                    or (stage == CONTEXT_ACCESS_E2E_PASSWORD_STAGE and snapshot.sensitive)
                )
                if not ready:
                    snapshot = None
                if snapshot is None:
                    time.sleep(CONTEXT_ACCESS_E2E_POLL_SECONDS)
        except Exception as failure:
            error = str(failure)
        GLib.idle_add(checked, snapshot, error)

    def start_read() -> bool:
        nonlocal worker
        worker = threading.Thread(target=read, daemon=True)
        worker.start()
        return GLib.SOURCE_REMOVE

    def prepare() -> bool:
        entry.grab_focus()
        entry.set_visibility(stage != CONTEXT_ACCESS_E2E_PASSWORD_STAGE)
        entry.set_text(prefix + suffix)
        # Focus/old PRIMARY ownership can enqueue SelectionClear. Let those
        # notifications drain before creating the selection we intend to test.
        GLib.timeout_add(CONTEXT_ACCESS_E2E_PREPARE_SETTLE_DELAY_MS, select_prepared)
        return GLib.SOURCE_REMOVE

    def select_prepared() -> bool:
        entry.set_position(len(prefix))
        if stage == 1:
            entry.select_region(0, CONTEXT_ACCESS_E2E_SELECTION_CHARACTERS)
            print(f"GTK_SELECTED: {entry.get_selection_bounds()}")
        GLib.timeout_add(CONTEXT_ACCESS_E2E_START_READ_DELAY_MS, start_read)
        return GLib.SOURCE_REMOVE

    GLib.timeout_add(CONTEXT_ACCESS_E2E_INITIAL_PREPARE_DELAY_MS, prepare)
    GLib.timeout_add_seconds(CONTEXT_ACCESS_E2E_TIMEOUT_SECONDS, finish)
    try:
        loop.run()
    finally:
        if worker is not None:
            worker.join(timeout=CONTEXT_ACCESS_E2E_WORKER_JOIN_TIMEOUT_SECONDS)
        window.destroy()
        backend.close()
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())

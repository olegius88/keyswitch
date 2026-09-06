import hashlib
import io
import json
import os
import zipfile

import pytest

from logcourier.batching import compact
from logcourier.catalog import current_catalog, deliver, list_entries
from logcourier.collector import Collector
from logcourier.store import QueueFull, Store
from logcourier.telegram import TelegramError


def line(version, text="text", time="12:00:00"):
    return f"2026-09-07 {time},123 {version} INFO keyswitch.engine: {text}\r\n".encode()


def logs(store, config):
    return [
        row
        for row in store.queue(config.destination)
        if json.loads(row["meta"]).get("kind") != "keyswitch.version"
    ]


def markers(store, config):
    return [
        json.loads(row["meta"])
        for row in store.queue(config.destination)
        if json.loads(row["meta"]).get("kind") == "keyswitch.version"
    ]


def collect_all(store, config, chunk_size=2 * 1024 * 1024):
    for _ in range(1000):
        if not Collector(store, chunk_size).scan(config)[0]:
            return
    raise AssertionError("collector did not finish")


def deliver_all(store, config, telegram):
    for _ in range(1000):
        if not store.queue(config.destination):
            return
        deliver(store, config, telegram)
    raise AssertionError("delivery did not finish")


def raw(row):
    with zipfile.ZipFile(io.BytesIO(row["payload"])) as archive:
        data = archive.read("fragment.log")
        meta = json.loads(archive.read("metadata.json"))
        assert meta["raw_sha256"] == hashlib.sha256(data).hexdigest()
        assert meta.get("keyswitch_version") == json.loads(row["meta"]).get("keyswitch_version")
        return data


@pytest.mark.parametrize("chunk_size", [7, 71, 100000])
def test_mixed_file_is_split_without_changing_bytes(store, configured, chunk_size):
    config, path = configured
    old = line("0.16.1", "старое") + b"Traceback: continuation\r\n"
    new = line("0.16.2", "новое", "12:01:00") + "  ещё строка\n".encode()
    path.write_bytes(old + new)
    collect_all(store, config, chunk_size)
    rows = logs(store, config)
    assert b"".join(raw(row) for row in rows) == old + new
    for version, expected in (("0.16.1", old), ("0.16.2", new)):
        assert (
            b"".join(
                raw(row) for row in rows if json.loads(row["meta"])["keyswitch_version"] == version
            )
            == expected
        )
    assert [m["keyswitch_version"] for m in markers(store, config)] == ["0.16.2"]


def test_latest_is_observed_before_reading_old_rotation_backlog(store, configured, telegram):
    config, path = configured
    path.with_name(path.name + ".1").write_bytes(line("0.15.0") * 20)
    path.write_bytes(line("0.16.2"))
    Collector(store, chunk_size=80).scan(config, max_chunks=1)
    deliver_all(store, config, telegram)
    head = current_catalog(telegram, config.chat_id)["index"]
    assert head["keyswitch_versions"][config.sources[0].id]["version"] == "0.16.2"
    assert list_entries(telegram, config.chat_id, keyswitch_version="current") == []
    collect_all(store, config)
    while compact(store, config):
        pass
    deliver_all(store, config, telegram)
    entries = list_entries(telegram, config.chat_id, keyswitch_version="current")
    assert entries and all(e["keyswitch_version"] == "0.16.2" for e in entries)


def test_restart_same_version_and_rollback_mark_once(tmp_path, store, configured):
    config, path = configured
    for version in ("0.16.1", "0.16.2", "0.16.2", "0.16.1"):
        with path.open("ab") as stream:
            stream.write(line(version))
        collect_all(store, config)
    assert [m["keyswitch_version"] for m in markers(store, config)] == [
        "0.16.1",
        "0.16.2",
        "0.16.1",
    ]
    reopened = Store(tmp_path / "state")
    try:
        collect_all(reopened, config)
        assert len(markers(reopened, config)) == 3
    finally:
        reopened.close()


def test_initial_baseline_does_not_send_existing_bytes_but_knows_version(store, configured):
    config, path = configured
    config.sources[0].include_existing = False
    path.write_bytes(line("0.16.2"))
    assert Collector(store).scan(config)[0] == 0
    assert logs(store, config) == []
    assert markers(store, config)[0]["keyswitch_version"] == "0.16.2"
    with path.open("ab") as stream:
        stream.write(line("0.16.2", "next"))
    collect_all(store, config)
    assert b"".join(raw(row) for row in logs(store, config)) == line("0.16.2", "next")


def test_current_selection_filters_before_limit_and_excludes_legacy(store, configured, telegram):
    config, path = configured
    collect_all(store, config)
    deliver_all(store, config, telegram)
    path.write_bytes(line("0.16.2"))
    collect_all(store, config)
    deliver_all(store, config, telegram)
    # An old rotation arrives after the new version marker and data.
    path.with_name(path.name + ".1").write_bytes(line("0.16.1"))
    collect_all(store, config)
    deliver_all(store, config, telegram)
    current = list_entries(telegram, config.chat_id, limit=1, keyswitch_version="current")
    assert len(current) == 1 and current[0]["keyswitch_version"] == "0.16.2"
    assert len(list_entries(telegram, config.chat_id)) == 3


def test_full_queue_does_not_acknowledge_version_change(tmp_path, configured):
    config, path = configured
    path.write_bytes(line("0.16.2"))
    store = Store(tmp_path / "small", capacity=1)
    try:
        with pytest.raises(QueueFull):
            Collector(store).scan(config)
        assert not store.get("keyswitch_versions:" + config.destination)
        store.capacity = 100000
        collect_all(store, config)
        assert len(markers(store, config)) == 1
    finally:
        store.close()


def test_partial_header_waits_and_continuations_survive_restart(tmp_path, store, configured):
    config, path = configured
    first = line("0.16.1")
    next_line = line("0.16.2", "новый текст")
    path.write_bytes(first + next_line[:33])
    collect_all(store, config, 9)
    assert b"".join(raw(row) for row in logs(store, config)) == first
    with path.open("ab") as stream:
        stream.write(next_line[33:] + b"traceback part one")
    collect_all(store, config, 9)
    reopened = Store(tmp_path / "state")
    try:
        with path.open("ab") as stream:
            stream.write(b" part two\n")
        collect_all(reopened, config, 9)
        new = [
            row
            for row in logs(reopened, config)
            if json.loads(row["meta"])["keyswitch_version"] == "0.16.2"
        ]
        assert b"".join(raw(row) for row in new) == next_line + b"traceback part one part two\n"
    finally:
        reopened.close()


def test_reset_does_not_assign_version_to_unstamped_text(store, configured):
    config, path = configured
    path.write_bytes(line("0.16.2"))
    collect_all(store, config)
    path.write_bytes(b"unknown\n")
    collect_all(store, config)
    last = logs(store, config)[-1]
    assert raw(last) == b"unknown\n"
    assert json.loads(last["meta"])["keyswitch_version"] is None


def test_compaction_keeps_source_versions_separate_and_skips_markers(store, configured):
    from logcourier.config import Source

    config, path = configured
    second = path.with_name("other.log")
    second.write_bytes(line("0.16.2"))
    config.sources.append(Source(str(second), include_existing=True))
    path.write_bytes(b"unknown prefix\n" + line("0.16.1") * 2 + line("0.16.2") * 2)
    collect_all(store, config, 70)
    before_markers = markers(store, config)
    originals = {row["id"]: row["payload"] for row in logs(store, config)}
    assert compact(store, config)
    while compact(store, config):
        pass
    assert markers(store, config) == before_markers
    seen = set()
    for row in logs(store, config):
        metadata = json.loads(row["meta"])
        if not metadata.get("batch"):
            seen.add(row["id"])
            continue
        with zipfile.ZipFile(io.BytesIO(row["payload"])) as archive:
            manifest = json.loads(archive.read("manifest.json"))
            for fragment in manifest["fragments"]:
                assert fragment["keyswitch_version"] == metadata["keyswitch_version"]
                if metadata["keyswitch_version"]:
                    assert fragment["source_id"] == metadata["source_id"]
                assert archive.read(fragment["archive"]) == originals[fragment["bundle_id"]]
                seen.add(fragment["bundle_id"])
    assert seen == set(originals)


def test_marker_uses_existing_rate_limit_and_is_not_reuploaded_after_pin_failure(
    store, configured, telegram
):
    from logcourier.rate_limit import RateLimitedClient

    config, path = configured
    path.write_bytes(line("0.16.2"))
    collect_all(store, config)
    clock = [100.0]
    calls = []
    telegram.after_upload = lambda: calls.append(clock[0])

    def wait(seconds):
        clock[0] += seconds

    client = RateLimitedClient(
        telegram, store, config.chat_id, lambda: False, lambda: clock[0], wait
    )
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        deliver(store, config, client)
    assert calls == [100.0, 104.0, 108.0]  # marker, data, catalog
    assert "МАРКЕР ВЕРСИИ" in telegram.messages[1]["caption"]
    assert "0.16.2" in telegram.messages[1]["caption"]
    telegram.fail_pin = False
    deliver(store, config, client)
    assert telegram.uploads == 3
    assert len(list_entries(telegram, config.chat_id, keyswitch_version="current")) == 1


def test_current_versions_are_per_source_and_removed_sources_are_not_current(
    store, configured, telegram
):
    from logcourier.config import Source

    config, path = configured
    second = path.with_name("second.log")
    second.write_bytes(line("0.16.1"))
    config.sources.append(Source(str(second), include_existing=True))
    path.write_bytes(line("0.16.2"))
    collect_all(store, config)
    deliver_all(store, config, telegram)
    current = list_entries(telegram, config.chat_id, keyswitch_version="current")
    assert {e["keyswitch_version"] for e in current} == {"0.16.1", "0.16.2"}
    config.sources.pop()
    deliver(store, config, telegram)  # publish updated current map even without new data
    assert {
        e["keyswitch_version"]
        for e in list_entries(telegram, config.chat_id, keyswitch_version="current")
    } == {"0.16.2"}
    assert len(list_entries(telegram, config.chat_id)) == 2


def test_cli_default_download_is_isolated_from_old_logs(
    tmp_path, store, configured, telegram, monkeypatch, capsys
):
    from logcourier import __main__
    from logcourier.config import save_config

    config, path = configured
    save_config(tmp_path, config)
    monkeypatch.setenv("LOGCOURIER_DATA_DIR", str(tmp_path))
    monkeypatch.setenv("LOGCOURIER_BOT_TOKEN", "not used by fake")
    monkeypatch.setattr(__main__, "Telegram", lambda _: telegram)
    path.write_bytes(line("0.16.1") + line("0.16.2"))
    collect_all(store, config)
    deliver_all(store, config, telegram)
    output = tmp_path / "received"
    output.mkdir()
    (output / "old.zip").write_bytes(b"old version from a previous fetch")
    assert __main__.main(["fetch", "--output", str(output)]) == 0
    manifests = list(output.glob("*/selection.json"))
    assert len(manifests) == 1
    manifest = json.loads(manifests[0].read_bytes())
    assert manifest["keyswitch_version"] == "current"
    assert len(manifest["files"]) == 1
    assert manifest["files"][0]["keyswitch_version"] == "0.16.2"
    assert __main__.main(["fetch", "--output", str(output)]) == 0
    assert list(output.glob("*/selection.json")) == manifests
    assert (output / "old.zip").read_bytes() == b"old version from a previous fetch"
    assert __main__.main(["fetch", "--keyswitch-version", "0.16.1", "--output", str(output)]) == 0
    assert len(list(output.glob("*/selection.json"))) == 2
    capsys.readouterr()
    assert __main__.main(["list"]) == 0
    assert {e["keyswitch_version"] for e in json.loads(capsys.readouterr().out)} == {"0.16.2"}


def test_legacy_catalog_requires_explicit_history_access(store, configured, telegram):
    config, _ = configured
    collect_all(store, config)
    deliver_all(store, config, telegram)
    with pytest.raises(TelegramError, match="all-versions"):
        list_entries(telegram, config.chat_id, keyswitch_version="current")
    assert list_entries(telegram, config.chat_id, keyswitch_version=None)
    assert list_entries(telegram, config.chat_id, keyswitch_version="0.16.2") == []


@pytest.mark.parametrize(
    "field,value",
    [
        ("version", "0.16.2/../../file"),
        ("version", 3),
        ("version", None),
        ("marker_id", "bad"),
        ("previous_version", []),
        ("detected_at", "bad"),
    ],
)
def test_invalid_catalog_version_metadata_is_rejected(store, configured, telegram, field, value):
    from logcourier.catalog import decode_index

    config, path = configured
    path.write_bytes(line("0.16.2"))
    collect_all(store, config)
    deliver_all(store, config, telegram)
    index = current_catalog(telegram, config.chat_id)["index"]
    index["keyswitch_versions"][config.sources[0].id][field] = value
    with pytest.raises(TelegramError):
        decode_index(json.dumps(index).encode(), config.chat_id, config.bot_id)


@pytest.mark.parametrize("length", [1, 4, 8, 23, 33])
def test_even_one_unfinished_timestamp_byte_cannot_inherit_old_version(store, configured, length):
    config, path = configured
    old, new = line("0.16.1"), line("0.16.2", "новая запись")
    path.write_bytes(old)
    collect_all(store, config)
    with path.open("ab") as stream:
        stream.write(new[:length])
    collect_all(store, config)
    assert b"".join(raw(row) for row in logs(store, config)) == old
    with path.open("ab") as stream:
        stream.write(new[length:])
    collect_all(store, config)
    assert raw(logs(store, config)[-1]) == new
    assert json.loads(logs(store, config)[-1]["meta"])["keyswitch_version"] == "0.16.2"
    assert [m["keyswitch_version"] for m in markers(store, config)] == ["0.16.1", "0.16.2"]


def test_tail_probe_is_bounded_and_ignores_versions_in_message_text():
    from logcourier.versions import PROBE_BYTES, latest_version

    data = line("0.16.1") + b"x" * PROBE_BYTES + b"\n" + line("0.16.2", "model_version=9.9.9")
    assert latest_version(io.BytesIO(data)) == "0.16.2"
    assert latest_version(io.BytesIO(line("0.16.2") + b"x" * (PROBE_BYTES + 10))) is None
    assert latest_version(io.BytesIO(b"model_version=0.16.2\n")) is None
    assert latest_version(io.BytesIO(line("0.16.2") + line("unknown"))) is None
    assert latest_version(io.BytesIO(line("0.16.3-rc.1"))) == "0.16.3-rc.1"


def test_unrecognized_header_starts_unknown_fragment(store, configured):
    config, path = configured
    old = line("0.16.1")
    unknown = line("unknown") + b"traceback continuation\n"
    path.write_bytes(old + unknown)
    collect_all(store, config)
    assert [json.loads(row["meta"])["keyswitch_version"] for row in logs(store, config)] == [
        "0.16.1",
        None,
    ]
    assert b"".join(raw(row) for row in logs(store, config)) == old + unknown


def test_version_probes_all_sources_even_if_first_fills_read_budget(store, configured):
    from logcourier.config import Source

    config, path = configured
    path.write_bytes(line("0.16.1") * 100)
    second = path.with_name("other.log")
    second.write_bytes(line("0.16.2"))
    config.sources.append(Source(str(second), include_existing=True))
    count, errors = Collector(store, chunk_size=80).scan(config, max_chunks=1)
    assert count == 1 and not errors
    assert [m["keyswitch_version"] for m in markers(store, config)] == ["0.16.1", "0.16.2"]


def test_marker_requires_consent_and_does_not_move_between_destinations(store, configured):
    config, path = configured
    path.write_bytes(line("0.16.2"))
    config.consent = False
    assert Collector(store).scan(config) == (0, [])
    assert store.queue_bytes() == 0
    config.consent = True
    collect_all(store, config)
    previous_destination = config.destination
    before = [(row["id"], row["payload"]) for row in store.queue(previous_destination)]
    config.chat_id = "-100999"
    collect_all(store, config)
    assert [(row["id"], row["payload"]) for row in store.queue(previous_destination)] == before
    assert len(markers(store, config)) == 1
    assert markers(store, config)[0]["previous_version"] is None
    assert logs(store, config) == []


def test_cli_empty_current_selection_never_substitutes_legacy_data(
    tmp_path, store, configured, telegram, monkeypatch, capsys
):
    from logcourier import __main__
    from logcourier.config import save_config
    from logcourier.versions import observe_version

    config, _ = configured
    collect_all(store, config)  # existing unversioned data
    observe_version(store, config, config.sources[0], "0.16.2")
    deliver_all(store, config, telegram)
    save_config(tmp_path, config)
    monkeypatch.setenv("LOGCOURIER_DATA_DIR", str(tmp_path))
    monkeypatch.setenv("LOGCOURIER_BOT_TOKEN", "fake only")
    monkeypatch.setattr(__main__, "Telegram", lambda _: telegram)
    output = tmp_path / "received"
    assert __main__.main(["fetch", "--output", str(output)]) == 0
    assert "пока нет" in capsys.readouterr().err and not output.exists()
    assert __main__.main(["fetch", "--all-versions", "--output", str(output)]) == 0
    assert len(list(output.glob("*.zip"))) == 1


def test_failed_download_does_not_publish_completed_selection(
    tmp_path, store, configured, telegram, monkeypatch, capsys
):
    from logcourier import __main__
    from logcourier.config import save_config

    config, path = configured
    path.write_bytes(line("0.16.2") * 2)
    collect_all(store, config, len(line("0.16.2")))
    deliver_all(store, config, telegram)
    entries = list_entries(telegram, config.chat_id, keyswitch_version="current")
    assert len(entries) == 2
    last = entries[-1]["file_id"]
    original = telegram.files[last]
    telegram.files[last] = b"corrupt"
    save_config(tmp_path, config)
    monkeypatch.setenv("LOGCOURIER_DATA_DIR", str(tmp_path))
    monkeypatch.setenv("LOGCOURIER_BOT_TOKEN", "fake only")
    monkeypatch.setattr(__main__, "Telegram", lambda _: telegram)
    output = tmp_path / "received"
    args = ["fetch", "--output", str(output)]
    assert __main__.main(args) == 1
    assert "сумма" in capsys.readouterr().err
    assert len(list(output.glob("*/*.zip"))) == 1
    assert not list(output.glob("*/selection.json"))
    telegram.files[last] = original
    assert __main__.main(args) == 0
    assert len(list(output.glob("*/*.zip"))) == 2
    assert len(list(output.glob("*/selection.json"))) == 1


@pytest.mark.parametrize(
    "collision",
    [
        pytest.param(
            "directory_link",
            marks=pytest.mark.skipif(
                os.name == "nt", reason="Реальные symlink проверяются на Linux"
            ),
        ),
        pytest.param(
            "manifest_link",
            marks=pytest.mark.skipif(
                os.name == "nt", reason="Реальные symlink проверяются на Linux"
            ),
        ),
        "manifest",
        "extra",
    ],
)
def test_download_selection_refuses_foreign_files_and_links(tmp_path, collision):
    from logcourier.__main__ import selection_directory

    entry = {
        "bundle_id": "a" * 32,
        "source_id": "b" * 32,
        "keyswitch_version": "0.16.2",
        "sha256": "c" * 64,
        "size": 10,
    }
    folder, _ = selection_directory(tmp_path, [entry], "current")
    unrelated = tmp_path / "unrelated"
    unrelated.mkdir()
    if collision == "directory_link":
        folder.rename(tmp_path / "original")
        folder.symlink_to(unrelated, target_is_directory=True)
    elif collision == "manifest_link":
        (folder / "selection.json").symlink_to(unrelated / "nonexistent")
    elif collision == "manifest":
        (folder / "selection.json").write_bytes(b"other manifest")
    else:
        (folder / "older.zip").write_bytes(b"old logs")
    with pytest.raises(ValueError):
        selection_directory(tmp_path, [entry], "current")
    assert not list(unrelated.iterdir())


def test_gui_reports_version_only_for_configured_sources(tmp_path, configured, monkeypatch):
    from PySide6.QtWidgets import QApplication

    from logcourier import gui

    config, _ = configured
    monkeypatch.setattr(gui, "read_token", lambda *_: None)
    monkeypatch.delenv("LOGCOURIER_BOT_TOKEN", raising=False)
    app = QApplication.instance() or QApplication([])
    window = gui.Window(tmp_path, config, start_service=False)
    try:
        window.on_status(
            "Сбор",
            {
                "keyswitch_versions": {
                    config.sources[0].id: {"source_label": "Active", "version": "0.16.2"},
                    "f" * 32: {"source_label": "Removed", "version": "0.16.1"},
                }
            },
        )
        assert "Active: 0.16.2" in window.summary.text()
        assert "0.16.1" not in window.summary.text()
    finally:
        window.exiting = True
        window.tray.hide()
        window.close()
        app.processEvents()

import hashlib
import io
import json
import os
import zipfile

import pytest

from logcourier.collector import Collector, open_regular
from logcourier.config import Source, load_config, save_config
from logcourier.store import QueueFull, Store


def fragments(store, config):
    result = []
    for row in store.queue(config.destination):
        with zipfile.ZipFile(io.BytesIO(row["payload"])) as archive:
            result.append(archive.read("fragment.log"))
            meta = json.loads(archive.read("metadata.json"))
            assert "path" not in meta
            assert meta["raw_sha256"] == hashlib.sha256(result[-1]).hexdigest()
            assert sorted(archive.namelist()) == ["fragment.log", "metadata.json"]
    return b"".join(result)


def test_roundtrip_and_safe_defaults(tmp_path, configured):
    config, path = configured
    save_config(tmp_path, config)
    assert load_config(tmp_path) == config
    assert "token" not in (tmp_path / "config.json").read_text()
    defaults = load_config(tmp_path / "absent")
    assert not defaults.consent and not defaults.auto_send
    config.sources.append(Source(str(path)))
    with pytest.raises(ValueError, match="дважды"):
        config.validate()


@pytest.mark.parametrize(
    "field,value",
    [("consent", "false"), ("auto_send", 1), ("interval_minutes", True), ("interval_minutes", 0)],
)
def test_invalid_configuration(configured, field, value):
    config, _ = configured
    setattr(config, field, value)
    with pytest.raises(ValueError):
        config.validate()


def test_exact_bytes_and_restart(store, configured):
    config, path = configured
    data = "Привет, мир! ghbdtn\r\nзь2 — pm2; ёЁ / ".encode() + b"\xff\x00end"
    path.write_bytes(data)
    collector = Collector(store, chunk_size=11)
    while collector.scan(config)[0]:
        pass
    assert fragments(store, config) == data
    assert Collector(store).scan(config) == (0, [])


def test_only_new_content_with_rotation_baseline(store, configured):
    config, path = configured
    config.sources[0].include_existing = False
    rotated = path.with_name(path.name + ".1")
    rotated.write_bytes(b"older\n")
    collector = Collector(store)
    assert collector.scan(config) == (0, [])
    with path.open("ab") as stream:
        stream.write(b"new\n")
    assert collector.scan(config)[0] == 1
    path.replace(rotated)
    path.write_bytes(b"after rotation\n")
    assert collector.scan(config)[0] == 1
    assert fragments(store, config) == b"new\nafter rotation\n"


def test_copytruncate_regrows_past_old_offset(store, configured):
    config, path = configured
    collector = Collector(store)
    collector.scan(config)
    path.write_bytes(b"entirely new and longer\n")
    collector.scan(config)
    assert fragments(store, config) == b"old\nentirely new and longer\n"
    assert json.loads(store.queue(config.destination)[-1]["meta"])["source_reset"]


def test_consent_and_missing_source(store, configured):
    config, path = configured
    config.consent = False
    assert Collector(store).scan(config) == (0, [])
    config.consent = True
    path.unlink()
    assert Collector(store).scan(config)[1]
    path.write_bytes(b"created later")
    assert Collector(store).scan(config)[0] == 1


def test_queue_full_does_not_advance_cursor(tmp_path, configured):
    config, _ = configured
    store = Store(tmp_path / "small", capacity=1)
    with pytest.raises(QueueFull):
        Collector(store).scan(config)
    assert not store.queue(config.destination)
    assert not store.db.execute("SELECT * FROM cursors").fetchall()
    store.capacity = 10000
    assert Collector(store).scan(config)[0] == 1
    store.close()


def test_destination_isolation(store, configured):
    config, _ = configured
    Collector(store).scan(config)
    config.chat_id = "-200"
    assert not store.queue(config.destination)
    assert store.stats(config.destination)["other"] == 1


def test_reject_non_regular_and_symlink(tmp_path):
    with pytest.raises((OSError, ValueError)):
        open_regular(tmp_path)
    if os.name != "nt":
        source = tmp_path / "source"
        source.write_bytes(b"private")
        link = tmp_path / "link"
        link.symlink_to(source)
        with pytest.raises(ValueError):
            open_regular(link)
        fifo = tmp_path / "fifo"
        os.mkfifo(fifo)
        with pytest.raises(ValueError):
            open_regular(fifo)

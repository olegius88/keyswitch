"""Explicit opt-in live smoke test; sends only the committed synthetic fixture."""

import copy
import hashlib
import io
import json
import uuid
import zipfile
from pathlib import Path

from logcourier.__main__ import main as cli
from logcourier.catalog import deliver, list_entries, verify_connection
from logcourier.collector import Collector
from logcourier.config import Source, data_directory, load_config, save_config
from logcourier.secrets import read_token
from logcourier.store import Store
from logcourier.telegram import Telegram

ROOT = Path(__file__).resolve().parents[1]


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chat-id", required=True)
    parser.add_argument("--send-synthetic-test", action="store_true", required=True)
    args = parser.parse_args()
    root = data_directory()
    config = load_config(root)
    client = Telegram(read_token(config.bot_id))
    print(verify_connection(client, args.chat_id), flush=True)
    if config.chat_id != args.chat_id:
        config.chat_id = args.chat_id
        config.auto_send = False
        config.consent = False
    config.validate()
    save_config(root, config)
    test = copy.deepcopy(config)
    fixture = ROOT / "tests/fixtures/telegram-smoke.txt"
    test.consent = True
    test.sources = [
        Source(
            str(fixture),
            "SYNTHETIC TEST — не реальные логи",
            0,
            True,
            uuid.uuid5(uuid.NAMESPACE_URL, "logcourier:live-smoke:v1").hex,
        )
    ]
    test_root = ROOT / ".local/live-telegram"
    store = Store(test_root / "state")
    try:
        count, errors = Collector(store).scan(test)
        assert not errors, errors
        print("Synthetic fragments collected:", count, flush=True)
        print(deliver(store, test, client), flush=True)
        entries = list_entries(client, test.chat_id, limit=100)
        expected = [item for item in entries if item["source_id"] == test.sources[0].id]
        assert len(expected) == 1, "Expected one synthetic archive in catalog"
        entry = expected[0]
        data = client.download(entry["file_id"])
        assert len(data) == entry["size"]
        assert hashlib.sha256(data).hexdigest() == entry["sha256"]
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            assert archive.read("fragment.log") == fixture.read_bytes()
        assert cli(["fetch", "--output", str(test_root / "downloads"), "--limit", "1"]) == 0
        assert not store.queue(test.destination)
        report = {
            "passed": True,
            "chat_id": test.chat_id,
            "bundle_id": entry["bundle_id"],
            "message_id": entry["message_id"],
            "sha256": entry["sha256"],
            "synthetic_only": True,
            "archive_bytes": len(data),
            "auto_send": config.auto_send,
            "real_log_consent": config.consent,
            "real_sources": len(config.sources),
        }
        (test_root / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2), flush=True)
    finally:
        store.close()


if __name__ == "__main__":
    main()

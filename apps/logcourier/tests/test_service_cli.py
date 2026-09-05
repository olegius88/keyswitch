import json

from logcourier import __main__, service
from logcourier.collector import Collector
from logcourier.config import save_config
from logcourier.store import QueueFull


def test_full_queue_still_delivers(tmp_path, configured, monkeypatch):
    config, _ = configured
    messages = []
    worker = service.Service(
        tmp_path / "worker", config, "dummy", lambda *args: messages.append(args)
    )
    worker.send_now()

    def full(*args):
        raise QueueFull("Очередь заполнена")

    def sent(*args):
        worker.stop()
        return "Sent"

    monkeypatch.setattr(service.Collector, "scan", full)
    monkeypatch.setattr(service, "Telegram", lambda _: object())
    monkeypatch.setattr(service, "deliver", sent)
    worker.run()
    assert messages and "Sent" in messages[0][0]
    assert "заполнена" in messages[0][0]


def test_service_retry_retains_queue(tmp_path, configured, monkeypatch):
    config, _ = configured
    config.auto_send = True
    messages = []

    def notify(*args):
        messages.append(args)
        worker.stop()

    worker = service.Service(tmp_path / "worker", config, "dummy", notify)
    worker.send_now()
    monkeypatch.setattr(service, "Telegram", lambda _: object())

    def failure(*args):
        raise service.TelegramError("retry", retry_after=60)

    monkeypatch.setattr(service, "deliver", failure)
    worker.run()
    assert worker.failures == 1
    assert worker.retry_at > 0
    assert messages[0][1]["queued"] == 1


def test_cli_fetch_validates_and_skips_existing(
    tmp_path, store, configured, telegram, monkeypatch, capsys
):
    from logcourier.catalog import deliver

    config, _ = configured
    save_config(tmp_path, config)
    monkeypatch.setenv("LOGCOURIER_DATA_DIR", str(tmp_path))
    monkeypatch.setenv("LOGCOURIER_BOT_TOKEN", "not used by fake")
    monkeypatch.setattr(__main__, "Telegram", lambda _: telegram)
    Collector(store).scan(config)
    deliver(store, config, telegram)
    output = tmp_path / "download"
    args = ["fetch", "--output", str(output)]
    assert __main__.main(args) == 0
    assert len(list(output.glob("*.zip"))) == 1
    assert __main__.main(args) == 0
    assert __main__.main(["list"]) == 0
    capsys.readouterr()
    assert __main__.main(["status"]) == 0
    assert json.loads(capsys.readouterr().out)["chat_id"] == config.chat_id
    archive_id = next(key for key, value in telegram.files.items() if value.startswith(b"PK"))
    telegram.files[archive_id] = b"corrupt"
    assert __main__.main(args) == 1
    assert "сумма" in capsys.readouterr().err

import pytest

from logcourier.catalog import (
    DeliveryCancelled,
    current_catalog,
    decode_index,
    deliver,
    list_entries,
    verify_connection,
)
from logcourier.collector import Collector
from logcourier.telegram import TelegramError


def test_delivery_catalog_chain_and_restart(store, configured, telegram):
    config, path = configured
    assert "log_bot" in verify_connection(telegram, config.chat_id)
    for text in (b"first\n", b"second\n"):
        with path.open("ab") as stream:
            stream.write(text)
        Collector(store).scan(config)
        assert "1 архивов" in deliver(store, config, telegram)
    assert len(list_entries(telegram, config.chat_id)) == 2
    assert len(list_entries(telegram, config.chat_id, limit=1)) == 1
    assert store.stats(config.destination)["queued"] == 0
    assert store.stats(config.destination)["unindexed"] == 0
    assert store.queue_bytes() == 0
    uploads = telegram.uploads
    deliver(store, config, telegram)
    assert telegram.uploads == uploads


def test_pin_failure_resumes_without_reupload(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        deliver(store, config, telegram)
    assert store.stats(config.destination)["unindexed"] == 1
    assert telegram.uploads == 2
    telegram.fail_pin = False
    assert "восстановлен" in deliver(store, config, telegram)
    assert telegram.uploads == 2
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_lost_pin_does_not_replace_catalog(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    telegram.pinned = None
    with pytest.raises(TelegramError, match="потеряно"):
        deliver(store, config, telegram)
    assert telegram.uploads == 2


def test_unrelated_pin_is_preserved(telegram):
    telegram.pinned = 99
    telegram.messages[99] = {"from": {"id": 777}, "text": "Important"}
    with pytest.raises(TelegramError, match="другое сообщение"):
        current_catalog(telegram, "-100123")
    assert telegram.pinned == 99


def test_catalog_hash_and_destination_validation(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    head = current_catalog(telegram, config.chat_id)
    with pytest.raises(TelegramError):
        decode_index(telegram.files[head["file_id"]], "-999", config.bot_id)
    telegram.files[head["file_id"]] += b" "
    with pytest.raises(TelegramError, match="сумма"):
        list_entries(telegram, config.chat_id)


def test_cancel_after_upload_preserves_receipt(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    cancelled = []
    telegram.after_upload = lambda: cancelled.append(True)
    with pytest.raises(DeliveryCancelled):
        deliver(store, config, telegram, lambda: bool(cancelled))
    assert telegram.uploads == 1
    assert store.stats(config.destination)["unindexed"] == 1
    telegram.after_upload = lambda: None
    deliver(store, config, telegram)
    assert telegram.uploads == 2


def test_no_consent_or_wrong_bot(store, configured, telegram):
    config, _ = configured
    config.consent = False
    with pytest.raises(ValueError):
        deliver(store, config, telegram)
    config.consent = True
    config.bot_id = "666666"
    with pytest.raises(ValueError):
        deliver(store, config, telegram)
    assert telegram.uploads == 0


@pytest.mark.parametrize("payload", [b"not json", b"[]", b"null", b"{}"])
def test_invalid_catalog(payload):
    with pytest.raises(TelegramError):
        decode_index(payload, "-1", "123456")

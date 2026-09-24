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
from logcourier.store import HEAD_DIGEST_KEY, LEGACY_HEAD_KEY, PENDING_KEY
from logcourier.telegram import TelegramError

# One archive upload plus one catalog upload, regardless of whether the pin
# that follows succeeds.
UPLOADS_PER_DELIVERY = 2
# Two archived files delivered across two scan-and-deliver cycles.
EXPECTED_ENTRIES = 2
SHA256_HEX_LENGTH = 64
# A message id/user id fixture standing in for something outside this
# delivery's own catalog chain (another process's pin, an unrelated sender).
FOREIGN_MESSAGE_ID = 99
FOREIGN_USER_ID = 777


def test_delivery_catalog_chain_and_restart(store, configured, telegram):
    config, path = configured
    assert "log_bot" in verify_connection(telegram, config.chat_id)
    for text in (b"first\n", b"second\n"):
        with path.open("ab") as stream:
            stream.write(text)
        Collector(store).scan(config)
        assert "1 архивов" in deliver(store, config, telegram)
    assert len(list_entries(telegram, config.chat_id)) == EXPECTED_ENTRIES
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
    assert telegram.uploads == UPLOADS_PER_DELIVERY
    telegram.fail_pin = False
    assert "восстановлен" in deliver(store, config, telegram)
    assert telegram.uploads == UPLOADS_PER_DELIVERY
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_lost_pin_does_not_replace_catalog(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    telegram.pinned = None
    with pytest.raises(TelegramError, match="потеряно"):
        deliver(store, config, telegram)
    assert telegram.uploads == UPLOADS_PER_DELIVERY


def test_reissued_file_id_does_not_block_delivery(store, configured, telegram):
    config, path = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    reissued = telegram.reissues
    with path.open("ab") as stream:
        stream.write(b"later\n")
    Collector(store).scan(config)
    assert "1 архивов" in deliver(store, config, telegram)
    assert telegram.reissues > reissued
    assert len(list_entries(telegram, config.chat_id)) == EXPECTED_ENTRIES


def test_head_stored_as_file_id_adopts_the_pinned_catalog(store, configured, telegram):
    config, path = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    store.forget(HEAD_DIGEST_KEY + config.destination)
    store.set(LEGACY_HEAD_KEY + config.destination, "file_id_from_an_older_version")
    with path.open("ab") as stream:
        stream.write(b"later\n")
    Collector(store).scan(config)
    assert "1 архивов" in deliver(store, config, telegram)
    assert store.get(LEGACY_HEAD_KEY + config.destination) is None
    head = current_catalog(telegram, config.chat_id)
    assert store.get(HEAD_DIGEST_KEY + config.destination) == head["sha256"]


def test_head_stored_as_file_id_still_requires_a_pinned_catalog(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    store.forget(HEAD_DIGEST_KEY + config.destination)
    store.set(LEGACY_HEAD_KEY + config.destination, "file_id_from_an_older_version")
    telegram.pinned = None
    with pytest.raises(TelegramError, match="потеряно"):
        deliver(store, config, telegram)
    assert telegram.uploads == UPLOADS_PER_DELIVERY


def test_pending_catalog_from_an_older_version_is_rebuilt(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        deliver(store, config, telegram)
    pending = store.get(PENDING_KEY + config.destination)
    store.set(
        PENDING_KEY + config.destination,
        {
            "file_id": "file_id_from_an_older_version",
            "message_id": pending["message_id"],
            "ids": pending["ids"],
            "previous_file_id": None,
            "previous_message_id": None,
        },
    )
    telegram.fail_pin = False
    uploads = telegram.uploads
    assert "1 архивов" in deliver(store, config, telegram)
    assert telegram.uploads == uploads + 1
    assert store.stats(config.destination)["unindexed"] == 0
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_catalog_unpinned_during_delivery_stops_the_send(store, configured, telegram):
    config, path = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    with path.open("ab") as stream:
        stream.write(b"later\n")
    Collector(store).scan(config)
    telegram.after_upload = lambda: setattr(telegram, "pinned", None)
    with pytest.raises(TelegramError, match="изменился"):
        deliver(store, config, telegram)
    assert store.stats(config.destination)["unindexed"] == 1


def test_pending_catalog_of_another_head_stops_recovery(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    store.set(
        PENDING_KEY + config.destination,
        {
            "sha256": "1" * SHA256_HEX_LENGTH,
            "message_id": FOREIGN_MESSAGE_ID,
            "ids": [],
            "previous_sha256": "0" * SHA256_HEX_LENGTH,
            "previous_message_id": None,
        },
    )
    with pytest.raises(TelegramError, match="другим процессом"):
        deliver(store, config, telegram)


def test_unrelated_pin_is_preserved(telegram):
    telegram.pinned = FOREIGN_MESSAGE_ID
    telegram.messages[FOREIGN_MESSAGE_ID] = {"from": {"id": FOREIGN_USER_ID}, "text": "Important"}
    with pytest.raises(TelegramError, match="другое сообщение"):
        current_catalog(telegram, "-100123")
    assert telegram.pinned == FOREIGN_MESSAGE_ID


def test_catalog_hash_and_destination_validation(store, configured, telegram):
    config, _ = configured
    Collector(store).scan(config)
    deliver(store, config, telegram)
    head = current_catalog(telegram, config.chat_id)
    with pytest.raises(TelegramError):
        decode_index(telegram.content(head["file_id"]), "-999", config.bot_id)
    telegram.replace(head["file_id"], telegram.content(head["file_id"]) + b" ")
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
    assert telegram.uploads == UPLOADS_PER_DELIVERY


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

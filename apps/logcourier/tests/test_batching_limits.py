import io
import json
import time
import zipfile
from types import SimpleNamespace

import pytest

from logcourier.batching import MIN_FRAGMENTS_TO_COMPACT, compact
from logcourier.catalog import DeliveryCancelled, deliver, list_entries
from logcourier.collector import Collector
from logcourier.rate_limit import GROUP_INTERVAL, TELEGRAM_RATE_LIMIT_STATUS, RateLimitedClient
from logcourier.service import Service
from logcourier.store import QueueFull
from logcourier.telegram import TelegramError

# Fixture fragment counts: the default scanned-and-compacted batch, and a
# smaller one for the tests that look at partial batching.
FRAGMENT_COUNT = 32
SMALL_FRAGMENT_COUNT = 5
# One data package upload plus one catalog upload, not one document per fragment.
UPLOADS_PER_DELIVERY = 2
COMPACTION_SIZE_LIMIT_BYTES = 6000
# The size-limited compaction above must still split the fragments into at
# least this many batches.
MIN_COMPACTED_BATCHES = 2

FIXTURE_CLOCK_START = 100.0
# Three mutation calls, each paced GROUP_INTERVAL apart from the fixture clock.
EXPECTED_CALL_TIMES = [
    FIXTURE_CLOCK_START,
    FIXTURE_CLOCK_START + GROUP_INTERVAL,
    FIXTURE_CLOCK_START + GROUP_INTERVAL + GROUP_INTERVAL,
]
# Telegram's reported retry_after, fixture-only.
RETRY_AFTER_SECONDS = 37
# clock() + delay, where delay = max(1, RETRY_AFTER_SECONDS) + 1 (see RateLimitedClient._perform).
EXPECTED_COOLDOWN = 138
# A clock reading for a second pacing group, still short of the first group's cooldown.
OTHER_GROUP_CLOCK = 110.0
# A "next allowed send" timestamp comfortably past the fixture clock, so the
# pacing wait is still in effect when cancellation is checked.
PACING_UNTIL_TIMESTAMP = 200
# An arbitrary retry_at offset, unrelated to RETRY_AFTER_SECONDS above.
RETRY_AT_OFFSET_SECONDS = 37


def add_fragments(store, config, path, count=FRAGMENT_COUNT):
    for _ in range(count):
        with path.open("ab") as stream:
            stream.write("Привет, logs!\n".encode())
        Collector(store).scan(config)


def test_thirty_two_fragments_become_one_byte_exact_archive(store, configured, telegram):
    config, path = configured
    add_fragments(store, config, path)
    before = {row["id"]: row["payload"] for row in store.queue(config.destination)}
    assert compact(store, config) == FRAGMENT_COUNT
    assert compact(store, config) == 0
    rows = store.queue(config.destination)
    assert len(rows) == 1
    with zipfile.ZipFile(io.BytesIO(rows[0]["payload"])) as archive:
        manifest = json.loads(archive.read("manifest.json"))
        assert len(manifest["fragments"]) == FRAGMENT_COUNT
        for entry in manifest["fragments"]:
            assert archive.read(entry["archive"]) == before[entry["bundle_id"]]
    deliver(store, config, telegram)
    assert (
        telegram.uploads == UPLOADS_PER_DELIVERY
    )  # data package + catalog, not one document per fragment
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_compaction_size_and_receipt_isolation(store, configured):
    config, path = configured
    add_fragments(store, config, path, SMALL_FRAGMENT_COUNT)
    first = store.queue(config.destination)[0]
    store.receipt(first["id"], "uploaded", 1)
    assert compact(store, config, limit=COMPACTION_SIZE_LIMIT_BYTES) >= MIN_COMPACTED_BATCHES
    for row in store.queue(config.destination):
        if row["file_id"]:
            assert row["id"] == first["id"]
        else:
            assert len(row["payload"]) <= COMPACTION_SIZE_LIMIT_BYTES


def test_compaction_full_queue_rolls_back(store, configured):
    config, path = configured
    add_fragments(store, config, path, MIN_FRAGMENTS_TO_COMPACT)
    before = [dict(row) for row in store.queue(config.destination)]
    store.capacity = store.queue_bytes()
    with pytest.raises(QueueFull):
        compact(store, config)
    assert [dict(row) for row in store.queue(config.destination)] == before


def test_mutations_including_pin_are_paced_and_persistent(store):
    clock = [FIXTURE_CLOCK_START]
    calls = []
    client = SimpleNamespace(
        bot_id="123456",
        send_document=lambda *args: calls.append(clock[0]),
        call=lambda *args: calls.append(clock[0]),
    )

    def sleep(delay):
        clock[0] += delay

    def make():
        return RateLimitedClient(client, store, "-1", lambda: False, lambda: clock[0], sleep)

    make().send_document("one")
    make().send_document("two")
    make().call("pinChatMessage", {})
    assert calls == EXPECTED_CALL_TIMES


def test_retry_after_is_persisted_and_stops_requests(store):
    calls = []

    def limited(*args):
        calls.append(True)
        raise TelegramError(
            "rate limit", retry_after=RETRY_AFTER_SECONDS, status_code=TELEGRAM_RATE_LIMIT_STATUS
        )

    client = SimpleNamespace(bot_id="123456", send_document=limited, call=limited)
    clock = [FIXTURE_CLOCK_START]
    limited_client = RateLimitedClient(client, store, "-1", lambda: False, lambda: clock[0])
    with pytest.raises(TelegramError) as result:
        limited_client.send_document("one")
    assert result.value.retry_after >= RETRY_AFTER_SECONDS
    assert store.get("telegram_cooldown:123456") == EXPECTED_COOLDOWN
    other_group = RateLimitedClient(client, store, "-2", lambda: False, lambda: OTHER_GROUP_CLOCK)
    with pytest.raises(TelegramError):
        other_group.call("getChat", {})
    assert len(calls) == 1


def test_pacing_can_be_cancelled(store):
    store.set("telegram_next:123456:-1", PACING_UNTIL_TIMESTAMP)
    client = SimpleNamespace(bot_id="123456")
    wrapper = RateLimitedClient(client, store, "-1", lambda: True, lambda: FIXTURE_CLOCK_START)
    with pytest.raises(DeliveryCancelled):
        wrapper.send_document("one")


def test_settings_and_manual_button_do_not_reset_retry(configured, tmp_path):
    config, _ = configured
    worker = Service(tmp_path, config, "", lambda *args: None)
    until = time.monotonic() + RETRY_AT_OFFSET_SECONDS
    worker.retry_at = until
    worker.update(config, "")
    worker.send_now()
    assert worker.retry_at == until

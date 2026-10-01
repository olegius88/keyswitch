import copy
import time
from types import SimpleNamespace

import pytest
from fixture_values.clock import RETRY_AT_OFFSET_SECONDS
from fixture_values.counts import (
    CLEARING_BATCHES,
    CLEARING_NOTICE_MESSAGE_ID,
    FAKE_HEX_ID_LENGTH,
    GROUP_HISTORY_MESSAGES,
    LAST_KEPT_MESSAGE_IDS,
)
from fixture_values.keys import SUPERGROUP_CREATION_MESSAGE_ID, WRONG_CHAT_ID

from logcourier import service
from logcourier.catalog import (
    LOST_PIN,
    DeliveryCancelled,
    current_catalog,
    deliver,
    list_entries,
)
from logcourier.collector import Collector
from logcourier.constants.limits import DELETE_BATCH_MESSAGES, FIRST_REGULAR_MESSAGE_ID
from logcourier.reset import NOTICE, reset_group
from logcourier.russian_text import ARCHIVES, quantity
from logcourier.store import HEAD_DIGEST_KEY, PENDING_KEY, RESET_KEY, Store
from logcourier.telegram import TelegramError


def delivered(store, config, telegram, path):
    with path.open("ab") as stream:
        stream.write(b"more\n")
    Collector(store).scan(config)
    return deliver(store, config, telegram)


def with_history(telegram):
    # Earlier messages of the group, read by nobody: only their identifiers matter.
    telegram.last_message_id = GROUP_HISTORY_MESSAGES


def test_clearing_deletes_everything_and_the_next_delivery_starts_a_new_chain(
    store, configured, telegram
):
    config, path = configured
    delivered(store, config, telegram, path)
    with_history(telegram)
    progress = []
    result = reset_group(store, config, telegram, True, progress=progress.append)
    assert result.startswith(
        f"Группа очищена: удалены все сообщения до №{CLEARING_NOTICE_MESSAGE_ID}"
    )
    assert set(telegram.messages) == {SUPERGROUP_CREATION_MESSAGE_ID}
    assert telegram.deleted == set(range(FIRST_REGULAR_MESSAGE_ID, CLEARING_NOTICE_MESSAGE_ID + 1))
    assert all(SUPERGROUP_CREATION_MESSAGE_ID not in batch for batch in telegram.delete_requests)
    assert len(telegram.delete_requests) == len(progress) == CLEARING_BATCHES
    assert telegram.pinned is None
    for key in (RESET_KEY, HEAD_DIGEST_KEY, PENDING_KEY):
        assert store.get(key + config.destination) is None
    delivered(store, config, telegram, path)
    head = current_catalog(telegram, config.chat_id)
    assert head["index"]["previous"] is None
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_clearing_restarts_a_chain_whose_pin_is_lost(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    telegram.pinned = None
    with pytest.raises(TelegramError, match="потеряно"):
        delivered(store, config, telegram, path)
    assert "Очистить группу и начать заново" in LOST_PIN
    assert "Группа очищена" in reset_group(store, config, telegram, True)
    assert "Отправлено и включено в каталог" in delivered(store, config, telegram, path)
    assert current_catalog(telegram, config.chat_id)["index"]["previous"] is None


def test_a_group_this_collector_never_wrote_to_is_left_alone(store, configured, telegram):
    config, _ = configured
    with pytest.raises(ValueError, match="ещё ничего не отправлял"):
        reset_group(store, config, telegram, True)
    assert telegram.last_message_id == SUPERGROUP_CREATION_MESSAGE_ID
    assert telegram.unpin_all_calls == 0 and not telegram.delete_requests


def test_another_collectors_catalog_is_left_alone(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    pinned = telegram.pinned
    other = copy.deepcopy(config)
    other.device_id = "b" * FAKE_HEX_ID_LENGTH
    with pytest.raises(TelegramError, match="другой сборщик"):
        reset_group(store, other, telegram, True)
    assert telegram.pinned == pinned and telegram.unpin_all_calls == 0


def test_rights_and_group_kind_are_checked_before_anything_changes(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    pinned = telegram.pinned
    other_bot = copy.deepcopy(config)
    other_bot.bot_id = "654321"
    with pytest.raises(ValueError, match="другому боту"):
        reset_group(store, other_bot, telegram, True)
    telegram.chat_type = "group"
    with pytest.raises(TelegramError, match="супергруппе"):
        reset_group(store, config, telegram, True)
    telegram.chat_type = "supergroup"
    telegram.member["can_delete_messages"] = False
    with pytest.raises(TelegramError, match="«Удаление сообщений»"):
        reset_group(store, config, telegram, True)
    telegram.member["can_pin_messages"] = False
    with pytest.raises(TelegramError, match="«Закрепление сообщений»"):
        reset_group(store, config, telegram, False)
    assert telegram.pinned == pinned and telegram.unpin_all_calls == 0
    assert not telegram.delete_requests
    assert telegram.last_message_id == pinned


def test_restart_without_deleting_keeps_archives_already_in_the_group(store, configured, telegram):
    config, path = configured
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        delivered(store, config, telegram, path)
    telegram.fail_pin = False
    assert store.stats(config.destination)["unindexed"] == 1
    telegram.member["can_delete_messages"] = False
    assert "сообщения не удалялись" in reset_group(store, config, telegram, False)
    assert telegram.unpin_all_calls == 1 and not telegram.delete_requests
    assert store.get(PENDING_KEY + config.destination) is None
    assert store.stats(config.destination)["unindexed"] == 1
    deliver(store, config, telegram)
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_clearing_drops_archives_uploaded_but_not_yet_listed(store, configured, telegram):
    config, path = configured
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        delivered(store, config, telegram, path)
    telegram.fail_pin = False
    result = reset_group(store, config, telegram, True)
    assert f"ещё не внесённые в каталог: {quantity(1, ARCHIVES)}." in result
    assert store.stats(config.destination)["unindexed"] == 0
    delivered(store, config, telegram, path)
    assert len(list_entries(telegram, config.chat_id)) == 1


@pytest.mark.parametrize("last_kept", LAST_KEPT_MESSAGE_IDS)
def test_messages_older_than_telegram_deletes_are_named_and_not_asked_for_again(
    store, configured, telegram, last_kept
):
    config, path = configured
    delivered(store, config, telegram, path)
    with_history(telegram)
    telegram.kept |= set(range(FIRST_REGULAR_MESSAGE_ID, last_kept + 1))
    result = reset_group(store, config, telegram, True)
    assert result.startswith(
        f"Группа очищена частично: удалены сообщения с №{last_kept + 1} "
        f"по №{CLEARING_NOTICE_MESSAGE_ID}. Более ранние, до №{last_kept}, Telegram"
    )
    assert telegram.deleted == set(range(last_kept + 1, CLEARING_NOTICE_MESSAGE_ID + 1))
    # Only the batch holding the newest kept message is searched; older ones are not asked for.
    requested = {identifier for batch in telegram.delete_requests for identifier in batch}
    assert min(requested) > last_kept - DELETE_BATCH_MESSAGES
    assert store.get(RESET_KEY + config.destination) is None


def test_an_interrupted_clearing_resumes_without_touching_a_new_chain(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    with_history(telegram)
    with pytest.raises(DeliveryCancelled, match="продолжится"):
        reset_group(store, config, telegram, True, lambda: bool(telegram.delete_requests))
    assert store.get(RESET_KEY + config.destination)["next"] < CLEARING_NOTICE_MESSAGE_ID
    delivered(store, config, telegram, path)
    pinned = telegram.pinned
    assert pinned > CLEARING_NOTICE_MESSAGE_ID
    assert "Группа очищена" in reset_group(store, config, telegram, False)
    assert telegram.unpin_all_calls == 1 and telegram.pinned == pinned
    assert all(identifier <= CLEARING_NOTICE_MESSAGE_ID for identifier in telegram.deleted)
    assert len(list_entries(telegram, config.chat_id)) == 1


def test_failures_keep_the_clearing_for_a_later_attempt(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    telegram.delete_failure = TelegramError("Нет связи с Telegram.")
    with pytest.raises(TelegramError, match="Нет связи"):
        reset_group(store, config, telegram, True)
    state = store.get(RESET_KEY + config.destination)
    assert state["next"] == state["top"]
    telegram.delete_result = False
    with pytest.raises(TelegramError, match="не подтвердил удаление"):
        reset_group(store, config, telegram, True)
    telegram.delete_result = True
    assert "Группа очищена" in reset_group(store, config, telegram, True)
    assert telegram.unpin_all_calls == 1


def test_unconfirmed_notice_or_unpin_changes_nothing_in_the_store(store, configured, telegram):
    config, path = configured
    delivered(store, config, telegram, path)
    digest = store.get(HEAD_DIGEST_KEY + config.destination)
    telegram.notice_chat = WRONG_CHAT_ID
    with pytest.raises(TelegramError, match="не подтвердил сообщение"):
        reset_group(store, config, telegram, True)
    assert telegram.messages[telegram.last_message_id]["text"] == NOTICE
    assert telegram.unpin_all_calls == 0
    telegram.unpin_all_result = False
    with pytest.raises(TelegramError, match="снятие закреплений"):
        reset_group(store, config, telegram, False)
    assert store.get(HEAD_DIGEST_KEY + config.destination) == digest
    assert store.get(RESET_KEY + config.destination) is None


def test_store_knows_whether_the_collector_wrote_to_a_group(store, configured, telegram):
    config, path = configured
    assert not store.delivered(config.destination)
    telegram.fail_pin = True
    with pytest.raises(TelegramError):
        delivered(store, config, telegram, path)
    store.forget(PENDING_KEY + config.destination)
    assert store.delivered(config.destination)


def worker(tmp_path, config, messages, monkeypatch, calls):
    instance = service.Service(tmp_path / "worker", config, "dummy", None)

    def notify(*args):
        messages.append(args)
        instance.stop()

    def reset_group(store, config, client, delete_messages, cancelled, progress):
        progress("Очистка группы: ход")
        calls.append(delete_messages)
        return "Готово"

    instance.notify = notify
    monkeypatch.setattr(service, "Telegram", lambda _: SimpleNamespace(bot_id=config.bot_id))
    monkeypatch.setattr(service, "reset_group", reset_group)
    return instance


def test_service_runs_a_requested_reset_and_sends_soon_after(tmp_path, configured, monkeypatch):
    config, _ = configured
    messages, calls = [], []
    instance = worker(tmp_path, config, messages, monkeypatch, calls)
    instance.reset(True)
    before = instance.next_send
    instance.run()
    assert calls == [True] and instance.reset_request is None
    assert messages[0][0] == "Очистка группы: ход"
    instance.stop_event.clear()
    instance.notify = lambda *args: (messages.append(args), instance.stop())
    instance.run()
    assert messages[-1][0] == "Автоматическая отправка выключена."
    assert instance.next_send < before


def test_service_drops_a_reset_requested_before_the_settings_change(tmp_path, configured):
    config, _ = configured
    instance = service.Service(tmp_path / "worker", config, "dummy", lambda *args: None)
    instance.reset(True)
    instance.update(config, "dummy")
    assert instance.reset_request is None


def test_service_continues_an_unfinished_clearing_only_while_sending(
    tmp_path, configured, monkeypatch
):
    config, _ = configured
    store = Store(tmp_path / "worker")
    store.set(RESET_KEY + config.destination, {"top": 1, "next": 1, "kept": None})
    store.close()
    messages, calls = [], []
    instance = worker(tmp_path, config, messages, monkeypatch, calls)
    instance.run()
    assert "не завершена" in messages[-1][0] and not calls
    instance.stop_event.clear()
    instance.send_now()
    instance.retry_at = time.monotonic() + RETRY_AT_OFFSET_SECONDS
    instance.run()
    assert "продолжится через" in messages[-1][0] and not calls
    instance.stop_event.clear()
    instance.retry_at = 0
    instance.run()
    assert calls == [False] and messages[-1][0] == "Готово"


def test_service_reset_needs_a_token(tmp_path, configured, monkeypatch):
    config, _ = configured
    messages, calls = [], []
    instance = worker(tmp_path, config, messages, monkeypatch, calls)
    instance._token = ""
    instance.reset(False)
    instance.run()
    assert messages[-1][0] == "Настройте токен и группу Telegram." and not calls

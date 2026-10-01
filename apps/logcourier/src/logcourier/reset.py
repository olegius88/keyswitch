"""Clearing the group and starting a new chain of catalogs.

The Bot API gives a bot no way to read a group's history, but in a supergroup message
identifiers run upward from 1. The collector posts one short notice to learn the newest
identifier, takes every pin down and forgets the chain of catalogs it was continuing, then asks
Telegram to delete every message up to the notice, a batch at a time from the newest. The
progress lives in the store: an interrupted clearing goes on from the batch it stopped at, and
since the pins are taken down only once, a chain started in the meantime keeps its pin.

A bot may delete only messages younger than two days (Bot API, deleteMessage), and Telegram
refuses a whole batch when it keeps even one of its messages, deleting none of them; both were
checked against the live Bot API on 2026-10-01, with an administrator bot allowed to delete
messages. Messages are numbered in the order they were sent, so the kept ones are all older than
the deleted ones: batches go from the newest, the first refused batch is searched by halving for
where the deletable part starts, and nothing older is asked for.
"""

from __future__ import annotations

from .catalog import DeliveryCancelled, checkpoint, current_catalog
from .config import Config
from .constants.limits import DELETE_BATCH_MESSAGES, FIRST_REGULAR_MESSAGE_ID, SEARCH_HALVES
from .constants.telegram import BOT_DELETE_WINDOW_HOURS, TELEGRAM_BAD_REQUEST_STATUS
from .russian_text import ARCHIVES, HOURS, quantity
from .store import RESET_KEY, Store
from .telegram import TelegramError

NOTICE = "LogCourier очищает группу и начинает каталог заново."
UNCHANGED = " Группа не изменена."


def reset_group(
    store: Store,
    config: Config,
    client,
    delete_messages: bool,
    cancelled=lambda: False,
    progress=lambda message: None,
) -> str:
    """Start a new chain of catalogs; with `delete_messages`, delete the group's messages first.

    A clearing left unfinished is continued, whatever `delete_messages` says now.
    """
    if client.bot_id != config.bot_id:
        raise ValueError("Токен относится к другому боту.")
    if store.get(RESET_KEY + config.destination) is None:
        restart(store, config, client, delete_messages, cancelled)
        if not delete_messages:
            return (
                "Каталог начнётся заново со следующей отправки. Закрепления в группе сняты, "
                "сообщения не удалялись."
            )
    return clear(store, config, client, cancelled, progress)


def restart(store: Store, config: Config, client, delete_messages: bool, cancelled) -> None:
    checkpoint(cancelled)
    chat = client.call("getChat", {"chat_id": config.chat_id})
    if delete_messages and chat.get("type") != "supergroup":
        raise TelegramError(
            "Удалить все сообщения бот может только в супергруппе. Очистите группу вручную "
            "и начните каталог заново без удаления сообщений." + UNCHANGED
        )
    member = client.call(
        "getChatMember", {"chat_id": config.chat_id, "user_id": int(client.bot_id)}
    )
    if member.get("status") != "administrator" or not member.get("can_pin_messages"):
        raise TelegramError("Дайте боту право администратора «Закрепление сообщений»." + UNCHANGED)
    if delete_messages and not member.get("can_delete_messages"):
        raise TelegramError(
            "Чтобы очистить группу, дайте боту право администратора «Удаление сообщений»."
            + UNCHANGED
        )
    head = current_catalog(client, config.chat_id)
    if head and head["index"]["device_id"] != config.device_id:
        raise TelegramError("Каталог в этой группе ведёт другой сборщик." + UNCHANGED)
    if head is None and not store.delivered(config.destination):
        # Nothing ties this group to the collector: it may be the wrong one, so leave it alone.
        raise ValueError("Этот сборщик ещё ничего не отправлял в эту группу." + UNCHANGED)
    clearing = None
    if delete_messages:
        checkpoint(cancelled)
        notice = client.call(
            "sendMessage",
            {"chat_id": config.chat_id, "text": NOTICE, "disable_notification": True},
        )
        try:
            if int(notice["chat"]["id"]) != int(config.chat_id):
                raise ValueError
            top = int(notice["message_id"])
        except (KeyError, TypeError, ValueError):
            raise TelegramError("Telegram не подтвердил сообщение в выбранной группе.") from None
        clearing = {"top": top, "next": top, "kept": None}
    checkpoint(cancelled)
    if client.call("unpinAllChatMessages", {"chat_id": config.chat_id}) is not True:
        raise TelegramError("Telegram не подтвердил снятие закреплений. Каталог не изменён.")
    store.restart_chain(config.destination, clearing)


def clear(store: Store, config: Config, client, cancelled, progress) -> str:
    key = RESET_KEY + config.destination
    state = store.get(key)
    try:
        while state["next"] >= FIRST_REGULAR_MESSAGE_ID:
            checkpoint(cancelled)
            last = state["next"]
            first = max(FIRST_REGULAR_MESSAGE_ID, last - DELETE_BATCH_MESSAGES + 1)
            start = deleted_from(client, config.chat_id, first, last, cancelled)
            if start > first:
                # Every message before a kept one is older still, so Telegram keeps it as well.
                state = {**state, "next": FIRST_REGULAR_MESSAGE_ID - 1, "kept": start - 1}
            else:
                state = {**state, "next": first - 1}
            store.set(key, state)
            progress(f"Очистка группы: обработаны сообщения с №{start} по №{state['top']}.")
    except DeliveryCancelled:
        raise DeliveryCancelled(
            "Очистка группы прервана; она продолжится с того же места."
        ) from None
    store.forget(key)
    return summary(state)


def deleted_from(client, chat_id: str, first: int, last: int, cancelled) -> int:
    """Delete messages first…last; returns where the part Telegram deleted starts."""
    if delete(client, chat_id, first, last):
        return first
    # The batch as a whole was refused. Deleting from `start` to `last` succeeds exactly when
    # `start` is past the kept messages, so the smallest such `start` is found by halving.
    low, high = first + 1, last + 1
    while low < high:
        checkpoint(cancelled)
        middle = (low + high) // SEARCH_HALVES
        if delete(client, chat_id, middle, last):
            high = middle
        else:
            low = middle + 1
    return low


def delete(client, chat_id: str, first: int, last: int) -> bool:
    try:
        done = client.call(
            "deleteMessages", {"chat_id": chat_id, "message_ids": list(range(first, last + 1))}
        )
    except TelegramError as error:
        if error.status_code != TELEGRAM_BAD_REQUEST_STATUS:
            raise
        return False
    if done is not True:
        raise TelegramError("Telegram не подтвердил удаление сообщений.")
    return True


def summary(state: dict) -> str:
    kept = state["kept"]
    if kept is not None:
        text = (
            f"Группа очищена частично: удалены сообщения с №{kept + 1} по №{state['top']}. "
            f"Более ранние, до №{kept}, Telegram удалить не дал: бот может удалять только "
            f"сообщения, отправленные за последние {quantity(BOT_DELETE_WINDOW_HOURS, HOURS)}. "
            "Их можно удалить вручную."
        )
    else:
        text = (
            f"Группа очищена: удалены все сообщения до №{state['top']}, кроме служебного "
            "о создании группы."
        )
    if state.get("dropped"):
        text += (
            " Вместе с ними удалены архивы, ещё не внесённые в каталог: "
            f"{quantity(state['dropped'], ARCHIVES)}."
        )
    return text + " Новый каталог начнётся со следующей отправки."

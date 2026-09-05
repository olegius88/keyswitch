from __future__ import annotations

import hashlib
import json
import re
import uuid
from datetime import datetime, timezone

from .config import Config
from .store import Store
from .telegram import FILE_ID, Telegram, TelegramError

INDEX_LIMIT = 2 * 1024 * 1024
HEX = re.compile(r"[a-f0-9]{64}")
IDENTIFIER = re.compile(r"[a-f0-9]{32}")


class DeliveryCancelled(RuntimeError):
    pass


def checkpoint(cancelled):
    if cancelled():
        raise DeliveryCancelled("Отправка остановлена. Очередь сохранена.")


def decode_index(data: bytes, chat_id: str, bot_id: str) -> dict:
    try:
        result = json.loads(data)
        if result["schema"] != 1 or result["kind"] != "logcourier.index":
            raise ValueError
        if result["chat_id"] != chat_id or result["bot_id"] != bot_id:
            raise ValueError
        if not IDENTIFIER.fullmatch(result["device_id"]):
            raise ValueError
        entries = result["entries"]
        if not isinstance(entries, list) or len(entries) > 200:
            raise ValueError
        for entry in entries:
            if not IDENTIFIER.fullmatch(entry["bundle_id"]):
                raise ValueError
            if not FILE_ID.fullmatch(entry["file_id"]) or not HEX.fullmatch(entry["sha256"]):
                raise ValueError
            if not 0 < entry["size"] <= 10 * 1024 * 1024:
                raise ValueError
            datetime.fromisoformat(entry["created_at"])
        previous = result["previous"]
        if previous is not None:
            if not FILE_ID.fullmatch(previous["file_id"]) or not HEX.fullmatch(previous["sha256"]):
                raise ValueError
        return result
    except (ValueError, KeyError, TypeError, AttributeError, OverflowError):
        raise TelegramError("Каталог повреждён или относится к другому боту/группе.") from None


def current_catalog(client: Telegram, chat_id: str) -> dict | None:
    chat = client.call("getChat", {"chat_id": chat_id})
    if chat.get("type") not in ("group", "supergroup") or chat.get("username"):
        raise TelegramError("Для личных логов нужна закрытая группа без публичного @имени.")
    message = chat.get("pinned_message")
    if not message:
        return None
    name = message.get("document", {}).get("file_name", "")
    if str(message.get("from", {}).get("id")) != client.bot_id or not re.fullmatch(
        r"lc-index-[a-f0-9]{32}\.json", name
    ):
        raise TelegramError(
            "В группе закреплено другое сообщение. Выберите отдельную группу "
            "или восстановите последнее закрепление каталога LogCourier. Чужие закрепления не изменены."
        )
    file_id = message["document"]["file_id"]
    data = client.download(file_id, INDEX_LIMIT)
    digest = hashlib.sha256(data).hexdigest()
    if message.get("caption") != "LogCourier catalog SHA256 " + digest:
        raise TelegramError("Контрольная сумма каталога не совпала.")
    return {
        "file_id": file_id,
        "sha256": digest,
        "message_id": message["message_id"],
        "index": decode_index(data, chat_id, client.bot_id),
    }


def verify_connection(client: Telegram, chat_id: str) -> str:
    me = client.call("getMe")
    if str(me["id"]) != client.bot_id:
        raise TelegramError("ID бота не совпал с токеном.")
    chat = client.call("getChat", {"chat_id": chat_id})
    if chat.get("type") not in ("group", "supergroup") or chat.get("username"):
        raise TelegramError("Нужна закрытая группа Telegram.")
    member = client.call("getChatMember", {"chat_id": chat_id, "user_id": int(client.bot_id)})
    if member.get("status") != "administrator" or not member.get("can_pin_messages"):
        raise TelegramError("Дайте боту право администратора «Закрепление сообщений».")
    return f"@{me.get('username', client.bot_id)} → {chat.get('title', chat_id)}"


def deliver(store: Store, config: Config, client: Telegram, cancelled=lambda: False) -> str:
    if not config.consent:
        raise ValueError("Отправка текста логов не разрешена.")
    if client.bot_id != config.bot_id:
        raise ValueError("Токен относится к другому боту.")
    key = "catalog_pending:" + config.destination
    checkpoint(cancelled)
    head = current_catalog(client, config.chat_id)
    if head and head["index"]["device_id"] != config.device_id:
        raise TelegramError("Эта группа занята другим сборщиком. Используйте отдельную группу.")
    pending = store.get(key)
    if pending:
        checkpoint(cancelled)
        finish_catalog(store, config, client, pending, head)
        return "Каталог восстановлен; отправленные архивы повторно не загружались."
    expected = store.get("catalog_head:" + config.destination)
    if expected and expected != (head or {}).get("file_id"):
        raise TelegramError(
            "Закрепление каталога потеряно или изменено. Восстановите последнее закрепление LogCourier."
        )
    for row in store.queue(config.destination)[:32]:
        checkpoint(cancelled)
        if row["file_id"]:
            continue
        meta = json.loads(row["meta"])
        message = client.send_document(
            config.chat_id,
            row["name"],
            row["payload"],
            f"LogCourier · {meta['device_name']} · {meta['source_label']}\n"
            f"{meta['created_at']}\nID: {row['id']}",
        )
        store.receipt(row["id"], message["document"]["file_id"], message["message_id"])
    entries = []
    for row in store.queue(config.destination):
        if row["file_id"]:
            entries.append(
                {
                    **json.loads(row["meta"]),
                    "file_id": row["file_id"],
                    "message_id": row["message_id"],
                }
            )
        if len(entries) == 200:
            break
    if not entries:
        return "Новых архивов для отправки нет."
    checkpoint(cancelled)
    # Detect another writer before replacing the catalog pointer.
    latest = current_catalog(client, config.chat_id)
    if (latest or {}).get("file_id") != (head or {}).get("file_id"):
        raise TelegramError("Каталог изменился во время отправки. Повторите позже.")
    index = {
        "schema": 1,
        "kind": "logcourier.index",
        "device_id": config.device_id,
        "chat_id": config.chat_id,
        "bot_id": client.bot_id,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "entries": entries,
        "previous": {"file_id": head["file_id"], "sha256": head["sha256"]} if head else None,
    }
    data = json.dumps(index, ensure_ascii=False).encode()
    if len(data) > INDEX_LIMIT:
        raise TelegramError("Каталог слишком большой; отправка остановлена.")
    name = f"lc-index-{uuid.uuid4().hex}.json"
    checkpoint(cancelled)
    message = client.send_document(
        config.chat_id, name, data, "LogCourier catalog SHA256 " + hashlib.sha256(data).hexdigest()
    )
    pending = {
        "file_id": message["document"]["file_id"],
        "message_id": message["message_id"],
        "ids": [entry["bundle_id"] for entry in entries],
        "previous_file_id": head["file_id"] if head else None,
        "previous_message_id": head["message_id"] if head else None,
    }
    store.set(key, pending)
    checkpoint(cancelled)
    finish_catalog(store, config, client, pending, head)
    return f"Отправлено и включено в каталог: {len(entries)} архивов."


def finish_catalog(
    store: Store, config: Config, client: Telegram, pending: dict, head: dict | None
) -> None:
    current_id = (head or {}).get("file_id")
    if current_id not in (pending["previous_file_id"], pending["file_id"]):
        raise TelegramError(
            "Закреплённый каталог изменён другим процессом. Восстановление остановлено."
        )
    if current_id != pending["file_id"]:
        result = client.call(
            "pinChatMessage",
            {
                "chat_id": config.chat_id,
                "message_id": pending["message_id"],
                "disable_notification": True,
            },
        )
        if result is not True:
            raise TelegramError("Telegram не подтвердил закрепление каталога.")
        fresh = current_catalog(client, config.chat_id)
        if not fresh or fresh["file_id"] != pending["file_id"]:
            raise TelegramError("Новый каталог не виден в закреплении. Проверьте права бота.")
    store.acknowledge_index(config.destination, pending["ids"], pending["file_id"])
    # Do not unpin anything automatically: never disturb another participant's pins.


def list_entries(client: Telegram, chat_id: str, limit: int = 100, pages: int = 100) -> list[dict]:
    head = current_catalog(client, chat_id)
    if not head:
        return []
    index = head["index"]
    device_id = index["device_id"]
    result = []
    seen_files = {head["file_id"]}
    seen_bundles = set()
    for _ in range(pages):
        for entry in reversed(index["entries"]):
            if entry["bundle_id"] not in seen_bundles:
                seen_bundles.add(entry["bundle_id"])
                result.append(entry)
            if len(result) >= limit:
                return result
        previous = index["previous"]
        if not previous:
            return result
        if previous["file_id"] in seen_files:
            raise TelegramError("Циклическая ссылка в каталоге.")
        seen_files.add(previous["file_id"])
        data = client.download(previous["file_id"], INDEX_LIMIT)
        if hashlib.sha256(data).hexdigest() != previous["sha256"]:
            raise TelegramError("Повреждена цепочка каталогов.")
        index = decode_index(data, chat_id, client.bot_id)
        if index["device_id"] != device_id:
            raise TelegramError("Смешаны каталоги разных устройств.")
    raise TelegramError("Достигнут предел страниц каталога. Уменьшите запрашиваемое число файлов.")

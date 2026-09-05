from __future__ import annotations

import json
import re
import urllib.error
import urllib.parse
import urllib.request
import uuid

from .secrets import redact, token_bot_id

FILE_ID = re.compile(r"[A-Za-z0-9_-]{1,512}")
MAX_DOWNLOAD = 10 * 1024 * 1024


class TelegramError(RuntimeError):
    def __init__(self, message: str, retry_after: int = 0):
        super().__init__(redact(message))
        self.retry_after = retry_after


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Token is inside the Telegram URL. Never follow a redirect carrying it elsewhere.
        raise TelegramError("Telegram вернул перенаправление; запрос остановлен.")


class Telegram:
    def __init__(self, token: str, opener=None):
        self.bot_id = token_bot_id(token)
        self._token = token
        self.opener = opener or urllib.request.build_opener(NoRedirect())

    def _open(self, request, limit: int) -> bytes:
        try:
            with self.opener.open(request, timeout=30) as response:
                data = response.read(limit + 1)
                if len(data) > limit:
                    raise TelegramError("Ответ Telegram превышает ограничение размера.")
                return data
        except urllib.error.HTTPError as error:
            try:
                payload = json.loads(error.read(8192))
                description = str(payload.get("description", "Ошибка Telegram"))[:250]
                retry_after = int(payload.get("parameters", {}).get("retry_after", 0))
            except (ValueError, TypeError, AttributeError):
                description, retry_after = "Ошибка Telegram", 0
            raise TelegramError(
                f"Telegram HTTP {error.code}: {description}", min(3600, max(0, retry_after))
            ) from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise TelegramError("Нет связи с Telegram. Архивы останутся в очереди.") from None

    def call(self, method: str, params: dict | None = None) -> dict | list | bool:
        allowed = {
            "getMe",
            "getChat",
            "getChatMember",
            "getUpdates",
            "getWebhookInfo",
            "getFile",
            "pinChatMessage",
            "unpinChatMessage",
        }
        if method not in allowed:
            raise ValueError("Метод Telegram не разрешён.")
        request = urllib.request.Request(
            f"https://api.telegram.org/bot{self._token}/{method}",
            data=json.dumps(params or {}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        return self._decode(self._open(request, 1024 * 1024))

    @staticmethod
    def _decode(data: bytes):
        try:
            payload = json.loads(data)
            if not isinstance(payload, dict) or payload.get("ok") is not True:
                raise ValueError
            return payload["result"]
        except (ValueError, KeyError, TypeError):
            raise TelegramError("Некорректный ответ Telegram.") from None

    def send_document(self, chat_id: str, filename: str, data: bytes, caption: str = "") -> dict:
        if not re.fullmatch(r"[A-Za-z0-9_.-]{1,160}", filename) or len(data) > MAX_DOWNLOAD:
            raise ValueError("Некорректный размер или имя архива.")
        boundary = "LogCourier" + uuid.uuid4().hex
        parts = []
        for name, value in {
            "chat_id": chat_id,
            "caption": caption[:900],
            "disable_notification": "true",
        }.items():
            parts.append(
                f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n'.encode()
                + str(value).encode()
                + b"\r\n"
            )
        parts.append(
            (
                f'--{boundary}\r\nContent-Disposition: form-data; name="document"; '
                f'filename="{filename}"\r\nContent-Type: application/octet-stream\r\n\r\n'
            ).encode()
            + data
            + b"\r\n"
        )
        parts.append(f"--{boundary}--\r\n".encode())
        request = urllib.request.Request(
            f"https://api.telegram.org/bot{self._token}/sendDocument",
            data=b"".join(parts),
            headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
        )
        result = self._decode(self._open(request, 1024 * 1024))
        try:
            if not FILE_ID.fullmatch(result["document"]["file_id"]):
                raise ValueError
            if int(result["chat"]["id"]) != int(chat_id):
                raise ValueError
            int(result["message_id"])
        except (KeyError, ValueError, TypeError):
            raise TelegramError("Telegram не подтвердил файл в выбранной группе.") from None
        return result

    def download(self, file_id: str, limit: int = MAX_DOWNLOAD) -> bytes:
        if not isinstance(file_id, str) or not FILE_ID.fullmatch(file_id):
            raise ValueError("Некорректный file_id.")
        info = self.call("getFile", {"file_id": file_id})
        try:
            path = info["file_path"]
            if not isinstance(path, str) or not re.fullmatch(r"[A-Za-z0-9_./-]+", path):
                raise ValueError
            if path.startswith("/") or ".." in path.split("/"):
                raise ValueError
            if int(info.get("file_size", 0)) > limit:
                raise ValueError
        except (TypeError, KeyError, ValueError):
            raise TelegramError("Некорректный путь или слишком большой файл Telegram.") from None
        request = urllib.request.Request(f"https://api.telegram.org/file/bot{self._token}/{path}")
        return self._open(request, limit)

    def groups(self) -> list[dict]:
        webhook = self.call("getWebhookInfo")
        if webhook.get("url"):
            raise TelegramError(
                "У бота настроен webhook. Введите Chat ID вручную; webhook не изменён."
            )
        # No offset: do not acknowledge or discard updates belonging to another client.
        updates = self.call("getUpdates", {"timeout": 0, "limit": 100})
        found = {}
        for update in updates:
            for key in ("message", "my_chat_member", "channel_post"):
                chat = update.get(key, {}).get("chat", {})
                if chat.get("type") in ("group", "supergroup"):
                    found[str(chat["id"])] = chat
        return list(found.values())

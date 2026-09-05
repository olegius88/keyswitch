from __future__ import annotations

import re

TOKEN_PATTERN = re.compile(r"[1-9][0-9]{4,19}:[A-Za-z0-9_-]{20,200}")
SERVICE = "LogCourier.Telegram"


def token_bot_id(token: str) -> str:
    if TOKEN_PATTERN.fullmatch(token) is None:
        raise ValueError("Некорректный формат токена Telegram.")
    return token.split(":", 1)[0]


def redact(text: str) -> str:
    return TOKEN_PATTERN.sub("<TOKEN>", text)


def secure_backend():
    # Never accept a plaintext fallback or an arbitrary user-installed backend.
    import sys

    if sys.platform == "win32":
        from keyring.backends.Windows import WinVaultKeyring

        return WinVaultKeyring()
    if sys.platform == "darwin":
        from keyring.backends.macOS import Keyring

        return Keyring()
    from keyring.backends.SecretService import Keyring

    return Keyring()


def read_token(bot_id: str) -> str:
    if not bot_id:
        return ""
    try:
        return secure_backend().get_password(SERVICE, bot_id) or ""
    except Exception:
        raise RuntimeError(
            "Системное хранилище секретов недоступно. Введите токен на эту сессию."
        ) from None


def store_token(token: str) -> None:
    bot_id = token_bot_id(token)
    try:
        secure_backend().set_password(SERVICE, bot_id, token)
    except Exception:
        raise RuntimeError(
            "Токен не сохранён: системное хранилище секретов недоступно. "
            "Можно снять флажок сохранения и использовать токен только до выхода."
        ) from None

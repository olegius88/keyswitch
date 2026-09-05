from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime
from pathlib import Path

from . import __version__
from .catalog import list_entries
from .config import data_directory, load_config
from .secrets import read_token, redact
from .telegram import Telegram


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="LogCourier — личный сборщик логов в Telegram")
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command")
    gui = commands.add_parser("gui", help="Открыть настройки и значок в трее")
    gui.add_argument("--minimized", action="store_true")
    commands.add_parser("status", help="Показать настройки назначения без секретов")
    for action in ("list", "fetch"):
        command = commands.add_parser(
            action, help="Каталог Telegram" if action == "list" else "Скачать логи"
        )
        command.add_argument("--chat-id", help="ID группы; иначе из настроек")
        command.add_argument("--bot-id", help="ID бота для системного хранилища")
        command.add_argument("--limit", type=int, default=20)
        if action == "fetch":
            command.add_argument("--output", type=Path, required=True)
            command.add_argument("--since", help="Дата ISO, например 2026-09-05 (UTC)")
    arguments = parser.parse_args(argv)
    try:
        if arguments.command in (None, "gui"):
            from .gui import run

            return run(minimized=getattr(arguments, "minimized", False))
        config = load_config(data_directory())
        if arguments.command == "status":
            print(
                json.dumps(
                    {
                        "version": __version__,
                        "chat_id": config.chat_id,
                        "bot_id": config.bot_id,
                        "auto_send": config.auto_send,
                        "sources": len(config.sources),
                    },
                    ensure_ascii=False,
                )
            )
            return 0
        if not 1 <= arguments.limit <= 10000:
            parser.error("--limit должен быть от 1 до 10000")
        chat_id = arguments.chat_id or config.chat_id
        if not chat_id:
            parser.error("Укажите --chat-id или сохраните группу в настройках")
        # Credentials are deliberately not accepted as command-line arguments.
        token = os.environ.get("LOGCOURIER_BOT_TOKEN") or read_token(
            arguments.bot_id or config.bot_id
        )
        if not token:
            parser.error("Нет токена в системном хранилище или LOGCOURIER_BOT_TOKEN")
        client = Telegram(token)
        entries = list_entries(client, chat_id, arguments.limit)
        if arguments.command == "list":
            print(json.dumps(entries, ensure_ascii=False, indent=2))
            return 0
        if arguments.since:
            since = datetime.fromisoformat(arguments.since).date()
            entries = [
                entry
                for entry in entries
                if datetime.fromisoformat(entry["created_at"]).date() >= since
            ]
        root = arguments.output.expanduser().absolute()
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        for entry in entries:
            data = client.download(entry["file_id"])
            if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Контрольная сумма архива не совпала; файл не сохранён.")
            target = root / f"lc-{entry['bundle_id']}.zip"
            # No overwrites, no archive extraction, no paths from remote metadata.
            if target.exists():
                if (
                    target.is_symlink()
                    or hashlib.sha256(target.read_bytes()).hexdigest() != entry["sha256"]
                ):
                    raise ValueError("Путь результата занят другим файлом; перезапись запрещена.")
                continue
            with target.open("xb") as stream:
                os.chmod(target, 0o600)
                stream.write(data)
            print(target)
        return 0
    except Exception as error:
        if isinstance(error, (ValueError, RuntimeError)):
            print(redact(str(error)), file=sys.stderr)
        else:
            print(f"Операция не выполнена ({type(error).__name__}).", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

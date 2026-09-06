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
from .versions import VERSION


def selection_directory(root: Path, entries: list[dict], scope: str | None):
    """A complete, content-addressed selection never shares a folder with old downloads."""
    if scope is None:
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        return root, None
    manifest = {
        "schema": 1,
        "kind": "logcourier.selection",
        "keyswitch_version": scope,
        "files": [
            {
                "name": f"lc-{entry['bundle_id']}.zip",
                **{
                    key: entry[key]
                    for key in ("bundle_id", "source_id", "keyswitch_version", "sha256", "size")
                },
            }
            for entry in entries
        ],
    }
    data = json.dumps(manifest, sort_keys=True, ensure_ascii=False, indent=2).encode()
    root = root / f"keyswitch-{scope}-{hashlib.sha256(data).hexdigest()[:20]}"
    if root.is_symlink():
        raise ValueError("Папка подборки является ссылкой; сохранение запрещено.")
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    expected = {entry["name"] for entry in manifest["files"]} | {"selection.json"}
    if any(path.name not in expected for path in root.iterdir()):
        raise ValueError("В папке подборки есть посторонние файлы. Выберите другой --output.")
    target = root / "selection.json"
    if target.exists() or target.is_symlink():
        if target.is_symlink() or target.stat().st_size != len(data) or target.read_bytes() != data:
            raise ValueError("Манифест подборки занят другим файлом; перезапись запрещена.")
    return root, data


def main(argv=None, error_handler=None) -> int:
    parser = argparse.ArgumentParser(description="LogCourier — личный сборщик логов в Telegram")
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command")
    gui = commands.add_parser("gui", help="Открыть настройки и значок в трее")
    gui.add_argument("--minimized", action="store_true")
    smoke = commands.add_parser("self-test", help="Проверить GUI на изолированном профиле")
    smoke.add_argument("--output", type=Path, required=True)
    commands.add_parser("status", help="Показать настройки назначения без секретов")
    for action in ("list", "fetch"):
        command = commands.add_parser(
            action, help="Каталог Telegram" if action == "list" else "Скачать логи"
        )
        command.add_argument("--chat-id", help="ID группы; иначе из настроек")
        command.add_argument("--bot-id", help="ID бота для системного хранилища")
        command.add_argument("--limit", type=int, default=20)
        scope = command.add_mutually_exclusive_group()
        scope.add_argument(
            "--keyswitch-version",
            default="current",
            metavar="VERSION",
            help="Версия KeySwitch; по умолчанию current — текущая для каждого источника",
        )
        scope.add_argument(
            "--all-versions",
            dest="keyswitch_version",
            action="store_const",
            const=None,
            help="Вся история, включая старые архивы без версии и другие программы",
        )
        if action == "fetch":
            command.add_argument("--output", type=Path, required=True)
            command.add_argument("--since", help="Дата ISO, например 2026-09-05 (UTC)")
    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "self-test":
            from .gui import smoke_test

            return smoke_test(arguments.output)
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
        if arguments.keyswitch_version not in (None, "current") and not VERSION.fullmatch(
            arguments.keyswitch_version
        ):
            parser.error("--keyswitch-version: ожидается версия, например 0.16.2, или current")
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
        entries = list_entries(
            client, chat_id, arguments.limit, keyswitch_version=arguments.keyswitch_version
        )
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
        if not entries:
            print(
                "Подходящих архивов пока нет; старые версии вместо них не скачивались.",
                file=sys.stderr,
            )
            return 0
        root, selection = selection_directory(
            arguments.output.expanduser().absolute(), entries, arguments.keyswitch_version
        )
        for entry in entries:
            data = client.download(entry["file_id"])
            if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Контрольная сумма архива не совпала; файл не сохранён.")
            target = root / f"lc-{entry['bundle_id']}.zip"
            # No overwrites, no archive extraction, no paths from remote metadata.
            if target.exists() or target.is_symlink():
                if (
                    target.is_symlink()
                    or target.stat().st_size != entry["size"]
                    or hashlib.sha256(target.read_bytes()).hexdigest() != entry["sha256"]
                ):
                    raise ValueError("Путь результата занят другим файлом; перезапись запрещена.")
                continue
            with target.open("xb") as stream:
                os.chmod(target, 0o600)
                stream.write(data)
            print(target)
        if selection is not None:
            target = root / "selection.json"
            if not target.exists():
                with target.open("xb") as stream:
                    os.chmod(target, 0o600)
                    stream.write(selection)
            print(f"Готовая подборка только выбранных версий: {target}", file=sys.stderr)
        return 0
    except Exception as error:
        if error_handler is not None:
            error_handler(error)
        elif isinstance(error, (ValueError, RuntimeError, ImportError)):
            print(redact(str(error)), file=sys.stderr)
        else:
            print(f"Операция не выполнена ({type(error).__name__}).", file=sys.stderr)
        return 1


if __name__ == "__main__":
    if sys.platform == "win32" and sys.stderr is None:
        from .startup import report_error

        raise SystemExit(main(error_handler=report_error))
    raise SystemExit(main())

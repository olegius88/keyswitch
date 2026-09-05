from __future__ import annotations

import json
import os
import re
import sys
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path


def data_directory() -> Path:
    override = os.environ.get("LOGCOURIER_DATA_DIR")
    if override:
        return Path(override).expanduser().absolute()
    if sys.platform == "win32":
        return Path(os.environ.get("LOCALAPPDATA", Path.home())) / "LogCourier"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "logcourier"


@dataclass
class Source:
    path: str
    label: str = "log"
    rotations: int = 5
    include_existing: bool = False
    id: str = field(default_factory=lambda: uuid.uuid4().hex)

    def validate(self) -> None:
        if type(self.include_existing) is not bool or type(self.rotations) is not int:
            raise ValueError("Некорректные типы настроек источника.")
        if not Path(self.path).is_absolute():
            raise ValueError("Выберите абсолютный путь к файлу.")
        if not 0 <= self.rotations <= 20:
            raise ValueError("Число ротаций должно быть от 0 до 20.")
        if not re.fullmatch(r"[a-f0-9]{32}", self.id):
            raise ValueError("Некорректный идентификатор источника.")
        if not self.label.strip() or len(self.label) > 80:
            raise ValueError("Название источника: от 1 до 80 символов.")


@dataclass
class Config:
    device_id: str = field(default_factory=lambda: uuid.uuid4().hex)
    device_name: str = "Мой компьютер"
    chat_id: str = ""
    bot_id: str = ""
    interval_minutes: int = 15
    auto_send: bool = False
    consent: bool = False
    sources: list[Source] = field(default_factory=list)

    @property
    def destination(self) -> str:
        return f"{self.bot_id}:{self.chat_id}"

    def validate(self) -> None:
        if (
            type(self.consent) is not bool
            or type(self.auto_send) is not bool
            or type(self.interval_minutes) is not int
        ):
            raise ValueError("Некорректные типы настроек отправки.")
        if not re.fullmatch(r"[a-f0-9]{32}", self.device_id):
            raise ValueError("Некорректный идентификатор устройства.")
        if self.chat_id and not re.fullmatch(r"-[1-9][0-9]{0,19}", self.chat_id):
            raise ValueError("Chat ID группы должен быть отрицательным числом.")
        if self.bot_id and not re.fullmatch(r"[1-9][0-9]{0,19}", self.bot_id):
            raise ValueError("Некорректный ID бота.")
        if not 1 <= self.interval_minutes <= 1440:
            raise ValueError("Интервал: от 1 до 1440 минут.")
        if not self.device_name.strip() or len(self.device_name) > 80:
            raise ValueError("Название устройства: от 1 до 80 символов.")
        if len(self.sources) > 50:
            raise ValueError("Поддерживается до 50 источников.")
        ids: set[str] = set()
        paths: set[str] = set()
        for source in self.sources:
            source.validate()
            path = os.path.normcase(os.path.abspath(source.path))
            if source.id in ids or path in paths:
                raise ValueError("Один источник указан дважды.")
            ids.add(source.id)
            paths.add(path)


def load_config(root: Path) -> Config:
    path = root / "config.json"
    if not path.exists():
        return Config()
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("Файл настроек слишком большой.")
    data = json.loads(path.read_text(encoding="utf-8"))
    sources = [Source(**item) for item in data.pop("sources", [])]
    result = Config(**data, sources=sources)
    result.validate()
    return result


def save_config(root: Path, config: Config) -> None:
    config.validate()
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = root / "config.json.tmp"
    with temporary.open("w", encoding="utf-8") as stream:
        os.chmod(temporary, 0o600)
        json.dump(asdict(config), stream, ensure_ascii=False, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(root / "config.json")

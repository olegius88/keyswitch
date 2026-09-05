from __future__ import annotations

import hashlib
import io
import json
import os
import stat
import uuid
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from . import __version__
from .config import Config, Source
from .store import Store

CHUNK_BYTES = 2 * 1024 * 1024
ARCHIVE_LIMIT = 10 * 1024 * 1024


def fingerprint(stream, offset: int) -> str:
    stream.seek(max(0, offset - 128))
    return hashlib.sha256(stream.read(min(offset, 128))).hexdigest()


def open_regular(path: Path):
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    if path.is_symlink():
        raise ValueError("Символьные ссылки не собираются.")
    descriptor = os.open(path, flags)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise ValueError("Источник должен быть обычным файлом.")
        return os.fdopen(descriptor, "rb")
    except BaseException:
        os.close(descriptor)
        raise


def make_archive(data: bytes, metadata: dict) -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("metadata.json", json.dumps(metadata, ensure_ascii=False, indent=2))
        archive.writestr("fragment.log", data)
    return output.getvalue()


class Collector:
    def __init__(self, store: Store, chunk_size: int = CHUNK_BYTES):
        self.store = store
        self.chunk_size = chunk_size

    def scan(self, config: Config, max_chunks: int = 16) -> tuple[int, list[str]]:
        if not config.consent:
            return 0, []
        if not config.bot_id or not config.chat_id:
            raise ValueError("Сначала настройте бота и группу.")
        total = 0
        errors = []
        for source in config.sources:
            if total >= max_chunks:
                break
            try:
                total += self.scan_source(source, config, max_chunks - total)
            except (OSError, ValueError) as error:
                # Do not expose paths or OS exception strings containing private paths.
                errors.append(f"{source.label}: источник недоступен ({type(error).__name__}).")
        return total, errors

    def scan_source(self, source: Source, config: Config, budget: int) -> int:
        paths = [Path(f"{source.path}.{n}") for n in range(source.rotations, 0, -1)]
        paths.append(Path(source.path))
        initial = not self.store.db.execute(
            "SELECT 1 FROM initialized WHERE source=?", (source.id,)
        ).fetchone()
        if initial and not source.include_existing:
            # Initial baseline includes rotated files, so old logs cannot leak later.
            with self.store.db:
                for path in paths:
                    try:
                        with open_regular(path) as stream:
                            info = os.fstat(stream.fileno())
                            identity = f"{info.st_dev}:{info.st_ino}"
                            self.store.db.execute(
                                "INSERT OR IGNORE INTO cursors VALUES (?,?,?,?)",
                                (
                                    source.id,
                                    identity,
                                    info.st_size,
                                    fingerprint(stream, info.st_size),
                                ),
                            )
                    except FileNotFoundError:
                        continue
                self.store.db.execute("INSERT INTO initialized VALUES (?)", (source.id,))
            return 0
        with self.store.db:
            self.store.db.execute("INSERT OR IGNORE INTO initialized VALUES (?)", (source.id,))
        count = 0
        found = False
        for path in paths:
            if count >= budget:
                break
            try:
                stream = open_regular(path)
            except FileNotFoundError:
                continue
            with stream:
                found = True
                info = os.fstat(stream.fileno())
                identity = f"{info.st_dev}:{info.st_ino}"
                cursor = self.store.db.execute(
                    "SELECT offset,anchor FROM cursors WHERE source=? AND identity=?",
                    (source.id, identity),
                ).fetchone()
                offset = cursor["offset"] if cursor else 0
                reset = bool(
                    cursor
                    and (offset > info.st_size or fingerprint(stream, offset) != cursor["anchor"])
                )
                if reset:
                    offset = 0
                while offset < info.st_size and count < budget:
                    stream.seek(offset)
                    data = stream.read(min(self.chunk_size, info.st_size - offset))
                    if not data:
                        break
                    # Prefer line boundaries; huge individual lines remain byte-exact fragments.
                    newline = data.rfind(b"\n")
                    if newline >= 0 and newline < len(data) - 1:
                        data = data[: newline + 1]
                    bundle_id = uuid.uuid4().hex
                    now = datetime.now(timezone.utc)
                    metadata = {
                        "schema": 1,
                        "bundle_id": bundle_id,
                        "collector_version": __version__,
                        "device_id": config.device_id,
                        "device_name": config.device_name,
                        "source_id": source.id,
                        "source_label": source.label,
                        "created_at": now.isoformat(),
                        "start_byte": offset,
                        "end_byte": offset + len(data),
                        "source_reset": reset,
                        "ends_with_newline": data.endswith(b"\n"),
                        "raw_sha256": hashlib.sha256(data).hexdigest(),
                    }
                    payload = make_archive(data, metadata)
                    if len(payload) > ARCHIVE_LIMIT:
                        raise ValueError("Архив превышает ограничение размера.")
                    metadata["sha256"] = hashlib.sha256(payload).hexdigest()
                    metadata["size"] = len(payload)
                    name = f"lc-{now:%Y%m%dT%H%M%SZ}-{bundle_id}.zip"
                    new_offset = offset + len(data)
                    self.store.enqueue(
                        source.id,
                        identity,
                        new_offset,
                        fingerprint(stream, new_offset),
                        config.destination,
                        metadata,
                        payload,
                        name,
                    )
                    offset = new_offset
                    reset = False
                    count += 1
        if not found:
            raise FileNotFoundError(source.path)
        return count

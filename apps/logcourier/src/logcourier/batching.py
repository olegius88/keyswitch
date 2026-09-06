"""Coalesce unsent legacy fragments transactionally, without changing their bytes."""

import hashlib
import io
import json
import uuid
import zipfile
from datetime import datetime, timezone

from .store import QueueFull
from .telegram import MAX_DOWNLOAD


def compact(store, config, limit=MAX_DOWNLOAD, max_fragments=4096):
    rows = store.db.execute(
        "SELECT * FROM bundles WHERE destination=? AND indexed=0 AND file_id IS NULL ORDER BY created,id",
        (config.destination,),
    )
    selected = []
    estimated = 1024
    for row in rows:
        meta = json.loads(row["meta"])
        if meta.get("batch"):
            continue
        # ZIP_STORED: payload lengths plus conservative per-entry/manifest overhead.
        cost = len(row["payload"]) + len(row["meta"].encode("utf-8")) + 512
        if estimated + cost > limit or len(selected) >= max_fragments:
            break
        selected.append(row)
        estimated += cost
    if len(selected) < 2:
        return 0
    buffer = io.BytesIO()
    manifest = []
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_STORED) as archive:
        for row in selected:
            meta = json.loads(row["meta"])
            name = f"fragments/{row['id']}.zip"
            archive.writestr(name, row["payload"])
            manifest.append({**meta, "archive": name})
        archive.writestr(
            "manifest.json", json.dumps({"schema": 1, "fragments": manifest}, ensure_ascii=False)
        )
    payload = buffer.getvalue()
    if len(payload) > limit:
        raise ValueError("Пакет превысил допустимый размер; исходная очередь сохранена.")
    identifier = uuid.uuid4().hex
    meta = {
        "schema": 2,
        "batch": True,
        "bundle_id": identifier,
        "device_id": config.device_id,
        "device_name": config.device_name,
        "source_label": f"Пакет из {len(selected)} фрагментов",
        "fragments": len(selected),
        "created_at": datetime.now(timezone.utc).isoformat(),
        "sha256": hashlib.sha256(payload).hexdigest(),
        "size": len(payload),
    }
    with store.db:
        before = sum(len(row["payload"]) for row in selected)
        if store.queue_bytes() - before + len(payload) > store.capacity:
            raise QueueFull("Для упаковки не хватает места в очереди. Фрагменты сохранены.")
        for row in selected:
            result = store.db.execute(
                "DELETE FROM bundles WHERE id=? AND file_id IS NULL AND indexed=0", (row["id"],)
            )
            if result.rowcount != 1:
                raise RuntimeError("Очередь изменилась во время упаковки; упаковка отменена.")
        store.db.execute(
            "INSERT INTO bundles(id,destination,created,name,meta,payload) VALUES (?,?,?,?,?,?)",
            (
                identifier,
                config.destination,
                selected[0]["created"],
                f"lc-batch-{identifier}.zip",
                json.dumps(meta, ensure_ascii=False),
                payload,
            ),
        )
    return len(selected)

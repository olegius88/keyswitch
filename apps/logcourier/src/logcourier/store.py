from __future__ import annotations

import json
import os
import sqlite3
import time
from pathlib import Path

MAX_QUEUE_BYTES = 128 * 1024 * 1024


class QueueFull(RuntimeError):
    pass


class Store:
    def __init__(self, root: Path, capacity: int = MAX_QUEUE_BYTES):
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.capacity = capacity
        self.db = sqlite3.connect(root / "state.sqlite", timeout=10)
        os.chmod(root / "state.sqlite", 0o600)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS cursors(
                source TEXT NOT NULL, identity TEXT NOT NULL, offset INTEGER NOT NULL,
                anchor TEXT NOT NULL, PRIMARY KEY(source, identity));
            CREATE TABLE IF NOT EXISTS initialized(source TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS bundles(
                id TEXT PRIMARY KEY, destination TEXT NOT NULL, created REAL NOT NULL,
                name TEXT NOT NULL, meta TEXT NOT NULL, payload BLOB,
                file_id TEXT, message_id INTEGER, indexed INTEGER NOT NULL DEFAULT 0);
        """)

    def close(self) -> None:
        self.db.close()

    def get(self, key: str, default=None):
        row = self.db.execute("SELECT value FROM kv WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else default

    def set(self, key: str, value) -> None:
        with self.db:
            self.db.execute("INSERT OR REPLACE INTO kv VALUES (?, ?)", (key, json.dumps(value)))

    def queue_bytes(self) -> int:
        return self.db.execute("SELECT COALESCE(SUM(length(payload)), 0) FROM bundles").fetchone()[
            0
        ]

    def queue(self, destination: str):
        return self.db.execute(
            "SELECT * FROM bundles WHERE destination=? AND indexed=0 ORDER BY created,id",
            (destination,),
        ).fetchall()

    def stats(self, destination: str) -> dict:
        return {
            "queued": self.db.execute(
                "SELECT count(*) FROM bundles WHERE destination=? AND file_id IS NULL",
                (destination,),
            ).fetchone()[0],
            "unindexed": self.db.execute(
                "SELECT count(*) FROM bundles WHERE destination=? AND file_id IS NOT NULL AND indexed=0",
                (destination,),
            ).fetchone()[0],
            "other": self.db.execute(
                "SELECT count(*) FROM bundles WHERE destination!=? AND indexed=0", (destination,)
            ).fetchone()[0],
            "queue_bytes": self.queue_bytes(),
            "last_success": self.get("success:" + destination),
        }

    def enqueue(
        self,
        source: str,
        identity: str,
        offset: int,
        anchor: str,
        destination: str,
        metadata: dict,
        payload: bytes,
        name: str,
    ) -> None:
        with self.db:
            if self.queue_bytes() + len(payload) > self.capacity:
                raise QueueFull(
                    "Очередь заполнена. Сбор приостановлен; исходные файлы не удаляются."
                )
            self.db.execute(
                "INSERT INTO bundles(id,destination,created,name,meta,payload) VALUES (?,?,?,?,?,?)",
                (
                    metadata["bundle_id"],
                    destination,
                    time.time(),
                    name,
                    json.dumps(metadata, ensure_ascii=False),
                    payload,
                ),
            )
            self.db.execute(
                "INSERT OR REPLACE INTO cursors VALUES (?,?,?,?)",
                (source, identity, offset, anchor),
            )

    def receipt(self, bundle_id: str, file_id: str, message_id: int) -> None:
        with self.db:
            self.db.execute(
                "UPDATE bundles SET file_id=?,message_id=?,payload=NULL WHERE id=?",
                (file_id, message_id, bundle_id),
            )

    def acknowledge_index(self, destination: str, ids: list[str], head_file_id: str) -> None:
        with self.db:
            self.db.executemany("UPDATE bundles SET indexed=1 WHERE id=?", [(x,) for x in ids])
            self.db.execute("DELETE FROM kv WHERE key=?", ("catalog_pending:" + destination,))
            self.db.execute(
                "INSERT OR REPLACE INTO kv VALUES (?,?)",
                ("success:" + destination, json.dumps(time.time())),
            )
            self.db.execute(
                "INSERT OR REPLACE INTO kv VALUES (?,?)",
                ("catalog_head:" + destination, json.dumps(head_file_id)),
            )
            # Remote linked catalogs preserve older receipts; local history stays bounded.
            self.db.execute(
                "DELETE FROM bundles WHERE indexed=1 AND created<?", (time.time() - 30 * 86400,)
            )

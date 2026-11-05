//! The queue and the read positions, in the same SQLite file the previous version kept.
use ks_cast::To as _;
use std::path::Path;
use std::time::Duration;

use ks_text::{JsonOptions, json_dumps};
use serde_json::{Value, json};
use sqlite::{Connection, OptionalExtension, Row, params};

use crate::config::private_directory;
use crate::constants::{MAX_QUEUE_BYTES, DB_CONNECT_TIMEOUT_SECONDS, BUNDLE_HELD_FOR_CLEARING, RETENTION_DAYS, SECONDS_PER_DAY};
use crate::error::{Error, Result};
use crate::timeutil::now_seconds;

pub const PENDING_KEY: &str = "catalog_pending:";
pub const HEAD_DIGEST_KEY: &str = "catalog_digest:";
/// Written instead of `HEAD_DIGEST_KEY` before 0.1.3: the `file_id` of the pinned catalog.
pub const LEGACY_HEAD_KEY: &str = "catalog_head:";
/// Progress of clearing the group, kept until every message up to the notice has been asked for.
pub const RESET_KEY: &str = "group_reset:";

const CHAIN_KEYS: [&str; 3] = [PENDING_KEY, HEAD_DIGEST_KEY, LEGACY_HEAD_KEY];

#[must_use]
pub fn dumps(value: &Value, ensure_ascii: bool) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii, ..JsonOptions::default() })
}

#[derive(Clone, Debug)]
pub struct Bundle {
    pub id: String,
    pub destination: String,
    pub created: f64,
    pub name: String,
    pub meta: String,
    pub payload: Option<Vec<u8>>,
    pub file_id: Option<String>,
    pub message_id: Option<i64>,
    pub indexed: i64,
}

impl Bundle {
    #[must_use]
    pub fn metadata(&self) -> Value {
        serde_json::from_str(&self.meta).unwrap_or(Value::Null)
    }

    pub(crate) fn from_row(row: &Row<'_>) -> sqlite::Result<Bundle> {
        Ok(Bundle {
            id: row.get("id")?,
            destination: row.get("destination")?,
            created: row.get("created")?,
            name: row.get("name")?,
            meta: row.get("meta")?,
            payload: row.get("payload")?,
            file_id: row.get("file_id")?,
            message_id: row.get("message_id")?,
            indexed: row.get("indexed")?,
        })
    }
}

pub struct Store {
    pub db: Connection,
    pub capacity: i64,
}

impl Store {
    pub fn open(root: &Path) -> Result<Store> {
        Store::with_capacity(root, MAX_QUEUE_BYTES)
    }

    pub fn with_capacity(root: &Path, capacity: i64) -> Result<Store> {
        private_directory(root)?;
        let path = root.join("state.sqlite");
        let db = Connection::open(&path)?;
        db.busy_timeout(Duration::from_secs(DB_CONNECT_TIMEOUT_SECONDS))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cursors(
                source TEXT NOT NULL, identity TEXT NOT NULL, offset INTEGER NOT NULL,
                anchor TEXT NOT NULL, PRIMARY KEY(source, identity));
            CREATE TABLE IF NOT EXISTS initialized(source TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS cursor_context(
                source TEXT NOT NULL, identity TEXT NOT NULL, version TEXT,
                PRIMARY KEY(source, identity));
            CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS bundles(
                id TEXT PRIMARY KEY, destination TEXT NOT NULL, created REAL NOT NULL,
                name TEXT NOT NULL, meta TEXT NOT NULL, payload BLOB,
                file_id TEXT, message_id INTEGER, indexed INTEGER NOT NULL DEFAULT 0);",
        )?;
        Ok(Store { db, capacity })
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let text: Option<String> = self.db.query_row("SELECT value FROM kv WHERE key=?", [key], |row| row.get(0)).optional().ok().flatten();
        text.and_then(|text| serde_json::from_str(&text).ok())
    }

    pub fn get_f64(&self, key: &str) -> f64 {
        self.get(key).and_then(|value| value.as_f64()).unwrap_or(0.0)
    }

    pub fn set(&self, key: &str, value: &Value) -> Result<()> {
        self.db.execute("INSERT OR REPLACE INTO kv VALUES (?, ?)", params![key, dumps(value, true)])?;
        Ok(())
    }

    pub fn forget(&self, key: &str) -> Result<()> {
        self.db.execute("DELETE FROM kv WHERE key=?", [key])?;
        Ok(())
    }

    pub fn queue_bytes(&self) -> Result<i64> {
        Ok(self.db.query_row("SELECT COALESCE(SUM(length(payload)), 0) FROM bundles", [], |row| row.get(0))?)
    }

    pub fn queue(&self, destination: &str) -> Result<Vec<Bundle>> {
        let mut statement = self.db.prepare("SELECT * FROM bundles WHERE destination=? AND indexed=0 ORDER BY created,id")?;
        let rows = statement.query_map([destination], Bundle::from_row)?;
        Ok(rows.collect::<sqlite::Result<_>>()?)
    }

    fn count(&self, sql: &str, destination: &str) -> Result<i64> {
        Ok(self.db.query_row(sql, [destination], |row| row.get(0))?)
    }

    pub fn stats(&self, destination: &str) -> Result<Value> {
        Ok(json!({
            "queued": self.count("SELECT count(*) FROM bundles WHERE destination=? AND file_id IS NULL", destination)?,
            "unindexed": self.count("SELECT count(*) FROM bundles WHERE destination=? AND file_id IS NOT NULL AND indexed=0", destination)?,
            "other": self.count("SELECT count(*) FROM bundles WHERE destination!=? AND indexed=0", destination)?,
            "queue_bytes": self.queue_bytes()?,
            "last_success": self.get(&format!("success:{destination}")),
            "keyswitch_versions": self.get(&format!("keyswitch_versions:{destination}")).unwrap_or_else(|| json!({})),
        }))
    }

    fn insert_bundle(&self, destination: &str, metadata: &Value, payload: &[u8], name: &str) -> Result<()> {
        if self.queue_bytes()? + payload.len().to::<i64>() > self.capacity {
            return Err(Error::QueueFull("Очередь заполнена. Сбор приостановлен; исходные файлы не удаляются.".to_owned()));
        }
        self.db.execute(
            "INSERT INTO bundles(id,destination,created,name,meta,payload) VALUES (?,?,?,?,?,?)",
            params![metadata["bundle_id"].as_str().unwrap_or_default(), destination, now_seconds(), name, dumps(metadata, false), payload],
        )?;
        Ok(())
    }

    pub fn enqueue(&self, source: &str, identity: &str, offset: i64, anchor: &str, destination: &str, metadata: &Value, payload: &[u8], name: &str, version: Option<&str>) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        self.insert_bundle(destination, metadata, payload, name)?;
        self.db.execute("INSERT OR REPLACE INTO cursors VALUES (?,?,?,?)", params![source, identity, offset, anchor])?;
        self.db.execute("INSERT OR REPLACE INTO cursor_context VALUES (?,?,?)", params![source, identity, version])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn enqueue_event(&self, destination: &str, metadata: &Value, payload: &[u8], name: &str, updates: &[(String, Value)]) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        self.insert_bundle(destination, metadata, payload, name)?;
        for (key, value) in updates {
            self.set(key, value)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn receipt(&self, bundle_id: &str, file_id: &str, message_id: i64) -> Result<()> {
        self.db.execute("UPDATE bundles SET file_id=?,message_id=?,payload=NULL WHERE id=?", params![file_id, message_id, bundle_id])?;
        Ok(())
    }

    /// Whether this collector has ever written to the group: a chain or an uploaded archive.
    pub fn delivered(&self, destination: &str) -> Result<bool> {
        if CHAIN_KEYS.iter().any(|key| self.get(&format!("{key}{destination}")).is_some()) {
            return Ok(true);
        }
        let row: Option<i64> = self.db.query_row("SELECT 1 FROM bundles WHERE destination=? AND file_id IS NOT NULL LIMIT 1", [destination], |row| row.get(0)).optional()?;
        Ok(row.is_some())
    }

    /// Forget the chain of catalogs, so the next delivery starts a new one.
    ///
    /// With `clearing`, the group's messages are about to be deleted: archives uploaded but not yet listed in a catalog
    /// are held back from the new chain until `finish_clearing` knows whether their messages went, and the clearing
    /// progress is stored in the same transaction. Returns how many archives were held.
    pub fn restart_chain(&self, destination: &str, clearing: Option<&Value>) -> Result<usize> {
        let transaction = self.db.unchecked_transaction()?;
        for key in CHAIN_KEYS {
            self.db.execute("DELETE FROM kv WHERE key=?", [format!("{key}{destination}")])?;
        }
        let mut held = 0;
        if let Some(clearing) = clearing {
            held = self.db.execute(
                "UPDATE bundles SET indexed=? WHERE destination=? AND file_id IS NOT NULL AND indexed=0",
                params![BUNDLE_HELD_FOR_CLEARING, destination],
            )?;
            self.set(&format!("{RESET_KEY}{destination}"), clearing)?;
        }
        transaction.commit()?;
        Ok(held)
    }

    /// Settle the archives held by `restart_chain` and forget the finished clearing.
    ///
    /// Telegram keeps the messages up to `kept`: an archive among them is still in the group and goes to the new chain's
    /// catalog. The others were deleted with the group and are dropped. Returns how many were dropped.
    pub fn finish_clearing(&self, destination: &str, kept: Option<i64>) -> Result<usize> {
        let transaction = self.db.unchecked_transaction()?;
        if let Some(kept) = kept {
            self.db.execute(
                "UPDATE bundles SET indexed=0 WHERE destination=? AND indexed=? AND message_id<=?",
                params![destination, BUNDLE_HELD_FOR_CLEARING, kept],
            )?;
        }
        let dropped = self.db.execute("UPDATE bundles SET indexed=1 WHERE destination=? AND indexed=?", params![destination, BUNDLE_HELD_FOR_CLEARING])?;
        self.db.execute("DELETE FROM kv WHERE key=?", [format!("{RESET_KEY}{destination}")])?;
        transaction.commit()?;
        Ok(dropped)
    }

    pub fn acknowledge_index(&self, destination: &str, ids: &[String], head_digest: &str) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        for id in ids {
            self.db.execute("UPDATE bundles SET indexed=1 WHERE id=?", [id])?;
        }
        for key in [PENDING_KEY, LEGACY_HEAD_KEY] {
            self.db.execute("DELETE FROM kv WHERE key=?", [format!("{key}{destination}")])?;
        }
        self.set(&format!("success:{destination}"), &json!(now_seconds()))?;
        self.set(&format!("{HEAD_DIGEST_KEY}{destination}"), &json!(head_digest))?;
        // Remote linked catalogs preserve older receipts; local history stays bounded.
        self.db.execute("DELETE FROM bundles WHERE indexed=1 AND created<?", [now_seconds() - RETENTION_DAYS * SECONDS_PER_DAY])?;
        transaction.commit()?;
        Ok(())
    }
}

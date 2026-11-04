//! Privacy-conscious correction history (only corrected word pairs).

use crate::config::home_dir;
use crate::constants::{DEFAULT_HISTORY_LIMIT, HISTORY_CONFIDENCE_DECIMALS};
use crate::logsetup::utc_now;
use crate::pycompat::{decode_text, encoded_text, json_loads, with_suffix, write_text};
use ks_text::{json_dumps, py_int, py_round_ndigits, JsonOptions};
use serde_json::{Map, Number, Value};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// `data_dir()` with the environment given: `KEYSWITCH_DATA_DIR`, else `%LOCALAPPDATA%\KeySwitch` on Windows and
/// `$XDG_DATA_HOME/keyswitch` (`~/.local/share/keyswitch`) elsewhere.
pub fn data_dir_from(windows: bool, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(over) = get("KEYSWITCH_DATA_DIR") {
        return PathBuf::from(over);
    }
    if windows {
        let base = match get("LOCALAPPDATA") {
            Some(local) => PathBuf::from(local),
            None => home.join("AppData").join("Local"),
        };
        return base.join("KeySwitch");
    }
    let base = match get("XDG_DATA_HOME") {
        Some(xdg) => PathBuf::from(xdg),
        None => home.join(".local").join("share"),
    };
    base.join("keyswitch")
}

pub fn data_dir() -> PathBuf {
    data_dir_from(cfg!(windows), &home_dir(), &|name| std::env::var(name).ok())
}

/// One correction. The five fields are those of the file, in its order.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub original: String,
    pub replacement: String,
    pub application: String,
    pub confidence: f64,
}

/// `datetime.now(timezone.utc).isoformat(timespec="seconds")`: `2026-11-04T12:00:00+00:00`.
pub fn utc_timestamp_now() -> String {
    let t = utc_now();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

impl HistoryEntry {
    /// `HistoryEntry.create`: stamped now (UTC), the confidence rounded to two places (halves to even, as `round`).
    pub fn create(original: &str, replacement: &str, application: &str, confidence: f64) -> HistoryEntry {
        HistoryEntry::with_timestamp(utc_timestamp_now(), original, replacement, application, confidence)
    }

    /// `create` with the time given.
    pub fn with_timestamp(
        timestamp: String,
        original: &str,
        replacement: &str,
        application: &str,
        confidence: f64,
    ) -> HistoryEntry {
        HistoryEntry {
            timestamp,
            original: original.to_string(),
            replacement: replacement.to_string(),
            application: application.to_string(),
            confidence: py_round_ndigits(confidence, HISTORY_CONFIDENCE_DECIMALS).unwrap_or(confidence),
        }
    }

    fn as_row(&self) -> Row {
        Row([
            Value::String(self.timestamp.clone()),
            Value::String(self.original.clone()),
            Value::String(self.replacement.clone()),
            Value::String(self.application.clone()),
            Number::from_f64(self.confidence).map(Value::Number).unwrap_or(Value::Null),
        ])
    }
}

/// `history_limit(value)`: the `history.limit` setting as a count of entries.
///
/// The settings file can be edited by hand; a value that is not a number falls back to the default instead of
/// stopping the application at start. A count beyond 64 bits is kept at the largest one.
pub fn history_limit(value: &Value) -> i64 {
    let parsed: Option<i128> = match value {
        Value::Number(n) => match n.as_i64() {
            Some(i) => Some(i128::from(i)),
            None => match n.as_u64() {
                Some(u) => Some(i128::from(u)),
                // int(float): the fraction is cut off; an infinity or a NaN has no int.
                None => n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i128),
            },
        },
        Value::String(text) => py_int(text),
        _ => None,
    };
    match parsed {
        Some(count) => count.clamp(1, i128::from(i64::MAX)) as i64,
        None => DEFAULT_HISTORY_LIMIT,
    }
}

/// The five values of a file line, whatever their types (Python's dataclass does not check them).
#[derive(Clone, Debug)]
struct Row([Value; 5]);

const FIELDS: [&str; 5] = ["timestamp", "original", "replacement", "application", "confidence"];

impl Row {
    fn from_line(line: &str) -> Option<Row> {
        let Value::Object(payload) = json_loads(line)? else { return None };
        // HistoryEntry(**payload): exactly the five fields, by name, in any order.
        if payload.len() != FIELDS.len() {
            return None;
        }
        let mut values: [Value; 5] = Default::default();
        for (slot, name) in values.iter_mut().zip(FIELDS) {
            *slot = payload.get(name)?.clone();
        }
        Some(Row(values))
    }

    fn line(&self) -> String {
        let mut map = Map::new();
        for (name, value) in FIELDS.iter().zip(&self.0) {
            map.insert((*name).to_string(), value.clone());
        }
        let options = JsonOptions { ensure_ascii: false, ..JsonOptions::default() };
        json_dumps(&Value::Object(map), &options) + "\n"
    }

    /// The typed entry, when the fields have the types the program writes.
    fn entry(&self) -> Option<HistoryEntry> {
        let [timestamp, original, replacement, application, confidence] = &self.0;
        Some(HistoryEntry {
            timestamp: timestamp.as_str()?.to_string(),
            original: original.as_str()?.to_string(),
            replacement: replacement.as_str()?.to_string(),
            application: application.as_str()?.to_string(),
            confidence: confidence.as_f64()?,
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub type HistoryCallback = Arc<dyn Fn() + Send + Sync>;

pub struct HistoryStore {
    pub path: PathBuf,
    limit: Mutex<i64>,
    lock: Mutex<()>,
    callbacks: Mutex<Vec<HistoryCallback>>,
}

impl HistoryStore {
    /// `HistoryStore(path, limit)`: at least one entry is kept.
    pub fn new(path: Option<PathBuf>, limit: i64) -> HistoryStore {
        HistoryStore {
            path: path.unwrap_or_else(|| data_dir().join("history.jsonl")),
            limit: Mutex::new(limit.max(1)),
            lock: Mutex::new(()),
            callbacks: Mutex::new(Vec::new()),
        }
    }

    /// `store.limit`.
    pub fn limit(&self) -> i64 {
        *lock(&self.limit)
    }

    /// `store.limit = value`, as the settings window changes it.
    pub fn set_limit(&self, limit: i64) {
        *lock(&self.limit) = limit;
    }

    fn callbacks(&self) -> Vec<HistoryCallback> {
        lock(&self.callbacks).clone()
    }

    /// Adds the entry; once the history is over its limit the oldest entries go. An error is returned where Python
    /// raises: the write, the rewrite, or a history file that is not UTF-8 (`UnicodeDecodeError`).
    pub fn append(&self, entry: &HistoryEntry) -> io::Result<()> {
        let callbacks;
        {
            let _guard = lock(&self.lock);
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut handle = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
            handle.write_all(&encoded_text(&entry.as_row().line()))?;
            drop(handle);
            let limit = self.limit();
            let rows = self.read_rows(Some(limit.saturating_add(1)))?;
            if rows.len() as i64 > limit {
                self.rewrite(&rows[rows.len() - limit as usize..])?;
            }
            callbacks = self.callbacks();
        }
        for callback in callbacks {
            callback();
        }
        Ok(())
    }

    fn read_rows(&self, limit: Option<i64>) -> io::Result<Vec<Row>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(Vec::new()),
        };
        let Some(text) = decode_text(&bytes, false) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the history file is not valid UTF-8"));
        };
        let mut rows: Vec<Row> = ks_text::pystr::splitlines(&text, false).into_iter().filter_map(Row::from_line).collect();
        if let Some(limit) = limit {
            // result[-limit:]: a limit of zero is "-0", the whole list.
            if limit > 0 && (limit as usize) < rows.len() {
                rows.drain(..rows.len() - limit as usize);
            }
        }
        Ok(rows)
    }

    /// The entries, oldest first, at most the last `limit` of them (`None` and `Some(0)` give all, as `result[-0:]`
    /// does). Rows whose fields do not have the types the program writes are not in the typed list (Python hands them
    /// out as they are); they stay in the file and in the count.
    pub fn read(&self, limit: Option<i64>) -> io::Result<Vec<HistoryEntry>> {
        let _guard = lock(&self.lock);
        Ok(self.read_rows(limit)?.iter().filter_map(Row::entry).collect())
    }

    fn rewrite(&self, rows: &[Row]) -> io::Result<()> {
        // Written aside and swapped in, as the settings and rules are: once the history is full every correction
        // rewrites it, and a crash or a full disk in the middle of an in-place write would leave it empty.
        let temporary = with_suffix(&self.path, ".jsonl.tmp");
        let text: String = rows.iter().map(Row::line).collect();
        write_text(&temporary, &text)?;
        std::fs::rename(&temporary, &self.path)
    }

    pub fn clear(&self) -> io::Result<()> {
        let callbacks;
        {
            let _guard = lock(&self.lock);
            if self.path.exists() {
                write_text(&self.path, "")?;
            }
            callbacks = self.callbacks();
        }
        for callback in callbacks {
            callback();
        }
        Ok(())
    }

    pub fn subscribe(&self, callback: HistoryCallback) {
        let _guard = lock(&self.lock);
        lock(&self.callbacks).push(callback);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn paths() {
        let env = |name: &str| (name == "XDG_DATA_HOME").then(|| "/tmp/xdg-data".to_string());
        assert_eq!(data_dir_from(false, Path::new("/h"), &env), Path::new("/tmp/xdg-data/keyswitch"));
        assert_eq!(data_dir_from(false, Path::new("/h"), &|_| None), Path::new("/h/.local/share/keyswitch"));
        let local = |name: &str| (name == "LOCALAPPDATA").then(|| "C:/L".to_string());
        assert_eq!(data_dir_from(true, Path::new("/h"), &local), Path::new("C:/L/KeySwitch"));
    }

    #[test]
    fn entries_rounding_trimming_callbacks_and_invalid_rows() {
        let path = directory("trim").join("history.jsonl");
        let store = HistoryStore::new(Some(path.clone()), 2);
        let calls = Arc::new(Mutex::new(0));
        let counter = calls.clone();
        store.subscribe(Arc::new(move || *counter.lock().unwrap() += 1));
        for index in 0..3 {
            let entry = HistoryEntry::with_timestamp(
                "2026-11-04T12:00:00+00:00".into(),
                &format!("bad-{index}"),
                &format!("good-{index}"),
                "editor",
                f64::from(index) + 0.125,
            );
            store.append(&entry).unwrap();
        }
        let read = store.read(None).unwrap();
        assert_eq!(read.iter().map(|e| e.original.as_str()).collect::<Vec<_>>(), ["bad-1", "bad-2"]);
        // 2.125 is exactly between 2.12 and 2.13 in binary: round() takes the even one.
        assert_eq!(store.read(Some(1)).unwrap()[0].confidence, 2.12);
        assert_eq!(*calls.lock().unwrap(), 3);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.lines().next().unwrap(),
            r#"{"timestamp": "2026-11-04T12:00:00+00:00", "original": "bad-1", "replacement": "good-1", "application": "editor", "confidence": 1.12}"#
        );
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"not json\n{\"unexpected\": true}\n").unwrap();
        assert_eq!(store.read(None).unwrap().len(), 2);
        store.clear().unwrap();
        assert!(store.read(None).unwrap().is_empty());
        store.clear().unwrap();
        std::fs::remove_file(&path).unwrap();
        store.clear().unwrap();
        assert_eq!(*calls.lock().unwrap(), 6);
    }

    #[test]
    fn hand_written_rows_survive_a_rewrite_as_they_are() {
        let path = directory("raw").join("history.jsonl");
        std::fs::write(&path, "{\"confidence\": 1, \"timestamp\": 5, \"original\": \"a\", \"replacement\": \"b\", \"application\": null}\n").unwrap();
        let store = HistoryStore::new(Some(path.clone()), 1);
        assert!(store.read(None).unwrap().is_empty());
        store.append(&HistoryEntry::with_timestamp("t".into(), "x", "y", "z", 0.5)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\"timestamp\": \"t\", \"original\": \"x\", \"replacement\": \"y\", \"application\": \"z\", \"confidence\": 0.5}\n");
    }

    #[test]
    fn limits() {
        assert_eq!(history_limit(&json!("abc")), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&Value::Null), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!(true)), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!([200])), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!("200")), 200);
        assert_eq!(history_limit(&json!(" 12 ")), 12);
        assert_eq!(history_limit(&json!(0)), 1);
        assert_eq!(history_limit(&json!(7.9)), 7);
        assert_eq!(history_limit(&json!(-3.5)), 1);
        let store = HistoryStore::new(Some(directory("min").join("missing.jsonl")), 0);
        assert_eq!(store.limit(), 1);
        assert!(store.read(None).unwrap().is_empty());
    }

    #[test]
    fn timestamp_shape() {
        let stamp = utc_timestamp_now();
        assert_eq!(stamp.len(), 25);
        assert!(stamp.ends_with("+00:00") && stamp.as_bytes()[10] == b'T');
    }
}

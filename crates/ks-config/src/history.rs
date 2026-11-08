//! Privacy-conscious correction history (only corrected word pairs).

use ks_cast::To as _;
use crate::config::home_dir;
use crate::constants::HISTORY_CONFIDENCE_DECIMALS;
use crate::logsetup::utc_now;
use crate::pycompat::create_parent;
use crate::pycompat::{decode_text, encoded_text, with_suffix, write_text};
pub use crate::settings_file::history_limit;
use ks_text::json::{Kept, ObjectOf, dumps, loads_lenient};
use ks_text::{JsonOptions, py_round_ndigits};
use serde::{Deserialize, Serialize};
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

#[must_use]
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
#[must_use]
pub fn utc_timestamp_now() -> String {
    let t = utc_now();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

impl HistoryEntry {
    /// `HistoryEntry.create`: stamped now (UTC), the confidence rounded to two places (halves to even, as `round`).
    #[must_use]
    pub fn create(original: &str, replacement: &str, application: &str, confidence: f64) -> HistoryEntry {
        HistoryEntry::with_timestamp(utc_timestamp_now(), original, replacement, application, confidence)
    }

    /// `create` with the time given.
    #[must_use]
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
        Row {
            timestamp: Kept::text(self.timestamp.clone()),
            original: Kept::text(self.original.clone()),
            replacement: Kept::text(self.replacement.clone()),
            application: Kept::text(self.application.clone()),
            confidence: Kept::float(self.confidence),
        }
    }
}

/// The five values of a file line, whatever their types (Python's dataclass does not check them): a hand-written row
/// goes back into the file as it came.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    timestamp: Kept,
    original: Kept,
    replacement: Kept,
    application: Kept,
    confidence: Kept,
}

impl Row {
    /// `HistoryEntry(**payload)`: exactly the five fields, by name, in any order, or no row at all.
    fn from_line(line: &str) -> Option<Row> {
        loads_lenient::<ObjectOf<Row>>(line).ok().map(|row| row.0)
    }

    fn line(&self) -> String {
        dumps(self, &JsonOptions { ensure_ascii: false, ..JsonOptions::default() }) + "\n"
    }

    /// The typed entry, when the fields have the types the program writes.
    fn entry(&self) -> Option<HistoryEntry> {
        Some(HistoryEntry {
            timestamp: self.timestamp.as_str()?.to_string(),
            original: self.original.as_str()?.to_string(),
            replacement: self.replacement.as_str()?.to_string(),
            application: self.application.as_str()?.to_string(),
            confidence: self.confidence.as_f64()?,
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    #[must_use]
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
            create_parent(&self.path)?;
            let mut handle = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
            handle.write_all(&encoded_text(&entry.as_row().line()))?;
            drop(handle);
            let limit = self.limit();
            let rows = self.read_rows(Some(limit.saturating_add(1)))?;
            if rows.len().to::<i64>() > limit {
                self.rewrite(&rows[rows.len() - limit.to::<usize>()..])?;
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
            if limit > 0 && limit.to::<usize>() < rows.len() {
                rows.drain(..rows.len() - limit.to::<usize>());
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn an_entry_made_now_and_a_folder_in_place_of_the_file() {
        let entry = HistoryEntry::create("a", "b", "app", 0.5);
        assert_eq!((entry.original.as_str(), entry.replacement.as_str()), ("a", "b"));
        let folder = std::env::temp_dir().join(format!("ks-history-folder-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert!(HistoryStore::new(Some(folder), 10).read(None).unwrap().is_empty());
    }

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
        let text = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        assert_eq!(text, "{\"timestamp\": \"t\", \"original\": \"x\", \"replacement\": \"y\", \"application\": \"z\", \"confidence\": 0.5}\n");
    }

    #[test]
    fn a_line_is_a_row_only_with_exactly_the_five_fields_of_an_object() {
        let row = |text: &str| Row::from_line(text);
        let whole = r#"{"timestamp": "t", "original": "a", "replacement": "b", "application": "c", "confidence": 1}"#;
        assert_eq!(row(whole).unwrap().entry().unwrap().confidence, 1.0);
        // A key given twice keeps its last value, as a dict does; the fields can come in any order.
        let doubled = r#"{"confidence": 0.5, "original": "x", "replacement": "b", "application": "c", "original": "a", "timestamp": "t"}"#;
        assert_eq!(row(doubled).unwrap().entry().unwrap().original, "a");
        assert!(row(r#"{"timestamp": "t", "original": "a", "replacement": "b", "application": "c"}"#).is_none());
        assert!(row(r#"{"timestamp": "t", "original": "a", "replacement": "b", "application": "c", "confidence": 1, "more": 2}"#).is_none());
        assert!(row(r#"["t", "a", "b", "c", 1]"#).is_none());
        // Fields of other types make a row of the file, but no entry; each field type is checked.
        for odd in [
            r#"{"timestamp": 1, "original": "a", "replacement": "b", "application": "c", "confidence": 1}"#,
            r#"{"timestamp": "t", "original": 2, "replacement": "b", "application": "c", "confidence": 1}"#,
            r#"{"timestamp": "t", "original": "a", "replacement": [3], "application": "c", "confidence": 1}"#,
            r#"{"timestamp": "t", "original": "a", "replacement": "b", "application": null, "confidence": 1}"#,
            r#"{"timestamp": "t", "original": "a", "replacement": "b", "application": "c", "confidence": "1"}"#,
        ] {
            assert!(row(odd).unwrap().entry().is_none(), "{odd}");
        }
    }

    #[test]
    fn at_least_one_entry_is_kept() {
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

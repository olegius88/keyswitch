//! What the hints remember between runs (`hints.json` in the data folder), and how a start is told apart: a new
//! installation, an update, or one more run of the same version.
//!
//! Every field is typed and read leniently: a field of another type reads as missing and is written back as this
//! version has it. Two things are kept as the JSON text they were read as, uninterpreted, so that a downgrade does not
//! lose what a newer version wrote: the top-level keys this version does not know, and every record under `seen`
//! (a hint is seen when its id is there, whatever the record holds).

use ks_config::config::set_aside_unreadable;
use ks_config::constants::{HINTS_FILE_NAME, HISTORY_FILE_NAME, LEARNING_FILE_NAME, PERSONAL_WORDS_FILE_NAME};
use ks_config::pycompat::{create_parent, decode_text, with_suffix, write_text};
use ks_updates::SemanticVersion;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::catalogue::Counted;
use crate::constants::HINTS_SCHEMA_VERSION;

/// JSON text kept as it was read.
type Raw = Box<RawValue>;

/// How a shown hint went away; `Shown` until it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Shown,
    /// «Понятно».
    Ok,
    Timeout,
    /// «Больше не показывать подсказки».
    Forever,
    /// Never shown: the user found it out first.
    Skipped,
    /// A click on its text.
    Click,
}

impl How {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            How::Shown => "shown",
            How::Ok => "ok",
            How::Timeout => "timeout",
            How::Forever => "forever",
            How::Skipped => "skipped",
            How::Click => "click",
        }
    }
}

/// A value of another type than the field's: read as the field's default.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Read(T),
    Other(IgnoredAny),
}

fn lenient<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(match Lenient::deserialize(deserializer)? {
        Lenient::Read(value) => value,
        Lenient::Other(IgnoredAny) => T::default(),
    })
}

/// The corrections counted while a hint waits for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    #[serde(default, deserialize_with = "lenient")]
    pub automatic_corrections: i64,
    #[serde(default, deserialize_with = "lenient")]
    pub manual_conversions: i64,
}

/// The counters a correction adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    AutomaticCorrections,
    ManualConversions,
}

impl Counters {
    fn slot(&mut self, counter: Counter) -> &mut i64 {
        match counter {
            Counter::AutomaticCorrections => &mut self.automatic_corrections,
            Counter::ManualConversions => &mut self.manual_conversions,
        }
    }
}

/// The hints shown on one calendar day.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Day {
    #[serde(default, deserialize_with = "lenient")]
    date: String,
    #[serde(default, deserialize_with = "lenient")]
    shown: i64,
}

/// The last update: the version it came from (none for one before the hints) and the one it went to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Update {
    #[serde(default, deserialize_with = "lenient")]
    from: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    to: Option<String>,
}

/// A hint seen: in which version, when, and how it went away.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    #[serde(default, deserialize_with = "lenient")]
    version: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    at: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    how: Option<String>,
}

/// The file as it is written: `schema_version` first, then the fields of this version, then the keys of another.
#[derive(Serialize)]
struct Written<'a> {
    schema_version: i64,
    last_run_version: &'a Option<String>,
    first_run_at: &'a Option<String>,
    fresh_install: bool,
    update: &'a Option<Update>,
    counters: &'a Counters,
    day: &'a Option<Day>,
    seen: &'a BTreeMap<String, Raw>,
    #[serde(flatten)]
    other: &'a BTreeMap<String, Raw>,
}

/// A field of the file, or its default when it holds something else.
fn field<T: DeserializeOwned + Default>(raw: &RawValue) -> T {
    // ks_text::json
    serde_json::from_str(raw.get()).unwrap_or_default()
}

fn version_of(text: Option<&str>) -> Option<SemanticVersion> {
    text.and_then(|text| SemanticVersion::parse(text).ok())
}

/// The contents of `hints.json`.
#[derive(Clone, Debug, Default)]
pub struct HintState {
    last_run_version: Option<String>,
    first_run_at: Option<String>,
    fresh_install: bool,
    update: Option<Update>,
    counters: Counters,
    day: Option<Day>,
    seen: BTreeMap<String, Raw>,
    other: BTreeMap<String, Raw>,
}

impl HintState {
    /// The state a file's text holds; None for text that is no JSON object.
    #[must_use]
    pub fn parse(text: &str) -> Option<HintState> {
        // ks_text::json
        let fields: BTreeMap<String, Raw> = serde_json::from_str(text).ok()?;
        let mut state = HintState::default();
        for (key, raw) in fields {
            match key.as_str() {
                // The file is written with this version's schema; a newer one's is read as far as it is understood.
                "schema_version" => {}
                "last_run_version" => state.last_run_version = field(&raw),
                "first_run_at" => state.first_run_at = field(&raw),
                "fresh_install" => state.fresh_install = field(&raw),
                "update" => state.update = field(&raw),
                "counters" => state.counters = field(&raw),
                "day" => state.day = field(&raw),
                "seen" => state.seen = field(&raw),
                _ => {
                    state.other.insert(key, raw);
                }
            }
        }
        Some(state)
    }

    /// The text of the file, indented by two spaces.
    #[must_use]
    pub fn text(&self) -> String {
        let written = Written {
            schema_version: HINTS_SCHEMA_VERSION,
            last_run_version: &self.last_run_version,
            first_run_at: &self.first_run_at,
            fresh_install: self.fresh_install,
            update: &self.update,
            counters: &self.counters,
            day: &self.day,
            seen: &self.seen,
            other: &self.other,
        };
        // ks_text::json
        let mut text = serde_json::to_string_pretty(&written).expect("strings, numbers and JSON text serialize");
        text.push('\n');
        text
    }

    /// The version that ran last, if the file names one that parses.
    #[must_use]
    pub fn last_run_version(&self) -> Option<SemanticVersion> {
        version_of(self.last_run_version.as_deref())
    }

    /// What a start writes before anything is shown, so that a crash does not bring the news of an update again.
    pub fn begin(&mut self, startup: Startup, current: SemanticVersion, timestamp: &str) {
        self.last_run_version = Some(current.to_string());
        if self.first_run_at.is_none() {
            self.first_run_at = Some(timestamp.to_owned());
        }
        match startup {
            Startup::Fresh => self.fresh_install = true,
            Startup::Updated { from } => {
                self.update = Some(Update { from: from.map(|from| from.to_string()), to: Some(current.to_string()) });
            }
            Startup::Same => {}
        }
    }

    /// Whether these hints began on a new installation.
    #[must_use]
    pub fn fresh_install(&self) -> bool {
        self.fresh_install
    }

    /// The last update: the version it came from (None for one older than the hints) and the one it went to.
    #[must_use]
    pub fn update(&self) -> Option<(Option<SemanticVersion>, SemanticVersion)> {
        let update = self.update.as_ref()?;
        let to = version_of(update.to.as_deref())?;
        Some((version_of(update.from.as_deref()), to))
    }

    #[must_use]
    pub fn counted(&self) -> Counted {
        Counted {
            automatic_corrections: self.counter(Counter::AutomaticCorrections),
            manual_conversions: self.counter(Counter::ManualConversions),
        }
    }

    /// A counter; a negative one, which no version writes, reads as none.
    #[must_use]
    pub fn counter(&self, counter: Counter) -> i64 {
        let mut counters = self.counters;
        let count = *counters.slot(counter);
        count.max(0)
    }

    pub fn count(&mut self, counter: Counter) {
        let count = self.counter(counter).saturating_add(1);
        *self.counters.slot(counter) = count;
    }

    /// How many hints were shown on `date` (the first characters of a timestamp).
    #[must_use]
    pub fn shown_on(&self, date: &str) -> i64 {
        self.day.as_ref().filter(|day| day.date == date).map_or(0, |day| day.shown.max(0))
    }

    pub fn count_shown(&mut self, date: &str) {
        let shown = self.shown_on(date).saturating_add(1);
        self.day = Some(Day { date: date.to_owned(), shown });
    }

    #[must_use]
    pub fn is_seen(&self, id: &str) -> bool {
        self.seen.contains_key(id)
    }

    fn record(&self, id: &str) -> Option<Seen> {
        self.seen.get(id).map(|raw| field(raw))
    }

    /// The version a hint was seen in, if its record names one.
    #[must_use]
    pub fn seen_in(&self, id: &str) -> Option<String> {
        self.record(id).and_then(|record| record.version)
    }

    fn put(&mut self, id: &str, record: &Seen) {
        // ks_text::json
        let raw = serde_json::value::to_raw_value(record).expect("a record of strings serializes");
        self.seen.insert(id.to_owned(), raw);
    }

    pub fn see(&mut self, id: &str, version: SemanticVersion, timestamp: &str, how: How) {
        let record = Seen { version: Some(version.to_string()), at: Some(timestamp.to_owned()), how: Some(how.name().to_owned()) };
        self.put(id, &record);
    }

    /// How a hint seen went away; nothing for one not seen.
    pub fn set_how(&mut self, id: &str, how: How) {
        if let Some(mut record) = self.record(id) {
            record.how = Some(how.name().to_owned());
            self.put(id, &record);
        }
    }

    /// How a seen hint went away, as its record says.
    #[must_use]
    pub fn how(&self, id: &str) -> Option<String> {
        self.record(id).and_then(|record| record.how)
    }
}

/// What kind of start this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    /// Nothing of an earlier KeySwitch is there.
    Fresh,
    /// The version that ran last, or a newer one (a downgrade brings no news).
    Same,
    /// An update; `from` is None when the version before had no hints (0.66.x, 0.70.0), or its file could not be read.
    Updated { from: Option<SemanticVersion> },
}

/// Tells the start apart from the file of the hints (None when there is none or it could not be read) and whether any
/// file an earlier version leaves exists.
#[must_use]
pub fn startup_kind(current: SemanticVersion, recorded: Option<&HintState>, older_files: bool) -> Startup {
    match recorded.map(HintState::last_run_version) {
        Some(Some(last)) if last < current => Startup::Updated { from: Some(last) },
        Some(Some(_)) => Startup::Same,
        Some(None) => Startup::Updated { from: None },
        None if older_files => Startup::Updated { from: None },
        None => Startup::Fresh,
    }
}

/// The files any version of KeySwitch leaves for its user: the settings file and those in the data folder.
#[must_use]
pub fn user_files(settings_file: &Path, data_dir: &Path) -> [PathBuf; 4] {
    [
        settings_file.to_path_buf(),
        data_dir.join(HISTORY_FILE_NAME),
        data_dir.join(LEARNING_FILE_NAME),
        data_dir.join(PERSONAL_WORDS_FILE_NAME),
    ]
}

/// What a load found.
#[derive(Clone, Debug)]
pub enum Loaded {
    Missing,
    /// The file was no JSON object; it was moved aside under this name, if it could be.
    SetAside(Option<PathBuf>),
    Read(HintState),
}

/// `hints.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintStore {
    pub path: PathBuf,
}

impl HintStore {
    #[must_use]
    pub fn in_folder(data_dir: &Path) -> HintStore {
        HintStore { path: data_dir.join(HINTS_FILE_NAME) }
    }

    #[must_use]
    pub fn load(&self) -> Loaded {
        if !self.path.is_file() {
            return Loaded::Missing;
        }
        // utf-8-sig, as the other files of the user are read.
        let state = std::fs::read(&self.path).ok().and_then(|bytes| decode_text(&bytes, true)).and_then(|text| HintState::parse(&text));
        match state {
            Some(state) => Loaded::Read(state),
            // The next save would write over it: what it held stays beside it.
            None => Loaded::SetAside(set_aside_unreadable(&self.path)),
        }
    }

    /// Writes the state aside under a temporary name, then swaps it in.
    pub fn save(&self, state: &HintState) -> io::Result<()> {
        create_parent(&self.path)?;
        let temporary = with_suffix(&self.path, ".json.tmp");
        write_text(&temporary, &state.text())?;
        std::fs::rename(&temporary, &self.path)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn version(text: &str) -> SemanticVersion {
        SemanticVersion::parse(text).unwrap()
    }

    fn folder(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-hints-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    const NEWER: &str = r#"{"schema_version": 99, "future": {"a": [1, 2]}, "last_run_version": "0.72.0",
        "seen": {"later_hint": {"version": "0.72.0", "extra": true}, "typo": 5},
        "counters": {"automatic_corrections": 4, "manual_conversions": -2, "undos": "x", "later": 1},
        "day": {"date": "2026-11-07", "shown": 2}, "fresh_install": true, "update": {"from": null, "to": "0.72.0"},
        "first_run_at": "2026-11-01T00:00:00+00:00"}"#;

    #[test]
    fn a_file_of_a_newer_version_is_read_as_far_as_it_is_understood_and_written_back_whole() {
        let state = HintState::parse(NEWER).unwrap();
        assert_eq!(state.last_run_version(), Some(version("0.72.0")));
        assert!(state.is_seen("later_hint") && state.is_seen("typo") && !state.is_seen("history"));
        assert_eq!(state.seen_in("later_hint").as_deref(), Some("0.72.0"));
        assert_eq!(state.seen_in("typo"), None);
        assert_eq!(state.seen_in("history"), None);
        assert_eq!((state.counter(Counter::AutomaticCorrections), state.counter(Counter::ManualConversions)), (4, 0));
        assert_eq!(state.counted(), Counted { automatic_corrections: 4, manual_conversions: 0 });
        assert_eq!((state.shown_on("2026-11-07"), state.shown_on("2026-11-08")), (2, 0));
        assert!(state.fresh_install());
        assert_eq!(state.update(), Some((None, version("0.72.0"))));
        let store = HintStore::in_folder(&folder("round"));
        store.save(&state).unwrap();
        let written = std::fs::read_to_string(&store.path).unwrap().replace("\r\n", "\n");
        assert!(written.starts_with("{\n  \"schema_version\": 1,\n  \"last_run_version\": \"0.72.0\",\n"), "{written}");
        assert!(written.contains(r#""future": {"a": [1, 2]}"#), "{written}");
        assert!(written.contains(r#""later_hint": {"version": "0.72.0", "extra": true}"#), "{written}");
        assert!(written.contains(r#""typo": 5"#) && written.ends_with("}\n"), "{written}");
        let Loaded::Read(again) = store.load() else { panic!("the file reads back") };
        assert_eq!(again.text(), state.text());
    }

    #[test]
    fn a_field_of_another_type_reads_as_missing() {
        let state = HintState::parse(
            r#"{"last_run_version": 5, "first_run_at": [], "fresh_install": "yes", "update": "x", "counters": "many", "day": [], "seen": [1]}"#,
        )
        .unwrap();
        assert_eq!((state.last_run_version(), state.fresh_install(), state.update()), (None, false, None));
        assert_eq!((state.counted(), state.shown_on("")), (Counted::default(), 0));
        assert!(!state.is_seen("1"));
        let state = HintState::parse(r#"{"last_run_version": "seven", "update": {"from": "x", "to": "0.71.0"}}"#).unwrap();
        assert_eq!(state.last_run_version(), None);
        assert_eq!(state.update(), Some((None, version("0.71.0"))));
        assert_eq!(HintState::parse(r#"{"update": {"from": "0.66.2", "to": 3}}"#).unwrap().update(), None);
        assert_eq!(HintState::parse(r#"{"update": {"from": "0.66.2", "to": "0.71.0"}}"#).unwrap().update(), Some((Some(version("0.66.2")), version("0.71.0"))));
        assert_eq!(HintState::parse(r#"{"day": {"date": "d", "shown": -1}}"#).unwrap().shown_on("d"), 0);
        assert_eq!(HintState::parse(r#"{"day": {"date": 1, "shown": "x"}}"#).unwrap().shown_on(""), 0);
        assert!(HintState::parse("[1]").is_none() && HintState::parse("{").is_none());
    }

    #[test]
    fn text_that_is_no_object_is_set_aside_and_a_byte_order_mark_is_read() {
        let directory = folder("aside");
        let store = HintStore::in_folder(&directory);
        assert!(matches!(store.load(), Loaded::Missing));
        std::fs::write(&store.path, "[1, 2]").unwrap();
        let Loaded::SetAside(Some(aside)) = store.load() else { panic!("set aside") };
        assert!(aside.file_name().unwrap().to_string_lossy().starts_with("hints.json.unreadable-"));
        assert!(!store.path.exists() && aside.exists());
        std::fs::write(&store.path, b"\xef\xbb\xbf{\"last_run_version\": \"0.71.0\"}").unwrap();
        let Loaded::Read(state) = store.load() else { panic!("read") };
        assert_eq!(state.last_run_version(), Some(version("0.71.0")));
    }

    #[test]
    fn a_folder_in_place_of_the_file_is_no_file_and_takes_no_save() {
        let directory = folder("blocked");
        let store = HintStore::in_folder(&directory);
        std::fs::create_dir_all(&store.path).unwrap();
        assert!(matches!(store.load(), Loaded::Missing));
        assert!(store.save(&HintState::default()).is_err());
        std::fs::write(directory.join("file"), "").unwrap();
        assert!(HintStore { path: directory.join("file").join("hints.json") }.save(&HintState::default()).is_err());
        let tmp_blocked = folder("tmp-blocked");
        let store = HintStore::in_folder(&tmp_blocked);
        std::fs::create_dir_all(tmp_blocked.join("hints.json.tmp")).unwrap();
        assert!(store.save(&HintState::default()).is_err());
    }

    #[test]
    fn a_start_writes_its_version_and_its_kind() {
        let mut fresh = HintState::default();
        fresh.begin(Startup::Fresh, version("0.71.0"), "2026-11-07T10:00:00+00:00");
        assert!(fresh.fresh_install() && fresh.update().is_none());
        assert!(fresh.text().contains(r#""first_run_at": "2026-11-07T10:00:00+00:00""#));
        let mut updated = HintState::parse(r#"{"first_run_at": "2026-01-01T00:00:00+00:00"}"#).unwrap();
        updated.begin(Startup::Updated { from: Some(version("0.70.0")) }, version("0.71.0"), "2026-11-07T10:00:00+00:00");
        assert_eq!(updated.update(), Some((Some(version("0.70.0")), version("0.71.0"))));
        assert!(updated.text().contains(r#""first_run_at": "2026-01-01T00:00:00+00:00""#));
        let mut same = HintState::default();
        same.begin(Startup::Same, version("0.71.0"), "t");
        assert!(!same.fresh_install() && same.update().is_none());
        assert_eq!(same.last_run_version(), Some(version("0.71.0")));
    }

    #[test]
    fn a_start_is_told_apart() {
        let current = version("0.71.0");
        let ran = |text: &str| HintState::parse(&format!(r#"{{"last_run_version": "{text}"}}"#)).unwrap();
        assert_eq!(startup_kind(current, Some(&ran("0.71.0")), true), Startup::Same);
        assert_eq!(startup_kind(current, Some(&ran("0.72.0")), true), Startup::Same);
        assert_eq!(startup_kind(current, Some(&ran("0.70.0")), false), Startup::Updated { from: Some(version("0.70.0")) });
        assert_eq!(startup_kind(current, Some(&ran("garbage")), false), Startup::Updated { from: None });
        assert_eq!(startup_kind(current, None, true), Startup::Updated { from: None });
        assert_eq!(startup_kind(current, None, false), Startup::Fresh);
    }

    #[test]
    fn counters_and_days_count() {
        let mut state = HintState::default();
        state.count(Counter::AutomaticCorrections);
        state.count(Counter::AutomaticCorrections);
        state.count(Counter::ManualConversions);
        assert_eq!(state.counted(), Counted { automatic_corrections: 2, manual_conversions: 1 });
        state.count_shown("2026-11-07");
        state.count_shown("2026-11-07");
        assert_eq!(state.shown_on("2026-11-07"), 2);
        state.count_shown("2026-11-08");
        assert_eq!((state.shown_on("2026-11-07"), state.shown_on("2026-11-08")), (0, 1));
    }

    #[test]
    fn how_a_hint_went_away_is_written_on_its_record() {
        let mut state = HintState::parse(r#"{"seen": {"odd": 5}}"#).unwrap();
        state.see("typo", version("0.71.0"), "t", How::Shown);
        assert_eq!(state.how("typo").as_deref(), Some("shown"));
        state.set_how("typo", How::Ok);
        assert_eq!((state.how("typo").as_deref(), state.seen_in("typo").as_deref()), (Some("ok"), Some("0.71.0")));
        state.set_how("never", How::Ok);
        assert!(!state.is_seen("never"));
        state.set_how("odd", How::Timeout);
        assert_eq!(state.how("odd").as_deref(), Some("timeout"));
        let names: Vec<&str> = [How::Shown, How::Ok, How::Timeout, How::Forever, How::Skipped, How::Click].iter().map(|how| how.name()).collect();
        assert_eq!(names, ["shown", "ok", "timeout", "forever", "skipped", "click"]);
    }

    #[test]
    fn the_files_an_earlier_version_leaves() {
        let data = Path::new("d");
        let files = user_files(Path::new("config.json"), data);
        assert_eq!(files, [PathBuf::from("config.json"), data.join("history.jsonl"), data.join("learning.json"), data.join("personal-words.json")]);
    }
}

//! Application log file with a rotation budget that follows the log mode.
//!
//! Ordinary operation writes a handful of lines per session, while the diagnostics ("developer") mode writes a JSON
//! line for every evaluated word, so one budget cannot serve both. The handler therefore carries the limits of the
//! mode that is switched on, and turning the diagnostics mode on starts a fresh file so the log that is later attached
//! to a report contains that session only.
//!
//! This is also the whole of the `logging` module that the program uses: one global entry [`log`] (and the
//! [`debug`], [`info`], [`warning`], [`error`] shorthands) that writes one line to the file handler installed on the
//! root, and the rotating file handler itself. No `log` or `tracing` crate: tools parse the line by regex.
//!
//! The line is `%(asctime)s %(version)s %(levelname)s %(name)s: %(message)s` as Python writes it, for instance
//! `2026-11-04 12:00:00,123 0.66.1 INFO keyswitch.engine: text`; `asctime` is local time with a comma before the
//! milliseconds. Python's rotating handler is reproduced: `keyswitch.log`, then `keyswitch.log.1` ... `.N`, a file is
//! rolled over when its size plus the length of the next line (counted in characters, as Python does) reaches the
//! limit, and an empty file is never rolled over.
//!
//! Local time. Reading the local clock takes a call into the operating system (`localtime_r`, `GetLocalTime`), and
//! `unsafe` is forbidden here, so the platform crate hands in the clock with [`set_local_clock`]. Until it does, the
//! time is UTC, computed by hand from the system clock. The same goes for the description of the system in the
//! first line ([`set_platform_description`], Python's `platform.platform()`).

use ks_cast::To as _;
use crate::config::SettingsStore;
use crate::constants::{BYTES_PER_MEBIBYTE, DEFAULT_LOG_ROTATION, TECHNICAL_LOG_ROTATION};
use crate::history::data_dir;
use crate::pycompat::truthy;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOG_FILE_NAME: &str = "keyswitch.log";
/// Every line carries the version: a rotated file that outlives an update must not be read as if it came from the
/// version that is installed now.
pub const LOG_FORMAT: &str = "%(asctime)s %(version)s %(levelname)s %(name)s: %(message)s";
pub const TECHNICAL_LOGGING_PATH: &str = "diagnostics.technical_logging";
/// The version stamped on every line.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The logger of this module's own messages.
const LOGGER_NAME: &str = "keyswitch.logsetup";

// ---------------------------------------------------------------- local time

/// A wall-clock reading, broken down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

static CLOCK: Mutex<Option<fn() -> LocalTime>> = Mutex::new(None);
static PLATFORM: Mutex<Option<String>> = Mutex::new(None);

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The platform crate sets the clock that reads the local time (with `localtime_r` or `GetLocalTime`).
pub fn set_local_clock(clock: fn() -> LocalTime) {
    *locked(&CLOCK) = Some(clock);
}

/// The platform crate sets the system description of the first log line (Python's `platform.platform()`).
pub fn set_platform_description(description: &str) {
    *locked(&PLATFORM) = Some(description.to_string());
}

/// The broken-down UTC time of `unix_milliseconds` since the epoch (the fallback clock).
#[must_use]
pub fn utc_time(unix_milliseconds: i64) -> LocalTime {
    let seconds = unix_milliseconds.div_euclid(1000);
    let millisecond = unix_milliseconds.rem_euclid(1000).to::<u32>();
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1).to::<u32>();
    let month = (if month_index < 10 { month_index + 3 } else { month_index - 9 }).to::<u32>();
    let year = (year_of_era + era * 400 + i64::from(month <= 2)).to::<i32>();
    LocalTime {
        year,
        month,
        day,
        hour: (rest / 3600).to::<u32>(),
        minute: (rest % 3600 / 60).to::<u32>(),
        second: (rest % 60).to::<u32>(),
        millisecond,
    }
}

/// The current time in UTC.
#[must_use]
pub fn utc_now() -> LocalTime {
    // A clock set before 1970 reads as 1970.
    let milliseconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |after| after.as_millis().to::<i64>());
    utc_time(milliseconds)
}

/// The current local time: the platform's clock when it has been set, UTC otherwise.
#[must_use]
pub fn local_time() -> LocalTime {
    let clock = *locked(&CLOCK);
    match clock {
        Some(clock) => clock(),
        None => utc_now(),
    }
}

/// `time.strftime(format, ...)` for the directives this program uses: `%Y %m %d %H %M %S` and `%%`.
#[must_use]
pub fn strftime(format: &str, time: &LocalTime) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{:04}", time.year)),
            Some('m') => out.push_str(&format!("{:02}", time.month)),
            Some('d') => out.push_str(&format!("{:02}", time.day)),
            Some('H') => out.push_str(&format!("{:02}", time.hour)),
            Some('M') => out.push_str(&format!("{:02}", time.minute)),
            Some('S') => out.push_str(&format!("{:02}", time.second)),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn platform_name() -> &'static str {
    // sys.platform
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn platform_description() -> String {
    if let Some(text) = locked(&PLATFORM).clone() {
        return text;
    }
    let os = std::env::consts::OS;
    let mut name = os.to_string();
    name.get_mut(0..1).into_iter().for_each(|first| first.make_ascii_uppercase());
    format!("{name}-{}", std::env::consts::ARCH)
}

// ---------------------------------------------------------------- levels and records

/// The logging levels, with Python's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warning = 30,
    Error = 40,
    Critical = 50,
}

impl Level {
    /// `logging.getLevelName(level)`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
            Level::Critical => "CRITICAL",
        }
    }
}

/// One record: when, how important, which logger wrote it, and what.
#[derive(Clone, Debug)]
pub struct LogRecord {
    pub time: LocalTime,
    pub level: Level,
    pub name: String,
    pub message: String,
}

/// `log_formatter().format(record)`: the line without its line ending.
#[must_use]
pub fn format_record(record: &LogRecord) -> String {
    let t = &record.time;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03} {} {} {}: {}",
        t.year,
        t.month,
        t.day,
        t.hour,
        t.minute,
        t.second,
        t.millisecond,
        VERSION,
        record.level.name(),
        record.name,
        record.message
    )
}

// ---------------------------------------------------------------- the rotating file handler

struct HandlerState {
    base_filename: PathBuf,
    max_bytes: u64,
    backup_count: u64,
    stream: Option<File>,
    /// `stream.tell()`: the size of the file, kept up as lines are written.
    position: u64,
}

/// `logging.handlers.RotatingFileHandler` (UTF-8, append mode, not delayed); a clone is another handle to the same one.
#[derive(Clone)]
pub struct RotatingFileHandler {
    state: Arc<Mutex<HandlerState>>,
}

/// What a text file opened with `open(path, "a")` ends a line with.
const TERMINATOR: &str = if cfg!(windows) { "\r\n" } else { "\n" };

impl HandlerState {
    fn open(&mut self) -> io::Result<()> {
        let file = OpenOptions::new().create(true).append(true).open(&self.base_filename)?;
        self.position = file.metadata()?.len();
        self.stream = Some(file);
        Ok(())
    }

    fn rotation_name(&self, index: u64) -> PathBuf {
        let mut name: OsString = self.base_filename.clone().into_os_string();
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    fn do_rollover(&mut self) -> io::Result<()> {
        self.stream = None;
        if self.backup_count > 0 {
            for index in (1..self.backup_count).rev() {
                let source = self.rotation_name(index);
                let target = self.rotation_name(index + 1);
                if source.exists() {
                    if target.exists() {
                        std::fs::remove_file(&target)?;
                    }
                    std::fs::rename(&source, &target)?;
                }
            }
            let target = self.rotation_name(1);
            if target.exists() {
                std::fs::remove_file(&target)?;
            }
            if self.base_filename.exists() {
                std::fs::rename(&self.base_filename, &target)?;
            }
        }
        self.open()
    }

    fn should_rollover(&mut self, line: &str) -> io::Result<bool> {
        if self.stream.is_none() {
            self.open()?;
        }
        if self.max_bytes > 0 {
            if self.position == 0 {
                // Never roll over an empty file.
                return Ok(false);
            }
            // The length of the text with its "\n", in characters, against the size of the file in bytes.
            let length = line.chars().count().to::<u64>() + 1;
            if self.position + length >= self.max_bytes {
                // Never roll over anything other than a regular file.
                if self.base_filename.exists() && !self.base_filename.is_file() {
                    return Ok(false);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn emit(&mut self, line: &str) -> io::Result<()> {
        if self.should_rollover(line)? {
            self.do_rollover()?;
        }
        let mut text = String::with_capacity(line.len() + 2);
        text.push_str(line);
        text.push_str(TERMINATOR);
        let stream = self.stream.as_mut().expect("opened above");
        stream.write_all(text.as_bytes())?;
        stream.flush()?;
        self.position += text.len().to::<u64>();
        Ok(())
    }
}

impl RotatingFileHandler {
    /// `RotatingFileHandler(path, maxBytes=..., backupCount=..., encoding="utf-8")`: the file is opened (created) at once.
    pub fn new(path: &Path, max_bytes: u64, backup_count: u64) -> io::Result<Self> {
        let base_filename = std::path::absolute(path)?;
        let mut state = HandlerState { base_filename, max_bytes, backup_count, stream: None, position: 0 };
        state.open()?;
        Ok(RotatingFileHandler { state: Arc::new(Mutex::new(state)) })
    }

    fn lock(&self) -> MutexGuard<'_, HandlerState> {
        locked(&self.state)
    }

    /// `handler.baseFilename`.
    #[must_use]
    pub fn base_filename(&self) -> PathBuf {
        self.lock().base_filename.clone()
    }
    /// `handler.maxBytes`.
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.lock().max_bytes
    }
    /// `handler.backupCount`.
    #[must_use]
    pub fn backup_count(&self) -> u64 {
        self.lock().backup_count
    }

    /// Writes one record. A failure to write loses the record and nothing else, as with Python's `handleError`.
    pub fn emit(&self, record: &LogRecord) {
        let line = format_record(record);
        let _ = self.lock().emit(&line);
    }

    /// `handler.doRollover()`.
    pub fn do_rollover(&self) -> io::Result<()> {
        self.lock().do_rollover()
    }

    /// `handler.close()`: the file is closed; the next record opens it again.
    pub fn close(&self) {
        self.lock().stream = None;
    }

    fn same(&self, other: &RotatingFileHandler) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

// ---------------------------------------------------------------- the root logger

struct Root {
    level: Level,
    handler: Option<RotatingFileHandler>,
}

// Python's root logger starts at WARNING.
static ROOT: Mutex<Root> = Mutex::new(Root { level: Level::Warning, handler: None });

/// `logging.getLogger(name).log(level, message)`: one line to the file handler on the root, if the root lets the level
/// through; with no handler, a warning or worse goes to the standard error stream as Python's last resort does.
pub fn log(level: Level, logger_name: &str, message: &str) {
    let (root_level, handler) = {
        let root = locked(&ROOT);
        (root.level, root.handler.clone())
    };
    if level < root_level {
        return;
    }
    match handler {
        Some(handler) => {
            handler.emit(&LogRecord { time: local_time(), level, name: logger_name.to_string(), message: message.to_string() });
        }
        None if level >= Level::Warning => eprintln!("{message}"),
        None => {}
    }
}

pub fn debug(logger_name: &str, message: &str) {
    log(Level::Debug, logger_name, message);
}
pub fn info(logger_name: &str, message: &str) {
    log(Level::Info, logger_name, message);
}
pub fn warning(logger_name: &str, message: &str) {
    log(Level::Warning, logger_name, message);
}
pub fn error(logger_name: &str, message: &str) {
    log(Level::Error, logger_name, message);
}

/// `logging.getLogger().setLevel(level)`.
pub fn set_root_level(level: Level) {
    locked(&ROOT).level = level;
}

/// `logging.getLogger().removeHandler(handler)` for the installed file handler; the handler is closed.
pub fn uninstall_handler() {
    let old = locked(&ROOT).handler.take();
    if let Some(old) = old {
        old.close();
    }
}

// ---------------------------------------------------------------- the module

/// Directory holding the log file and its rotated copies.
#[must_use]
pub fn log_directory() -> PathBuf {
    data_dir()
}

#[must_use]
pub fn log_path() -> PathBuf {
    log_directory().join(LOG_FILE_NAME)
}

#[must_use]
pub fn rotation_limits(technical: bool) -> (u64, u64) {
    if technical { TECHNICAL_LOG_ROTATION } else { DEFAULT_LOG_ROTATION }
}

/// Human wording of the current budget, shown on the diagnostics page.
#[must_use]
pub fn rotation_summary(technical: bool) -> String {
    let (maximum, backups) = rotation_limits(technical);
    format!("{} МБ × {} файлов", maximum / BYTES_PER_MEBIBYTE, backups + 1)
}

/// Attach the file log to the root logger and let INFO records through.
///
/// `logging.basicConfig` does nothing at all once the root logger carries a handler, and it fails silently: the file
/// is created by the handler and then never written, which is indistinguishable from a quiet session. Wiring the root
/// logger directly is the only way to be sure the log is really live.
pub fn install_handler(handler: &RotatingFileHandler) {
    let old = {
        let mut root = locked(&ROOT);
        root.level = Level::Info;
        root.handler.replace(handler.clone())
    };
    if let Some(old) = old
        && !old.same(handler)
    {
        old.close();
    }
}

/// State of the file log, so a silent journal is visible in a report.
#[must_use]
pub fn log_status() -> Map<String, Value> {
    log_status_of(&log_path())
}

/// `log_status` for a given log file path.
#[must_use]
pub fn log_status_of(path: &Path) -> Map<String, Value> {
    let handler = file_handler();
    let size = std::fs::metadata(path).map_or(-1, |m| m.len().to::<i64>());
    let level = locked(&ROOT).level;
    let technical = handler.as_ref().is_some_and(|h| h.max_bytes() == TECHNICAL_LOG_ROTATION.0);
    let mut status = Map::new();
    status.insert("path".into(), json!(path.to_string_lossy()));
    status.insert("installed".into(), json!(handler.is_some()));
    status.insert("level".into(), json!(level.name()));
    status.insert("size".into(), json!(size));
    status.insert("technical".into(), json!(technical));
    status
}

/// The rotating handler installed by [`configure_logging`], if any.
#[must_use]
pub fn file_handler() -> Option<RotatingFileHandler> {
    locked(&ROOT).handler.clone()
}

/// Give the handler the budget of `technical` and report a rollover.
///
/// Switching the diagnostics mode on rolls the current file over, so the new session starts empty; switching it off
/// only shrinks the budget, because discarding the diagnostics that were just recorded would be surprising.
pub fn apply_rotation(handler: &RotatingFileHandler, technical: bool) -> io::Result<bool> {
    let (maximum, backups) = rotation_limits(technical);
    let mut state = handler.lock();
    let was_technical = state.max_bytes == TECHNICAL_LOG_ROTATION.0;
    state.max_bytes = maximum;
    state.backup_count = backups;
    if was_technical || !technical {
        return Ok(false);
    }
    match std::fs::metadata(&state.base_filename) {
        Ok(meta) if meta.len() > 0 => {}
        _ => return Ok(false),
    }
    state.do_rollover()?;
    Ok(true)
}

/// Stops the following of the settings; may be called any number of times.
pub type Unsubscribe = Box<dyn Fn() + Send + Sync>;

/// Track the diagnostics setting on `handler` and apply it right away.
///
/// Returns the unsubscribe callable, or `None` when no file handler is installed (`--diagnose` and the tests run
/// without one).
pub fn follow_settings(settings: &Arc<SettingsStore>, handler: Option<&RotatingFileHandler>) -> Option<Unsubscribe> {
    let target = match handler {
        Some(handler) => handler.clone(),
        None => file_handler()?,
    };
    let technical = |settings: &SettingsStore| truthy(&settings.get_or(TECHNICAL_LOGGING_PATH, Value::Bool(false)));
    let _ = apply_rotation(&target, technical(settings));
    let weak: Weak<SettingsStore> = Arc::downgrade(settings);
    let changed_target = target.clone();
    Some(settings.subscribe(Arc::new(move |path: &str, _value: &Value| {
        if path != TECHNICAL_LOGGING_PATH && path != "*" {
            return;
        }
        // The store calls this, so it is alive.
        weak.upgrade().into_iter().for_each(|settings| {
            let _ = apply_rotation(&changed_target, truthy(&settings.get_or(TECHNICAL_LOGGING_PATH, Value::Bool(false))));
        });
    })))
}

/// Install the rotating file log; follow `settings` when they are known.
pub fn configure_logging(settings: Option<&Arc<SettingsStore>>) -> io::Result<RotatingFileHandler> {
    configure_logging_in(&log_directory(), settings)
}

/// [`configure_logging`] with the log directory given.
pub fn configure_logging_in(directory: &Path, settings: Option<&Arc<SettingsStore>>) -> io::Result<RotatingFileHandler> {
    std::fs::create_dir_all(directory)?;
    let technical = settings.is_some_and(|s| truthy(&s.get_or(TECHNICAL_LOGGING_PATH, Value::Bool(false))));
    let (maximum, backups) = rotation_limits(technical);
    let path = directory.join(LOG_FILE_NAME);
    let handler = RotatingFileHandler::new(&path, maximum, backups)?;
    install_handler(&handler);
    info(
        LOGGER_NAME,
        &format!(
            "KeySwitch {} запущен на {} ({}): журнал {}, ротация {}{}",
            VERSION,
            platform_name(),
            // The build of the system: on Windows it decides how the keyboard hook is treated.
            platform_description(),
            path.display(),
            rotation_summary(technical),
            if technical { ", режим диагностики" } else { "" }
        ),
    );
    if let Some(settings) = settings {
        follow_settings(settings, Some(&handler));
    }
    Ok(handler)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use std::sync::Mutex as TestMutex;

    #[test]
    fn a_folder_in_place_of_the_log_is_never_rolled_over() {
        let folder = directory("folder-in-place");
        let file = folder.join("keyswitch.log");
        let mut state = HandlerState { base_filename: file.clone(), max_bytes: 5, backup_count: 1, stream: None, position: 0 };
        state.open().unwrap();
        state.position = 4;
        assert!(state.should_rollover("line").unwrap());
        std::fs::remove_file(&file).unwrap();
        assert!(state.should_rollover("line").unwrap());
        std::fs::create_dir(&file).unwrap();
        assert!(!state.should_rollover("line").unwrap());
    }

    #[test]
    fn format_codes_it_does_not_know_stay_as_they_are() {
        let time = record("x").time;
        assert_eq!(strftime("%% %q %", &time), "% %q %");
        assert_eq!(utc_now().year >= 2026, true);
    }

    #[test]
    fn the_platform_crate_sets_the_clock_and_the_description() {
        let _turn = ROOT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_local_clock(|| utc_time(0));
        assert_eq!(local_time().year, 1970);
        *locked(&CLOCK) = None;
        assert!(local_time().year >= 2026);
        set_platform_description("Linux-test");
        assert_eq!(platform_description(), "Linux-test");
        *locked(&PLATFORM) = None;
        assert!(platform_description().contains(std::env::consts::ARCH));
        assert!(!platform_name().is_empty());
    }

    #[test]
    fn rotation_survives_a_missing_file_and_a_closed_stream() {
        let folder = directory("rotation-edges");
        let file = folder.join("keyswitch.log");
        let handler = RotatingFileHandler::new(&file, 10, 1).unwrap();
        assert_eq!(handler.base_filename(), std::path::absolute(&file).unwrap());
        handler.emit(&record("a long enough line to roll over"));
        handler.close();
        handler.emit(&record("after closing it opens again"));
        std::fs::remove_file(&file).unwrap();
        handler.do_rollover().unwrap();
        handler.close();
        let mut state = HandlerState { base_filename: folder.clone(), max_bytes: 1, backup_count: 1, stream: None, position: 1 };
        state.position = 1;
        assert!(state.open().is_err() || !state.should_rollover("line").unwrap_or(false));
    }

    #[test]
    fn without_a_handler_only_warnings_are_shown_and_a_handler_is_replaced_once() {
        let _turn = ROOT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        uninstall_handler();
        uninstall_handler();
        locked(&ROOT).level = Level::Debug;
        debug("keyswitch.test", "not shown");
        warning("keyswitch.test", "shown");
        error("keyswitch.test", "shown");
        let folder = directory("replace");
        let handler = RotatingFileHandler::new(&folder.join("a.log"), 0, 0).unwrap();
        install_handler(&handler);
        install_handler(&handler);
        uninstall_handler();
        assert!(!log_status_of(&folder.join("a.log")).is_empty());
        assert_eq!(log_path(), log_directory().join(LOG_FILE_NAME));
        let _ = log_status();
    }

    #[test]
    fn a_change_of_another_setting_leaves_the_rotation_alone() {
        let _turn = ROOT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let folder = directory("settings");
        let settings = Arc::new(SettingsStore::new(Some(folder.join("config.json"))));
        let handler = configure_logging_in(&folder, Some(&settings)).unwrap();
        settings.set("history.limit", Value::from(5), false).unwrap();
        settings.set(TECHNICAL_LOGGING_PATH, Value::Bool(true), false).unwrap();
        handler.close();
        uninstall_handler();
    }

    /// The root logger is one per process: the tests that use it take turns.
    static ROOT_TURN: TestMutex<()> = TestMutex::new(());

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-logsetup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn record(message: &str) -> LogRecord {
        LogRecord {
            time: LocalTime { year: 2026, month: 11, day: 4, hour: 9, minute: 5, second: 7, millisecond: 42 },
            level: Level::Info,
            name: "keyswitch.engine".into(),
            message: message.into(),
        }
    }

    #[test]
    fn the_utc_clock_is_computed_by_hand() {
        let t = utc_time(1_700_000_000_123);
        assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second, t.millisecond), (2023, 11, 14, 22, 13, 20, 123));
        let t = utc_time(951_782_400_000); // 2000-02-29
        assert_eq!((t.year, t.month, t.day), (2000, 2, 29));
        let t = utc_time(-1);
        assert_eq!((t.year, t.month, t.day, t.hour, t.second, t.millisecond), (1969, 12, 31, 23, 59, 999));
        assert_eq!(strftime("%Y%m%d-%H%M%S %%", &utc_time(1_700_000_000_123)), "20231114-221320 %");
    }

    #[test]
    fn the_line_is_the_contract() {
        let line = format_record(&record("исправление"));
        assert_eq!(line, format!("2026-11-04 09:05:07,042 {VERSION} INFO keyswitch.engine: исправление"));
        // ^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) (\S+) (\w+) ([\w.]+): ...
        let bytes = line.as_bytes();
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[19], b',');
        assert_eq!(bytes[23], b' ');
    }

    #[test]
    fn the_budget_follows_the_mode() {
        assert_eq!(rotation_limits(false), DEFAULT_LOG_ROTATION);
        assert_eq!(rotation_limits(true), TECHNICAL_LOG_ROTATION);
        assert_eq!(rotation_summary(true), "5 МБ × 6 файлов");
        assert_eq!(rotation_summary(false), "1 МБ × 3 файлов");
    }

    #[test]
    fn rotation_scheme_and_limits() {
        let dir = directory("rotate");
        let path = dir.join("keyswitch.log");
        // Room for two lines: the third one does not fit.
        let length = (format_record(&record("")).chars().count()).to::<u64>();
        let handler = RotatingFileHandler::new(&path, 2 * (length + 1) + 1, 2).unwrap();
        for _ in 0..8 {
            handler.emit(&record(""));
        }
        let text = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(text("keyswitch.log").lines().count(), 2);
        assert_eq!(text("keyswitch.log.1").lines().count(), 2);
        assert_eq!(text("keyswitch.log.2").lines().count(), 2);
        assert!(!dir.join("keyswitch.log.3").exists());
        assert!(text("keyswitch.log").lines().all(|line| line.chars().count().to::<u64>() == length));
    }

    #[test]
    fn enabling_the_mode_starts_a_new_file() {
        let dir = directory("apply");
        let path = dir.join("keyswitch.log");
        let handler = RotatingFileHandler::new(&path, DEFAULT_LOG_ROTATION.0, DEFAULT_LOG_ROTATION.1).unwrap();
        // An empty file is not rolled over.
        assert!(!apply_rotation(&handler, true).unwrap());
        assert!(!dir.join("keyswitch.log.1").exists());
        apply_rotation(&handler, false).unwrap();
        handler.emit(&record("before"));
        assert!(!apply_rotation(&handler, false).unwrap());
        assert!(apply_rotation(&handler, true).unwrap());
        assert_eq!((handler.max_bytes(), handler.backup_count()), TECHNICAL_LOG_ROTATION);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(std::fs::read_to_string(dir.join("keyswitch.log.1")).unwrap().contains("before"));
        handler.emit(&record("during"));
        assert!(!apply_rotation(&handler, true).unwrap());
        assert!(!apply_rotation(&handler, false).unwrap());
        assert_eq!((handler.max_bytes(), handler.backup_count()), DEFAULT_LOG_ROTATION);
        assert!(std::fs::read_to_string(&path).unwrap().contains("during"));
        handler.close();
        std::fs::remove_file(&path).unwrap();
        assert!(!apply_rotation(&handler, true).unwrap());
    }

    #[test]
    fn the_handler_follows_the_setting_and_the_root_is_wired() {
        let _turn = ROOT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = directory("follow");
        let settings = Arc::new(SettingsStore::new(Some(dir.join("config.json"))));
        settings.set("diagnostics.technical_logging", Value::Bool(true), true).unwrap();
        let handler = configure_logging_in(&dir.join("data"), Some(&settings)).unwrap();
        assert_eq!((handler.max_bytes(), handler.backup_count()), TECHNICAL_LOG_ROTATION);
        let banner = std::fs::read_to_string(dir.join("data").join("keyswitch.log")).unwrap();
        assert!(banner.contains(&format!(" {VERSION} INFO keyswitch.logsetup: KeySwitch {VERSION} запущен на ")));
        assert!(banner.contains("5 МБ × 6 файлов, режим диагностики"));
        settings.set("diagnostics.technical_logging", Value::Bool(false), true).unwrap();
        assert_eq!((handler.max_bytes(), handler.backup_count()), DEFAULT_LOG_ROTATION);
        settings.set("diagnostics.technical_logging", Value::Bool(true), true).unwrap();
        settings.set("detection.aggressive", Value::Bool(true), true).unwrap();
        assert_eq!(handler.max_bytes(), TECHNICAL_LOG_ROTATION.0);
        settings.reset().unwrap();
        assert_eq!(handler.max_bytes(), DEFAULT_LOG_ROTATION.0);
        let unsubscribe = follow_settings(&settings, None).expect("the installed handler is found");
        unsubscribe();
        settings.set("diagnostics.technical_logging", Value::Bool(true), true).unwrap();
        assert_eq!(handler.max_bytes(), TECHNICAL_LOG_ROTATION.0);

        info("keyswitch.engine", "проверка");
        debug("keyswitch.engine", "не пишется");
        let status = log_status_of(&dir.join("data").join("keyswitch.log"));
        assert_eq!(status["installed"], Value::Bool(true));
        assert_eq!(status["level"], "INFO");
        assert!(status["size"].as_i64().unwrap() > 0);
        assert_eq!(status["technical"], Value::Bool(true));
        let text = std::fs::read_to_string(dir.join("data").join("keyswitch.log")).unwrap();
        assert!(text.contains(" INFO keyswitch.engine: проверка") && !text.contains("не пишется"));

        // A second start replaces the handler instead of doubling it.
        let replacement = configure_logging_in(&dir.join("data"), None).unwrap();
        assert!(file_handler().unwrap().same(&replacement));
        uninstall_handler();
        assert!(file_handler().is_none());
        set_root_level(Level::Warning);
        assert_eq!(log_status_of(&dir.join("missing").join("keyswitch.log"))["size"], -1);
        assert!(follow_settings(&settings, None).is_none());
    }
}

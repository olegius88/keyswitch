//! The settings file, in the format the previous version wrote.
use std::io::Write;
use std::path::{Path, PathBuf};

use ks_text::{Indent, JsonOptions, json_dumps};
use serde_json::{Map, Value, json};

use crate::constants::*;
use crate::error::{Error, Result};
use crate::timeutil::new_id;

pub fn data_directory() -> PathBuf {
    if let Some(over) = std::env::var_os("LOGCOURIER_DATA_DIR").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(over);
        return std::path::absolute(&path).unwrap_or(path);
    }
    let home = || std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map_or_else(|| PathBuf::from("."), PathBuf::from);
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map_or_else(home, PathBuf::from).join("LogCourier");
    }
    std::env::var_os("XDG_DATA_HOME").map_or_else(|| home().join(".local/share"), PathBuf::from).join("logcourier")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub label: String,
    pub rotations: i64,
    pub include_existing: bool,
    pub id: String,
}

fn hex32(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digits_after(text: &str, first_required: bool, longest: usize) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty() && bytes.len() <= longest && bytes.iter().all(u8::is_ascii_digit) && (!first_required || bytes[0] != b'0')
}

impl Source {
    pub fn new(path: impl Into<String>, label: impl Into<String>) -> Source {
        Source { path: path.into(), label: label.into(), rotations: DEFAULT_ROTATIONS, include_existing: false, id: new_id() }
    }

    pub fn validate(&self) -> Result<()> {
        if !Path::new(&self.path).is_absolute() {
            return Err(Error::value("Выберите абсолютный путь к файлу."));
        }
        if !(0..=MAX_ROTATIONS).contains(&self.rotations) {
            return Err(Error::value(format!("Число ротаций должно быть от 0 до {MAX_ROTATIONS}.")));
        }
        if !hex32(&self.id) {
            return Err(Error::value("Некорректный идентификатор источника."));
        }
        if self.label.trim().is_empty() || self.label.chars().count() > MAX_SOURCE_LABEL_CHARACTERS {
            return Err(Error::value(format!("Длина названия источника в символах: от 1 до {MAX_SOURCE_LABEL_CHARACTERS}.")));
        }
        Ok(())
    }

    fn to_json(&self) -> Value {
        json!({"path": self.path, "label": self.label, "rotations": self.rotations, "include_existing": self.include_existing, "id": self.id})
    }

    fn from_json(value: &Value) -> Result<Source> {
        let object = value.as_object().ok_or_else(|| Error::Other("TypeError"))?;
        let mut source = Source::new("", "log");
        for (key, item) in object {
            let wrong = || Error::value("Некорректные типы настроек источника.");
            match key.as_str() {
                "path" => source.path = item.as_str().ok_or_else(wrong)?.to_owned(),
                "label" => source.label = item.as_str().ok_or_else(wrong)?.to_owned(),
                "rotations" => source.rotations = item.as_i64().ok_or_else(wrong)?,
                "include_existing" => source.include_existing = item.as_bool().ok_or_else(wrong)?,
                "id" => source.id = item.as_str().ok_or_else(wrong)?.to_owned(),
                _ => return Err(Error::Other("TypeError")),
            }
        }
        Ok(source)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub device_id: String,
    pub device_name: String,
    pub chat_id: String,
    pub bot_id: String,
    pub interval_minutes: i64,
    pub auto_send: bool,
    pub consent: bool,
    pub sources: Vec<Source>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            device_id: new_id(),
            device_name: "Мой компьютер".to_owned(),
            chat_id: String::new(),
            bot_id: String::new(),
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            auto_send: false,
            consent: false,
            sources: Vec::new(),
        }
    }
}

fn case_folded_path(path: &str) -> String {
    let absolute = std::path::absolute(path).map_or_else(|_| path.to_owned(), |resolved| resolved.display().to_string());
    if cfg!(windows) { absolute.to_lowercase().replace('/', "\\") } else { absolute }
}

impl Config {
    pub fn destination(&self) -> String {
        format!("{}:{}", self.bot_id, self.chat_id)
    }

    pub fn validate(&self) -> Result<()> {
        if !hex32(&self.device_id) {
            return Err(Error::value("Некорректный идентификатор устройства."));
        }
        if !self.chat_id.is_empty() && !self.chat_id.strip_prefix('-').is_some_and(|rest| digits_after(rest, true, 20)) {
            return Err(Error::value("Chat ID группы должен быть отрицательным числом."));
        }
        if !self.bot_id.is_empty() && !digits_after(&self.bot_id, true, 20) {
            return Err(Error::value("Некорректный ID бота."));
        }
        if !(1..=MAX_INTERVAL_MINUTES).contains(&self.interval_minutes) {
            return Err(Error::value(format!("Интервал в минутах: от 1 до {MAX_INTERVAL_MINUTES}.")));
        }
        if self.device_name.trim().is_empty() || self.device_name.chars().count() > MAX_DEVICE_NAME_CHARACTERS {
            return Err(Error::value(format!("Длина названия устройства в символах: от 1 до {MAX_DEVICE_NAME_CHARACTERS}.")));
        }
        if self.sources.len() > MAX_SOURCES {
            return Err(Error::value(format!("Число источников: не больше {MAX_SOURCES}.")));
        }
        let (mut ids, mut paths) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        for source in &self.sources {
            source.validate()?;
            if !ids.insert(source.id.clone()) || !paths.insert(case_folded_path(&source.path)) {
                return Err(Error::value("Один источник указан дважды."));
            }
        }
        Ok(())
    }

    fn to_json(&self) -> Value {
        json!({
            "device_id": self.device_id, "device_name": self.device_name, "chat_id": self.chat_id, "bot_id": self.bot_id,
            "interval_minutes": self.interval_minutes, "auto_send": self.auto_send, "consent": self.consent,
            "sources": self.sources.iter().map(Source::to_json).collect::<Vec<_>>(),
        })
    }

    fn from_json(object: &Map<String, Value>) -> Result<Config> {
        let mut config = Config::default();
        let wrong = || Error::value("Некорректные типы настроек отправки.");
        for (key, item) in object {
            match key.as_str() {
                "device_id" => config.device_id = item.as_str().ok_or_else(wrong)?.to_owned(),
                "device_name" => config.device_name = item.as_str().ok_or_else(wrong)?.to_owned(),
                "chat_id" => config.chat_id = item.as_str().ok_or_else(wrong)?.to_owned(),
                "bot_id" => config.bot_id = item.as_str().ok_or_else(wrong)?.to_owned(),
                "interval_minutes" => config.interval_minutes = item.as_i64().ok_or_else(wrong)?,
                "auto_send" => config.auto_send = item.as_bool().ok_or_else(wrong)?,
                "consent" => config.consent = item.as_bool().ok_or_else(wrong)?,
                "sources" => config.sources = item.as_array().ok_or_else(wrong)?.iter().map(Source::from_json).collect::<Result<_>>()?,
                _ => return Err(Error::Other("TypeError")),
            }
        }
        Ok(config)
    }
}

pub fn load_config(root: &Path) -> Result<Config> {
    let path = root.join("config.json");
    if !path.exists() {
        return Ok(Config::default());
    }
    if std::fs::metadata(&path)?.len() > CONFIG_MAX_BYTES {
        return Err(Error::value("Файл настроек слишком большой."));
    }
    let text = std::fs::read_to_string(&path)?;
    let data: Value = serde_json::from_str(&text).map_err(|_| Error::value("Файл настроек повреждён."))?;
    let config = Config::from_json(data.as_object().ok_or_else(|| Error::value("Файл настроек повреждён."))?)?;
    config.validate()?;
    Ok(config)
}

/// Creates a directory only its owner can open.
pub fn private_directory(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(root)?;
    Ok(())
}

/// Creates a new file only its owner can read.
pub fn private_new_file(path: &Path, overwrite: bool) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

pub fn save_config(root: &Path, config: &Config) -> Result<()> {
    config.validate()?;
    private_directory(root)?;
    let temporary = root.join("config.json.tmp");
    let mut stream = private_new_file(&temporary, true)?;
    let options = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(JSON_INDENT_SPACES)), ..JsonOptions::default() };
    stream.write_all(json_dumps(&config.to_json(), &options).as_bytes())?;
    stream.sync_all()?;
    drop(stream);
    std::fs::rename(&temporary, root.join("config.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_come_back_as_they_were_written() {
        let root = std::env::temp_dir().join(format!("lc-config-{}", new_id()));
        let mut config = Config { chat_id: "-1001".to_owned(), bot_id: "123".to_owned(), ..Config::default() };
        config.sources.push(Source::new("/var/log/a.log", "журнал"));
        save_config(&root, &config).expect("save");
        assert_eq!(load_config(&root).expect("load"), config);
        let text = std::fs::read_to_string(root.join("config.json")).expect("read");
        assert!(text.starts_with("{\n  \"device_id\": ") && text.contains("журнал"));
        std::fs::remove_dir_all(&root).expect("clean");
    }

    #[test]
    fn wrong_values_are_refused() {
        let good = Config::default();
        assert!(good.validate().is_ok());
        assert!(Config { chat_id: "100".to_owned(), ..good.clone() }.validate().is_err());
        assert!(Config { bot_id: "0123".to_owned(), ..good.clone() }.validate().is_err());
        assert!(Config { interval_minutes: 0, ..good.clone() }.validate().is_err());
        let mut twice = good;
        twice.sources = vec![Source::new("/a", "x"), Source::new("/a", "y")];
        assert!(twice.validate().is_err());
    }
}

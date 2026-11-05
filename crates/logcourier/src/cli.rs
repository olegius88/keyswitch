//! The command line of `LogCourier`: status, the catalog and fetching archives.
use ks_cast::To as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use ks_text::{Indent, JsonOptions, json_dumps};
use crate::catalog::list_entries;
use crate::config::{data_directory, load_config, private_directory, private_new_file};
use crate::constants::{JSON_INDENT_SPACES, SELECTION_HASH_PREFIX_CHARACTERS, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, MAX_CATALOG_PAGES, MAX_DOWNLOAD};
use crate::error::{Error, Result};
use crate::secrets::{read_token, redact};
use crate::telegram::{Client, Telegram};
use crate::timeutil::{iso_date, sha256_hex};
use crate::versions::is_version;
use serde_json::{Value, json};

fn pretty(value: &Value, sort_keys: bool) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(JSON_INDENT_SPACES)), sort_keys, ..JsonOptions::default() })
}

/// A complete, content-addressed selection never shares a folder with old downloads.
fn selection_directory(root: &Path, entries: &[Value], scope: Option<&str>) -> Result<(PathBuf, Option<Vec<u8>>)> {
    let Some(scope) = scope else {
        private_directory(root)?;
        return Ok((root.to_owned(), None));
    };
    let files: Vec<Value> = entries
        .iter()
        .map(|entry| json!({"name": format!("lc-{}.zip", entry["bundle_id"].as_str().unwrap_or_default()), "bundle_id": entry["bundle_id"], "source_id": entry["source_id"], "keyswitch_version": entry["keyswitch_version"], "sha256": entry["sha256"], "size": entry["size"]}))
        .collect();
    let manifest = json!({"schema": 1, "kind": "logcourier.selection", "keyswitch_version": scope, "files": files});
    let data = pretty(&manifest, true).into_bytes();
    let digest = sha256_hex(&data);
    let root = root.join(format!("keyswitch-{scope}-{}", digest.get(..SELECTION_HASH_PREFIX_CHARACTERS).unwrap_or_default()));
    if std::fs::symlink_metadata(&root).is_ok_and(|info| info.file_type().is_symlink()) {
        return Err(Error::value("Папка подборки является ссылкой; сохранение запрещено."));
    }
    private_directory(&root)?;
    let mut expected: Vec<String> = files.iter().filter_map(|file| file["name"].as_str().map(str::to_owned)).collect();
    expected.push("selection.json".to_owned());
    for item in std::fs::read_dir(&root)? {
        let name = item?.file_name();
        if !expected.iter().any(|known| *known == name.to_string_lossy()) {
            return Err(Error::value("В папке подборки есть посторонние файлы. Выберите другой --output."));
        }
    }
    let target = root.join("selection.json");
    if let Ok(info) = std::fs::symlink_metadata(&target)
        && (info.file_type().is_symlink() || std::fs::read(&target)? != data) {
            return Err(Error::value("Манифест подборки занят другим файлом; перезапись запрещена."));
        }
    Ok((root, Some(data)))
}

struct Options {
    chat_id: Option<String>,
    bot_id: Option<String>,
    limit: i64,
    version: Option<String>,
    output: Option<PathBuf>,
    since: Option<String>,
}

fn usage(message: &str) -> i32 {
    eprintln!("usage: logcourier [--version] {{gui,self-test,status,list,fetch}} ...\nlogcourier: error: {message}");
    2
}

fn parse(arguments: &[String]) -> std::result::Result<Options, String> {
    let mut options = Options { chat_id: None, bot_id: None, limit: DEFAULT_LIST_LIMIT, version: Some("current".to_owned()), output: None, since: None };
    let (mut index, mut scope) = (0, false);
    while index < arguments.len() {
        let name = arguments[index].as_str();
        index += 1;
        if name == "--all-versions" {
            if scope {
                return Err("--all-versions: not allowed with argument --keyswitch-version".to_owned());
            }
            scope = true;
            options.version = None;
            continue;
        }
        let value = arguments.get(index).cloned().ok_or_else(|| format!("argument {name}: expected one argument"))?;
        index += 1;
        match name {
            "--chat-id" => options.chat_id = Some(value),
            "--bot-id" => options.bot_id = Some(value),
            "--limit" => options.limit = value.parse().map_err(|_| format!("argument --limit: invalid int value: '{value}'"))?,
            "--keyswitch-version" => {
                if scope {
                    return Err("--keyswitch-version: not allowed with argument --all-versions".to_owned());
                }
                scope = true;
                options.version = Some(value);
            }
            "--output" => options.output = Some(PathBuf::from(value)),
            "--since" => options.since = Some(value),
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    Ok(options)
}

fn fetch_or_list(command: &str, options: Options) -> Result<i32> {
    let config = load_config(&data_directory())?;
    if !(1..=MAX_LIST_LIMIT).contains(&options.limit) {
        return Ok(usage(&format!("--limit должен быть от 1 до {MAX_LIST_LIMIT}")));
    }
    if options.version.as_deref().is_some_and(|version| version != "current" && !is_version(version)) {
        return Ok(usage("--keyswitch-version: ожидается версия, например 0.16.2, или current"));
    }
    if command == "fetch" && options.output.is_none() {
        return Ok(usage("the following arguments are required: --output"));
    }
    let chat_id = options.chat_id.filter(|chat| !chat.is_empty()).unwrap_or_else(|| config.chat_id.clone());
    if chat_id.is_empty() {
        return Ok(usage("Укажите --chat-id или сохраните группу в настройках"));
    }
    // Credentials are deliberately not accepted as command-line arguments.
    let token = match std::env::var("LOGCOURIER_BOT_TOKEN").ok().filter(|token| !token.is_empty()) {
        Some(token) => token,
        None => read_token(options.bot_id.as_deref().filter(|bot| !bot.is_empty()).unwrap_or(&config.bot_id))?,
    };
    if token.is_empty() {
        return Ok(usage("Нет токена в системном хранилище или LOGCOURIER_BOT_TOKEN"));
    }
    let client = Telegram::new(&token)?;
    let mut entries = list_entries(&client, &chat_id, options.limit.to::<usize>(), MAX_CATALOG_PAGES, options.version.as_deref())?;
    if command == "list" {
        println!("{}", pretty(&Value::Array(entries), false));
        return Ok(0);
    }
    if let Some(since) = &options.since {
        let since = iso_date(since).ok_or_else(|| Error::value("Invalid isoformat string"))?.to_owned();
        entries.retain(|entry| entry["created_at"].as_str().and_then(iso_date).is_some_and(|date| date >= since.as_str()));
    }
    if entries.is_empty() {
        eprintln!("Подходящих архивов пока нет; старые версии вместо них не скачивались.");
        return Ok(0);
    }
    let output = options.output.unwrap_or_default();
    let output = std::path::absolute(&output).unwrap_or(output);
    let (root, selection) = selection_directory(&output, &entries, options.version.as_deref())?;
    for entry in &entries {
        let data = client.download(entry["file_id"].as_str().unwrap_or_default(), MAX_DOWNLOAD)?;
        let digest = entry["sha256"].as_str().unwrap_or_default();
        if data.len().to::<i64>() != entry["size"].as_i64().unwrap_or(-1) || sha256_hex(&data) != digest {
            return Err(Error::value("Контрольная сумма архива не совпала; файл не сохранён."));
        }
        let target = root.join(format!("lc-{}.zip", entry["bundle_id"].as_str().unwrap_or_default()));
        // No overwrites, no archive extraction, no paths from remote metadata.
        if let Ok(info) = std::fs::symlink_metadata(&target) {
            if info.file_type().is_symlink() || sha256_hex(&std::fs::read(&target)?) != digest {
                return Err(Error::value("Путь результата занят другим файлом; перезапись запрещена."));
            }
            continue;
        }
        private_new_file(&target, false)?.write_all(&data)?;
        println!("{}", target.display());
    }
    if let Some(selection) = selection {
        let target = root.join("selection.json");
        if !target.exists() {
            private_new_file(&target, false)?.write_all(&selection)?;
        }
        eprintln!("Готовая подборка только выбранных версий: {}", target.display());
    }
    Ok(0)
}

#[cfg(windows)]
fn gui(arguments: &[String]) -> Result<i32> {
    if arguments.first().map(String::as_str) == Some("self-test") {
        let output = parse_output(&arguments[1..]);
        return crate::gui_win::smoke_test(output.as_deref());
    }
    Ok(crate::gui_win::run(arguments.iter().any(|argument| argument == "--minimized")))
}

#[cfg(target_os = "linux")]
fn gui(arguments: &[String]) -> Result<i32> {
    if arguments.first().map(String::as_str) == Some("self-test") {
        return crate::gui_gtk::smoke_test(parse_output(&arguments[1..]).as_deref());
    }
    Ok(crate::gui_gtk::run(arguments.iter().any(|argument| argument == "--minimized")))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn gui(_arguments: &[String]) -> Result<i32> {
    eprintln!("Окна настроек LogCourier на этой системе нет; настройки читаются из config.json.");
    Ok(1)
}

#[cfg(any(windows, target_os = "linux"))]
fn parse_output(arguments: &[String]) -> Option<PathBuf> {
    arguments.iter().position(|argument| argument == "--output").and_then(|index| arguments.get(index + 1)).map(PathBuf::from)
}

fn run(arguments: &[String]) -> Result<i32> {
    match arguments.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", crate::VERSION);
            Ok(0)
        }
        Some("status") => {
            let config = load_config(&data_directory())?;
            let status = json!({"version": crate::VERSION, "chat_id": config.chat_id, "bot_id": config.bot_id, "auto_send": config.auto_send, "sources": config.sources.len()});
            println!("{}", crate::store::dumps(&status, false));
            Ok(0)
        }
        Some(command @ ("list" | "fetch")) => match parse(&arguments[1..]) {
            Ok(options) => fetch_or_list(command, options),
            Err(message) => Ok(usage(&message)),
        },
        None | Some("gui" | "self-test") => gui(arguments),
        Some(other) => Ok(usage(&format!("argument command: invalid choice: '{other}'"))),
    }
}

/// The whole program: the console and the windowed binaries differ only in their subsystem.
pub fn entry() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&arguments) {
        Ok(code) => code,
        Err(error) if error.has_user_text() => {
            eprintln!("{}", redact(&error.to_string()));
            1
        }
        Err(error) => {
            eprintln!("Операция не выполнена ({}).", error.type_name());
            1
        }
    };
    std::process::exit(code);
}

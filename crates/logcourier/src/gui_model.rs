//! What the settings window shows and checks, with no window in it.
use ks_cast::To as _;
use serde_json::Value;

use crate::config::{Config, Source};
use crate::constants::{WAKE_POLL_SECONDS, MAX_QUEUE_BYTES, CHUNK_BYTES, MAX_DOWNLOAD, GROUP_INTERVAL, BOT_DELETE_WINDOW_HOURS, MAX_INTERVAL_MINUTES, MAX_ROTATIONS};
use crate::error::{Error, Result};
use crate::secrets::token_bot_id;

pub const WINDOW_TITLE_PREFIX: &str = "LogCourier";
pub const TRAY_TOOLTIP_MAX_CHARACTERS: usize = 120;
pub const EVENTS_LOG_MAX_LINES: usize = 150;

pub const CONNECTION_INTRO: &str = "1. Создайте отдельного бота через @BotFather.\r\n2. Создайте закрытую группу и добавьте бота администратором\r\n   с правом закреплять сообщения (чтобы очищать группу — и удалять их).\r\n3. Введите токен здесь, найдите группу и проверьте подключение.\r\n\r\nОдна группа — один сборщик. Не закрепляйте в ней другие сообщения.\r\nТокен не нужно присылать в чат, записывать в файл или включать в установщик.";
pub const SOURCES_NOTE: &str = "Читаются только выбранные файлы и их числовые ротации.\r\nИсходные файлы не изменяются и не удаляются. Удаление источника\r\nостанавливает новый сбор, но уже собранные архивы остаются в очереди.";
pub const SOURCE_NOTE: &str = "Без этого флажка сбор начнётся с конца файлов при первом запуске сбора.\r\nРотации: имя.log.1 … имя.log.N; архивы .gz не читаются.";

fn seconds(count: f64) -> String {
    let whole = count.to::<i64>();
    let (last, last_two) = (whole % 10, whole % 100);
    let form = if last == 1 && last_two != 11 {
        "секунду"
    } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
        "секунды"
    } else {
        "секунд"
    };
    format!("{whole} {form}")
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 { format!("{}", value.to::<i64>()) } else { format!("{value}").replace('.', ",") }
}

#[must_use]
pub fn delivery_note() -> String {
    format!(
        "Логи могут содержать переписку, имена, пути и другие личные данные.\r\nЭто передача полного содержимого, не обезличенная телеметрия.\r\n\r\nСбор раз в {}; отправка по выбранному интервалу.\r\nОчередь: до {} МиБ. Локальный фрагмент: до {} МиБ до сжатия.\r\nПеред отправкой фрагменты объединяются в пакеты до {} МБ.\r\nДокументы и закрепления — не чаще одного раза в {}.\r\nПауза Telegram сохраняется после перезапуска; кнопка её не сбрасывает.\r\nПри заполнении очереди сбор остановится. Ротация исходной программы\r\nможет удалить ещё не прочитанные записи — увеличьте её срок хранения.\r\n\r\nПри неопределённом ответе сети возможен повторный файл с тем же ID.\r\nКаталог и скачивание исключают повторы по идентификатору архива.\r\nФайлы в Telegram автоматически не удаляются.",
        seconds(WAKE_POLL_SECONDS.to::<f64>()),
        number(MAX_QUEUE_BYTES.to::<f64>() / 1_048_576.0),
        number(CHUNK_BYTES.to::<f64>() / 1_048_576.0),
        number(MAX_DOWNLOAD.to::<f64>() / 1_000_000.0),
        seconds(GROUP_INTERVAL),
    )
}

#[must_use]
pub fn reset_warning(chat_id: &str) -> String {
    format!(
        "Группа {chat_id}: сборщик снимет в ней все закрепления и начнёт новый каталог. Архивы, отправленные раньше, в новый каталог не попадут.\r\n\r\nС удалением бот удалит сообщения группы, в том числе чужие (нужно право бота «Удаление сообщений»). Telegram даёт ботам удалять только сообщения, отправленные за последние {BOT_DELETE_WINDOW_HOURS} часов; номера более ранних программа назовёт, их можно удалить вручную.\r\n\r\nНеотправленная очередь сохранится и уйдёт в новый каталог.\r\n\r\nДа — начать заново и удалить сообщения. Нет — начать заново без удаления. Отмена — ничего не менять."
    )
}

/// What the form holds, read into settings the way a save would store them.
pub struct Form<'a> {
    pub token: &'a str,
    pub chat: &'a str,
    pub device: &'a str,
    pub sources: &'a [Source],
    pub consent: bool,
    pub auto: bool,
    pub interval: &'a str,
}

pub fn draft(config: &Config, form: &Form<'_>) -> Result<(Config, String)> {
    let token = form.token.trim().to_owned();
    let mut draft = config.clone();
    if !token.is_empty() {
        draft.bot_id = token_bot_id(&token)?;
    }
    form.chat.trim().clone_into(&mut draft.chat_id);
    form.device.trim().clone_into(&mut draft.device_name);
    draft.sources = form.sources.to_vec();
    draft.consent = form.consent;
    draft.auto_send = form.auto;
    draft.interval_minutes = form.interval.trim().parse().map_err(|_| Error::value(format!("Интервал в минутах: от 1 до {MAX_INTERVAL_MINUTES}.")))?;
    draft.validate()?;
    if draft.auto_send && (!draft.consent || token.is_empty() || draft.chat_id.is_empty()) {
        return Err(Error::value("Для автоотправки нужны токен, группа и разрешение на передачу текста."));
    }
    Ok((draft, token))
}

/// A new source from the dialog; the same file twice is refused.
pub fn source_from(path: &str, label: &str, rotations: &str, existing: bool, sources: &[Source]) -> Result<Source> {
    let mut source = Source::new(path, label);
    source.rotations = rotations.trim().parse().map_err(|_| Error::value(format!("Число ротаций должно быть от 0 до {MAX_ROTATIONS}.")))?;
    source.include_existing = existing;
    source.validate()?;
    let same = |a: &str, b: &str| if cfg!(windows) { a.to_lowercase().replace('/', "\\") == b.to_lowercase().replace('/', "\\") } else { a == b };
    if sources.iter().any(|item| same(&item.path, &source.path)) {
        return Err(Error::value("Этот файл уже добавлен."));
    }
    Ok(source)
}

#[must_use]
pub fn group_label(group: &Value) -> String {
    format!("{} ({})", group["title"].as_str().unwrap_or("Группа"), group["id"])
}

/// The status block: `stamp` is the last delivery in the local form, `None` before the first.
#[must_use]
pub fn summary(message: &str, stats: &Value, sources: &[Source], stamp: Option<&str>) -> String {
    let versions = stats["keyswitch_versions"].as_object();
    let listed: Vec<String> = versions
        .map(|versions| {
            versions
                .iter()
                .filter(|(source, _)| sources.iter().any(|item| item.id == **source))
                .map(|(_, state)| format!("{}: {}", state["source_label"].as_str().unwrap_or(""), state["version"].as_str().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default();
    let version_text = if listed.is_empty() { "ещё не определена по логам".to_owned() } else { listed.join("; ") };
    format!(
        "{message}\r\nВ очереди: {}; ждут каталога: {}.\r\nВерсия KeySwitch: {version_text}.\r\nОбъём очереди: {:.1} МиБ.\r\nПоследняя успешная отправка: {}.\r\nАрхивов для других ботов/групп: {} (не отправляются сюда).",
        stats["queued"].as_i64().unwrap_or(0),
        stats["unindexed"].as_i64().unwrap_or(0),
        stats["queue_bytes"].as_f64().unwrap_or(0.0) / 1_048_576.0,
        stamp.unwrap_or("ещё не было"),
        stats["other"].as_i64().unwrap_or(0),
    )
}

#[must_use]
pub fn tray_tooltip(message: &str) -> String {
    format!("LogCourier — {message}").chars().take(TRAY_TOOLTIP_MAX_CHARACTERS).collect()
}

/// The command a user's login starts: the program in the tray.
pub fn autostart_command(executable: &str) -> Result<String> {
    let command = format!("\"{executable}\" gui --minimized");
    if command.chars().count() > 260 {
        return Err(Error::value("Путь для автозапуска Windows слишком длинный. Переместите программу ближе к корню диска."));
    }
    Ok(command)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn russian_numbers_of_seconds_and_the_texts_of_the_window() {
        for (count, form) in [(1.0, "1 секунду"), (21.0, "21 секунду"), (2.0, "2 секунды"), (5.0, "5 секунд"), (11.0, "11 секунд"), (12.0, "12 секунд"), (22.0, "22 секунды")] {
            assert_eq!(seconds(count), form);
        }
        assert_eq!((number(2.0), number(2.5)), ("2".to_owned(), "2,5".to_owned()));
        assert!(reset_warning("-100").contains("-100"));
        assert_eq!(group_label(&serde_json::json!({"title": "Логи", "id": 5})), "Логи (5)");
        assert_eq!(group_label(&serde_json::json!({"id": 5})), "Группа (5)");
        assert_eq!(autostart_command("a.exe").unwrap(), "\"a.exe\" gui --minimized");
        assert!(autostart_command(&"x".repeat(300)).is_err());
    }

    const TOKEN: &str = "123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn form(sources: &[Source]) -> Form<'_> {
        Form { token: TOKEN, chat: "-100", device: "ПК", sources, consent: true, auto: true, interval: "15" }
    }

    #[test]
    fn a_draft_takes_the_bot_from_the_token() {
        let (draft, token) = draft(&Config::default(), &form(&[])).unwrap();
        assert_eq!((draft.bot_id.as_str(), draft.chat_id.as_str(), token.as_str()), ("123456", "-100", TOKEN));
    }

    #[test]
    fn automatic_sending_needs_consent_a_token_and_a_group() {
        let sources = [];
        assert!(draft(&Config::default(), &Form { consent: false, ..form(&sources) }).is_err());
        assert!(draft(&Config::default(), &Form { token: "", ..form(&sources) }).is_err());
        assert!(draft(&Config::default(), &Form { chat: "", ..form(&sources) }).is_err());
        assert!(draft(&Config::default(), &Form { auto: false, chat: "", token: "", ..form(&sources) }).is_ok());
        assert!(draft(&Config::default(), &Form { interval: "много", ..form(&sources) }).is_err());
    }

    #[test]
    fn the_same_file_is_not_added_twice() {
        let path = if cfg!(windows) { "C:\\logs\\a.log" } else { "/var/log/a.log" };
        let first = source_from(path, "a", "5", false, &[]).unwrap();
        assert_eq!(first.rotations, 5);
        assert!(source_from(path, "b", "5", false, &[first]).is_err());
        assert!(source_from("relative.log", "b", "5", false, &[]).is_err());
    }

    #[test]
    fn the_status_names_only_current_sources() {
        let source = Source::new("/a.log", "a");
        let stats = json!({"queued": 2, "unindexed": 1, "queue_bytes": 1_572_864, "other": 0, "keyswitch_versions": {source.id.clone(): {"source_label": "a", "version": "0.70.0"}, "gone": {"source_label": "b", "version": "0.1.0"}}});
        let text = summary("Готово", &stats, std::slice::from_ref(&source), None);
        assert!(text.contains("a: 0.70.0") && !text.contains("0.1.0") && text.contains("1.5 МиБ") && text.contains("ещё не было"), "{text}");
        assert!(summary("x", &stats, &[], Some("01.01.2026 00:00:00")).contains("ещё не определена"));
        assert_eq!(tray_tooltip(&"я".repeat(200)).chars().count(), TRAY_TOOLTIP_MAX_CHARACTERS);
    }

    #[test]
    fn the_notes_carry_the_limits() {
        let note = delivery_note();
        assert!(note.contains("Сбор раз в 5 секунд;") && note.contains("до 128 МиБ") && note.contains("до 19 МБ") && note.contains("в 4 секунды"), "{note}");
    }
}

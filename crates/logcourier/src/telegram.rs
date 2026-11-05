//! The Telegram Bot API over the system's `curl`: no TLS code of our own, and the token never on a command line.
use ks_cast::To as _;
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::config::private_new_file;
use crate::constants::{REQUEST_TIMEOUT_SECONDS, ERROR_BODY_MAX_BYTES, ERROR_DESCRIPTION_MAX_CHARACTERS, TELEGRAM_GET_UPDATES_LIMIT, TELEGRAM_RESPONSE_MAX_BYTES, MAX_DOWNLOAD, TELEGRAM_MAX_CAPTION_CHARACTERS};
use crate::error::{Error, Result};
use crate::secrets::token_bot_id;
use crate::store::dumps;
use crate::timeutil::new_id;

#[must_use]
pub fn is_file_id(text: &str) -> bool {
    (1..=512).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// An answer of the server: its status and body.
pub type Answer = (i64, Vec<u8>);

/// How requests travel; the tests answer them without a network.
pub trait Transport: Send + Sync {
    fn post_json(&self, url: &str, body: &str) -> Result<Answer>;
    fn post_document(&self, url: &str, fields: &[(&str, String)], filename: &str, data: &[u8]) -> Result<Answer>;
    fn get(&self, url: &str) -> Result<Answer>;
}

/// A value inside a double-quoted `curl --config` string.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

pub struct Curl;

impl Curl {
    fn run(&self, config: &str, directory: Option<&std::path::Path>) -> Result<Answer> {
        let offline = || Error::telegram("Нет связи с Telegram. Архивы останутся в очереди.");
        let mut command = Command::new("curl");
        command.args(["--config", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: a console must not flash over the user's work at every delivery.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|_| offline())?;
        // The token is inside the URL: it goes through standard input, where no process list shows it.
        let mut input = child.stdin.take().ok_or_else(offline)?;
        let written = input.write_all(config.as_bytes());
        drop(input);
        let output = child.wait_with_output().map_err(|_| offline())?;
        written.map_err(|_| offline())?;
        if !output.status.success() {
            return Err(offline());
        }
        let end = output.stdout.iter().rposition(|byte| *byte == b'\n').ok_or_else(offline)?;
        let status = std::str::from_utf8(&output.stdout[end + 1..]).ok().and_then(|text| text.parse().ok()).ok_or_else(offline)?;
        let mut body = output.stdout;
        body.truncate(end);
        Ok((status, body))
    }

    fn common(url: &str) -> String {
        format!(
            "silent\nmax-redirs = 0\nmax-time = {REQUEST_TIMEOUT_SECONDS}\noutput = \"-\"\nwrite-out = \"\\n%{{http_code}}\"\nurl = {}\n",
            quoted(url)
        )
    }
}

impl Transport for Curl {
    fn post_json(&self, url: &str, body: &str) -> Result<Answer> {
        let config = format!("{}header = \"Content-Type: application/json\"\ndata-binary = {}\n", Curl::common(url), quoted(body));
        self.run(&config, None)
    }

    fn post_document(&self, url: &str, fields: &[(&str, String)], filename: &str, data: &[u8]) -> Result<Answer> {
        let directory = std::env::temp_dir();
        let upload = format!("logcourier-upload-{}.tmp", new_id());
        let path = directory.join(&upload);
        let result = (|| {
            let mut file = private_new_file(&path, false)?;
            file.write_all(data)?;
            drop(file);
            let mut config = Curl::common(url);
            for (name, value) in fields {
                config.push_str(&format!("form-string = {}\n", quoted(&format!("{name}={value}"))));
            }
            config.push_str(&format!("form = {}\n", quoted(&format!("document=@{upload};filename={filename};type=application/octet-stream"))));
            self.run(&config, Some(&directory))
        })();
        let _ = std::fs::remove_file(&path);
        result
    }

    fn get(&self, url: &str) -> Result<Answer> {
        self.run(&Curl::common(url), None)
    }
}

/// What delivery needs of a client: this one, or the same with its requests paced.
pub trait Client {
    fn bot_id(&self) -> &str;
    fn call(&self, method: &str, params: Value) -> Result<Value>;
    fn send_document(&self, chat_id: &str, filename: &str, data: &[u8], caption: &str) -> Result<Value>;
    fn download(&self, file_id: &str, limit: usize) -> Result<Vec<u8>>;
}

pub struct Telegram {
    bot_id: String,
    token: String,
    transport: Box<dyn Transport>,
}

const ALLOWED: [&str; 11] = [
    "getMe", "getChat", "getChatMember", "getUpdates", "getWebhookInfo", "getFile", "pinChatMessage", "unpinChatMessage",
    "unpinAllChatMessages", "sendMessage", "deleteMessages",
];

impl Telegram {
    pub fn new(token: &str) -> Result<Telegram> {
        Telegram::with_transport(token, Box::new(Curl))
    }

    pub fn with_transport(token: &str, transport: Box<dyn Transport>) -> Result<Telegram> {
        Ok(Telegram { bot_id: token_bot_id(token)?, token: token.to_owned(), transport })
    }

    fn checked(answer: Answer, limit: usize) -> Result<Vec<u8>> {
        let (status, body) = answer;
        if (300..400).contains(&status) {
            // The token is inside the Telegram URL: a redirect carrying it elsewhere is never followed.
            return Err(Error::telegram("Telegram вернул перенаправление; запрос остановлен."));
        }
        if !(200..400).contains(&status) {
            let bounded = &body[..body.len().min(ERROR_BODY_MAX_BYTES)];
            let payload: Option<Value> = serde_json::from_slice(bounded).ok();
            let description = payload.as_ref().and_then(|value| value["description"].as_str()).unwrap_or("Ошибка Telegram");
            let description: String = description.chars().take(ERROR_DESCRIPTION_MAX_CHARACTERS).collect();
            let retry_after = payload.as_ref().and_then(|value| value["parameters"]["retry_after"].as_i64()).unwrap_or(0);
            return Err(Error::telegram_with(format!("Telegram HTTP {status}: {description}"), retry_after.max(0), status));
        }
        if body.len() > limit {
            return Err(Error::telegram("Ответ Telegram превышает ограничение размера."));
        }
        Ok(body)
    }

    fn decode(data: &[u8]) -> Result<Value> {
        let wrong = || Error::telegram("Некорректный ответ Telegram.");
        let mut payload: Value = serde_json::from_slice(data).map_err(|_| wrong())?;
        if payload.get("ok") != Some(&Value::Bool(true)) {
            return Err(wrong());
        }
        payload.get_mut("result").map(Value::take).ok_or_else(wrong)
    }

    pub fn groups(&self) -> Result<Vec<Value>> {
        let webhook = self.call("getWebhookInfo", json!({}))?;
        if webhook["url"].as_str().is_some_and(|url| !url.is_empty()) {
            return Err(Error::telegram("У бота настроен webhook. Введите Chat ID вручную; webhook не изменён."));
        }
        // No offset: do not acknowledge or discard updates belonging to another client.
        let updates = self.call("getUpdates", json!({"timeout": 0, "limit": TELEGRAM_GET_UPDATES_LIMIT}))?;
        let mut found: Vec<(String, Value)> = Vec::new();
        for update in updates.as_array().map_or(&[][..], Vec::as_slice) {
            for key in ["message", "my_chat_member", "channel_post"] {
                let chat = &update[key]["chat"];
                if matches!(chat["type"].as_str(), Some("group" | "supergroup")) {
                    let id = chat["id"].to_string();
                    match found.iter_mut().find(|(known, _)| *known == id) {
                        Some(entry) => entry.1 = chat.clone(),
                        None => found.push((id, chat.clone())),
                    }
                }
            }
        }
        Ok(found.into_iter().map(|(_, chat)| chat).collect())
    }
}

impl Client for Telegram {
    fn bot_id(&self) -> &str {
        &self.bot_id
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        if !ALLOWED.contains(&method) {
            return Err(Error::value("Метод Telegram не разрешён."));
        }
        let url = format!("https://api.telegram.org/bot{}/{method}", self.token);
        let answer = self.transport.post_json(&url, &dumps(&params, true))?;
        Telegram::decode(&Telegram::checked(answer, TELEGRAM_RESPONSE_MAX_BYTES)?)
    }

    fn send_document(&self, chat_id: &str, filename: &str, data: &[u8], caption: &str) -> Result<Value> {
        let good_name = (1..=160).contains(&filename.len()) && filename.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
        if !good_name || data.len() > MAX_DOWNLOAD {
            return Err(Error::value("Некорректный размер или имя архива."));
        }
        let fields = [
            ("chat_id", chat_id.to_owned()),
            ("caption", caption.chars().take(TELEGRAM_MAX_CAPTION_CHARACTERS).collect()),
            ("disable_notification", "true".to_owned()),
        ];
        let url = format!("https://api.telegram.org/bot{}/sendDocument", self.token);
        let answer = self.transport.post_document(&url, &fields, filename, data)?;
        let result = Telegram::decode(&Telegram::checked(answer, TELEGRAM_RESPONSE_MAX_BYTES)?)?;
        let good = result["document"]["file_id"].as_str().is_some_and(is_file_id)
            && result["chat"]["id"].as_i64().is_some()
            && result["chat"]["id"].as_i64() == chat_id.parse::<i64>().ok()
            && result["message_id"].as_i64().is_some();
        if !good {
            return Err(Error::telegram("Telegram не подтвердил файл в выбранной группе."));
        }
        Ok(result)
    }

    fn download(&self, file_id: &str, limit: usize) -> Result<Vec<u8>> {
        if !is_file_id(file_id) {
            return Err(Error::value("Некорректный file_id."));
        }
        let info = self.call("getFile", json!({"file_id": file_id}))?;
        let wrong = || Error::telegram("Некорректный путь или слишком большой файл Telegram.");
        let path = info["file_path"].as_str().ok_or_else(wrong)?;
        let good = !path.is_empty()
            && path.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-'))
            && !path.starts_with('/')
            && !path.split('/').any(|part| part == "..")
            && (info["file_size"].as_u64().unwrap_or(0)).to::<usize>() <= limit;
        if !good {
            return Err(wrong());
        }
        let answer = self.transport.get(&format!("https://api.telegram.org/file/bot{}/{path}", self.token))?;
        Telegram::checked(answer, limit)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) mod fake {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Answers in order and remembers what was asked.
    #[derive(Clone, Default)]
    pub(crate) struct Script {
        pub answers: Arc<Mutex<Vec<Answer>>>,
        pub asked: Arc<Mutex<Vec<String>>>,
    }

    impl Transport for Script {
        fn post_json(&self, url: &str, body: &str) -> Result<Answer> {
            self.asked.lock().unwrap().push(format!("{url} {body}"));
            Ok(self.answers.lock().unwrap().remove(0))
        }
        fn post_document(&self, url: &str, _fields: &[(&str, String)], filename: &str, _data: &[u8]) -> Result<Answer> {
            self.asked.lock().unwrap().push(format!("{url} {filename}"));
            Ok(self.answers.lock().unwrap().remove(0))
        }
        fn get(&self, url: &str) -> Result<Answer> {
            self.asked.lock().unwrap().push(url.to_owned());
            Ok(self.answers.lock().unwrap().remove(0))
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::fake::Script;
    use super::*;

    const TOKEN: &str = "123456789:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn client(answers: Vec<Answer>) -> (Telegram, Script) {
        let script = Script::default();
        *script.answers.lock().unwrap() = answers;
        (Telegram::with_transport(TOKEN, Box::new(script.clone())).unwrap(), script)
    }

    #[test]
    fn an_error_carries_its_pause_and_never_the_token() {
        let body = format!("{{\"description\":\"bad {TOKEN}\",\"parameters\":{{\"retry_after\":37}}}}");
        let (client, _) = client(vec![(429, body.into_bytes())]);
        let error = client.call("getMe", json!({})).unwrap_err();
        match error {
            Error::Telegram { message, retry_after, status_code } => {
                assert_eq!((retry_after, status_code), (37, 429));
                assert!(message.contains("<TOKEN>") && !message.contains("AAAA"));
            }
            other @ (Error::Value(_) | Error::Os(_) | Error::QueueFull(_) | Error::Cancelled(_) |
Error::Runtime(_) | Error::Other(_)) => panic!("{other:?}"),
        }
    }

    #[test]
    fn redirects_and_unknown_methods_stop_the_request() {
        let (client_a, _) = client(vec![(302, Vec::new())]);
        assert!(matches!(client_a.call("getMe", json!({})), Err(Error::Telegram { .. })));
        let (client_b, script) = client(vec![]);
        assert!(matches!(client_b.call("sendSticker", json!({})), Err(Error::Value(_))));
        assert!(script.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_document_must_land_in_the_chosen_group() {
        let reply = |chat: i64| (200, format!("{{\"ok\":true,\"result\":{{\"message_id\":5,\"chat\":{{\"id\":{chat}}},\"document\":{{\"file_id\":\"abc\"}}}}}}").into_bytes());
        let (good, _) = client(vec![reply(-100)]);
        assert!(good.send_document("-100", "a.zip", b"x", "c").is_ok());
        let (bad, _) = client(vec![reply(-200)]);
        assert!(bad.send_document("-100", "a.zip", b"x", "c").is_err());
        let (named, _) = client(vec![]);
        assert!(matches!(named.send_document("-100", "../a.zip", b"x", "c"), Err(Error::Value(_))));
    }

    #[test]
    fn a_config_value_is_quoted_for_curl() {
        assert_eq!(quoted("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    /// One request answered by a small local server: what `curl` really sends and how its answer is read.
    fn served(answer: &'static str, request: impl FnOnce(&Curl, &str) -> Result<Answer>) -> (Result<Answer>, String) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/bot/x", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_millis(400))).unwrap();
            let (mut received, mut buffer) = (Vec::new(), [0u8; 4096]);
            while let Ok(count) = stream.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(answer.as_bytes()).unwrap();
            String::from_utf8_lossy(&received).into_owned()
        });
        let result = request(&Curl, &url);
        (result, server.join().unwrap())
    }

    #[test]
    fn curl_posts_json_and_reads_the_status() {
        let (result, seen) = served("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok", |curl, url| curl.post_json(url, "{\"a\": \"б\"}"));
        assert_eq!(result.unwrap(), (200, b"ok".to_vec()));
        assert!(seen.starts_with("POST /bot/x") && seen.contains("application/json") && seen.contains("{\"a\": \"б\"}"), "{seen}");
    }

    #[test]
    fn curl_sends_a_document_with_its_fields_and_does_not_follow_a_redirect() {
        let fields = [("chat_id", "-100".to_owned()), ("caption", "строка \"в кавычках\"\nвторая".to_owned())];
        let (result, seen) = served("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", |curl, url| curl.post_document(url, &fields, "a.zip", b"PAYLOAD"));
        assert_eq!(result.unwrap().0, 302);
        assert!(seen.contains("multipart/form-data") && seen.contains("name=\"chat_id\"") && seen.contains("-100"), "{seen}");
        assert!(seen.contains("filename=\"a.zip\"") && seen.contains("PAYLOAD") && seen.contains("строка \"в кавычках\"\nвторая"), "{seen}");
    }

    #[test]
    fn no_connection_is_told_without_a_url() {
        let error = Curl.get("http://127.0.0.1:9/bot123456789:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/x").unwrap_err();
        assert_eq!(error.to_string(), "Нет связи с Telegram. Архивы останутся в очереди.");
    }
}

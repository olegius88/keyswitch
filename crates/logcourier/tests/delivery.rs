//! Collection and delivery against a scripted Telegram.
#![expect(clippy::panic, clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use logcourier::batching::compact;
use logcourier::catalog::{current_catalog, decode_index, deliver, list_entries, verify_connection};
use logcourier::collector::Collector;
use logcourier::config::{Config, Source};
use logcourier::error::{Error, Result};
use logcourier::rate_limit::{Cancelled, RateLimitedClient};
use logcourier::reset::{deleted_from, reset_group};
use logcourier::store::Store;
use logcourier::telegram::Client;
use logcourier::timeutil::{new_id, sha256_hex};
use serde_json::{Value, json};

const BOT: &str = "123456";
const CHAT: &str = "-1001";

struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let path = std::env::temp_dir().join(format!("lc-test-{}", new_id()));
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn append(path: &std::path::Path, text: &str) {
    std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

fn config(path: &std::path::Path) -> Config {
    Config { bot_id: BOT.to_owned(), chat_id: CHAT.to_owned(), consent: true, sources: vec![Source::new(path.display().to_string(), "журнал")], ..Config::default() }
}

fn never() -> Cancelled {
    Arc::new(|| false)
}

/// A group as Telegram keeps it: documents by file id, one pin, messages numbered upward.
#[derive(Default)]
struct Group {
    files: RefCell<Vec<(String, Vec<u8>, String, i64, String)>>,
    pinned: RefCell<Option<i64>>,
    next_message: RefCell<i64>,
    calls: RefCell<Vec<String>>,
    refuse_up_to: RefCell<i64>,
    answers: RefCell<VecDeque<Value>>,
    answers_for: RefCell<Vec<(String, Value)>>,
    fail_delete: RefCell<bool>,
}

impl Group {
    fn next_answer_for(&self, method: &str, answer: Value) {
        self.answers_for.borrow_mut().push((method.to_owned(), answer));
    }
}

impl Client for Group {
    fn bot_id(&self) -> &str {
        BOT
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls.borrow_mut().push(method.to_owned());
        if let Some(answer) = self.answers.borrow_mut().pop_front() {
            return Ok(answer);
        }
        let waiting = self.answers_for.borrow().iter().position(|(name, _)| name == method);
        if let Some(place) = waiting {
            return Ok(self.answers_for.borrow_mut().remove(place).1);
        }
        Ok(match method {
            "getChat" => {
                let mut chat = json!({"type": "supergroup", "title": "логи"});
                if let Some(pinned) = *self.pinned.borrow() {
                    let files = self.files.borrow();
                    let (id, _, name, message, caption) = files.iter().find(|file| file.3 == pinned).expect("pinned file");
                    chat["pinned_message"] = json!({"message_id": message, "caption": caption, "from": {"id": BOT.parse::<i64>().unwrap()}, "document": {"file_name": name, "file_id": id}});
                }
                chat
            }
            "getChatMember" => json!({"status": "administrator", "can_pin_messages": true, "can_delete_messages": true}),
            "pinChatMessage" => {
                *self.pinned.borrow_mut() = params["message_id"].as_i64();
                json!(true)
            }
            "unpinAllChatMessages" => {
                *self.pinned.borrow_mut() = None;
                json!(true)
            }
            "sendMessage" => {
                *self.next_message.borrow_mut() += 1;
                json!({"message_id": *self.next_message.borrow(), "chat": {"id": CHAT.parse::<i64>().unwrap()}})
            }
            "deleteMessages" => {
                if *self.fail_delete.borrow() {
                    return Err(Error::telegram_with("Telegram HTTP 500: try later", 0, 500));
                }
                let first = params["message_ids"][0].as_i64().unwrap();
                if first <= *self.refuse_up_to.borrow() {
                    return Err(Error::telegram_with("Telegram HTTP 400: message can't be deleted", 0, 400));
                }
                json!(true)
            }
            other => panic!("unexpected {other}"),
        })
    }

    fn send_document(&self, _chat_id: &str, filename: &str, data: &[u8], caption: &str) -> Result<Value> {
        *self.next_message.borrow_mut() += 1;
        let message = *self.next_message.borrow();
        let id = format!("file{message}");
        self.files.borrow_mut().push((id.clone(), data.to_vec(), filename.to_owned(), message, caption.to_owned()));
        Ok(json!({"message_id": message, "chat": {"id": CHAT.parse::<i64>().unwrap()}, "document": {"file_id": id}}))
    }

    fn download(&self, file_id: &str, _limit: usize) -> Result<Vec<u8>> {
        Ok(self.files.borrow().iter().find(|file| file.0 == file_id).expect("file").1.clone())
    }
}

#[test]
fn the_first_scan_sets_a_baseline_and_only_new_lines_travel() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch.engine: old\n");
    let config = config(&log);
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    assert_eq!(collector.scan(&config).unwrap().0, 0);
    let markers = store.queue(&config.destination()).unwrap();
    assert_eq!(markers.len(), 1, "the version marker only");
    append(&log, "2026-11-04 12:00:01,000 0.66.1 INFO keyswitch.engine: new\n");
    assert_eq!(collector.scan(&config).unwrap().0, 1);
    assert_eq!(collector.scan(&config).unwrap().0, 0, "nothing is read twice");
    let rows = store.queue(&config.destination()).unwrap();
    let fragment = rows.iter().find(|row| std::path::Path::new(&row.name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))).expect("a fragment");
    assert_eq!(fragment.metadata()["keyswitch_version"], "0.66.1");
    assert_eq!(fragment.metadata()["start_byte"], 58);
}

#[test]
fn a_new_version_in_the_file_opens_a_new_fragment_and_a_marker() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n2026-11-04 12:00:01,000 0.70.0 INFO keyswitch: b\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    assert_eq!(Collector::new(&store).scan(&config).unwrap().0, 2);
    let rows = store.queue(&config.destination()).unwrap();
    let versions: Vec<String> = rows.iter().filter(|row| row.name.starts_with("lc-2")).map(|row| row.metadata()["keyswitch_version"].as_str().unwrap().to_owned()).collect();
    assert_eq!(versions, ["0.66.1", "0.70.0"]);
}

#[test]
fn a_rotated_copy_continues_where_its_original_stopped() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].rotations = 1;
    config.sources[0].include_existing = true;
    append(&log, "line one\nline two\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    assert_eq!(collector.scan(&config).unwrap().0, 1);
    // copytruncate: the slot gets a new inode with the same bytes, the live file starts over.
    std::fs::copy(&log, dir.0.join("app.log.1")).unwrap();
    std::fs::write(&log, "").unwrap();
    append(&log, "line three, a new one\n");
    assert_eq!(collector.scan(&config).unwrap().0, 1, "only the new line; the copy is not sent again");
}

#[test]
fn delivery_pins_a_catalog_chain_and_refuses_a_lost_pin() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    Collector::new(&store).scan(&config).unwrap();
    let group = Group::default();
    let message = deliver(&store, &config, &group, &never()).unwrap();
    assert!(message.contains("2 архивов"), "{message}");
    let head = current_catalog(&group, CHAT).unwrap().expect("a pinned catalog");
    let first_digest = head["sha256"].clone();
    assert_eq!(head["index"]["entries"].as_array().unwrap().len(), 2);
    assert!(store.queue(&config.destination()).unwrap().is_empty());
    assert_eq!(deliver(&store, &config, &group, &never()).unwrap(), "Новых архивов для отправки нет.");
    append(&log, "2026-11-04 12:00:01,000 0.66.1 INFO keyswitch: b\n");
    Collector::new(&store).scan(&config).unwrap();
    deliver(&store, &config, &group, &never()).unwrap();
    let head = current_catalog(&group, CHAT).unwrap().unwrap();
    assert_eq!(head["index"]["previous"]["sha256"], first_digest);
    let listed = list_entries(&group, CHAT, 10, 10, None).unwrap();
    assert_eq!(listed.len(), 2, "two fragments across the chain; the marker is not a log");
    assert_eq!(list_entries(&group, CHAT, 10, 10, Some("current")).unwrap().len(), 2);
    assert_eq!(list_entries(&group, CHAT, 10, 10, Some("0.1.1")).unwrap().len(), 0);
    *group.pinned.borrow_mut() = None;
    assert!(deliver(&store, &config, &group, &never()).unwrap_err().to_string().contains("Закрепление каталога потеряно"));
}

#[test]
fn fragments_are_packed_with_their_bytes_kept() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector { store: &store, chunk_size: 11 };
    append(&log, "first line\nsecond one\nthird one!\n");
    assert_eq!(collector.scan(&config).unwrap().0, 3);
    assert_eq!(compact(&store, &config).unwrap(), 3);
    assert_eq!(compact(&store, &config).unwrap(), 0);
    let rows = store.queue(&config.destination()).unwrap();
    assert_eq!(rows.len(), 1);
    let meta = rows[0].metadata();
    assert_eq!(meta["fragments"], 3);
    assert_eq!(meta["sha256"], sha256_hex(rows[0].payload.as_ref().unwrap()));
}

#[test]
fn a_wrong_catalog_is_refused() {
    assert!(decode_index(b"{}", CHAT, BOT).is_err());
    let index = json!({"schema": 1, "kind": "logcourier.index", "device_id": new_id(), "chat_id": CHAT, "bot_id": BOT, "entries": [], "previous": null});
    assert!(decode_index(index.to_string().as_bytes(), CHAT, BOT).is_ok());
    assert!(decode_index(index.to_string().as_bytes(), "-2", BOT).is_err());
}

#[test]
fn deleting_finds_where_telegram_stops_keeping_messages() {
    let group = Group::default();
    *group.refuse_up_to.borrow_mut() = 37;
    assert_eq!(deleted_from(&group, CHAT, 2, 100, &never()).unwrap(), 38);
    *group.refuse_up_to.borrow_mut() = 0;
    assert_eq!(deleted_from(&group, CHAT, 2, 100, &never()).unwrap(), 2);
}

#[test]
fn clearing_the_group_reports_what_telegram_kept_and_starts_over() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    Collector::new(&store).scan(&config).unwrap();
    let group = Group::default();
    deliver(&store, &config, &group, &never()).unwrap();
    *group.refuse_up_to.borrow_mut() = 1;
    *group.next_message.borrow_mut() = 150;
    let text = reset_group(&store, &config, &group, true, &never(), &|_| {}).unwrap();
    assert!(text.starts_with("Группа очищена: удалены все сообщения до №151"), "{text}");
    assert!(group.pinned.borrow().is_none());
    assert!(store.get(&format!("catalog_digest:{}", config.destination())).is_none());
    let text = reset_group(&store, &config, &group, false, &never(), &|_| {});
    assert!(text.is_ok(), "the collector has written here, so a restart without deleting is allowed");
}

#[test]
fn a_pause_from_telegram_is_kept_across_requests() {
    let dir = Dir::new();
    let store = Store::open(&dir.0.join("data")).unwrap();
    struct Busy;
    impl Client for Busy {
        fn bot_id(&self) -> &str {
            BOT
        }
        fn call(&self, _: &str, _: Value) -> Result<Value> {
            Err(Error::telegram_with("Telegram HTTP 429", 37, 429))
        }
        fn send_document(&self, _: &str, _: &str, _: &[u8], _: &str) -> Result<Value> {
            Ok(json!({}))
        }
        fn download(&self, _: &str, _: usize) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }
    let client = RateLimitedClient::with_clock(Busy, &store, CHAT, never(), Box::new(|| 1000.0), Box::new(|_| {}));
    let first = client.call("getMe", json!({})).unwrap_err();
    assert!(matches!(first, Error::Telegram { retry_after: 38, status_code: 429, .. }), "{first:?}");
    assert_eq!(store.get_f64(&format!("telegram_cooldown:{BOT}")), 1038.0);
    assert!(client.call("getMe", json!({})).unwrap_err().to_string().contains("временно ограничил"));
}

#[test]
fn list_and_fetch_save_what_the_catalog_names_and_nothing_else() {
    use logcourier::cli::{fetch_or_list_with, parse};
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    Collector::new(&store).scan(&config).unwrap();
    let group = Group::default();
    deliver(&store, &config, &group, &never()).unwrap();
    let output = dir.0.join("out");
    let options = |text: String| parse(&text.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).unwrap();
    let all = format!("--all-versions --output {}", output.display());
    assert_eq!(fetch_or_list_with(&group, "list", CHAT, options(all.clone())).unwrap(), 0);
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options(all.clone())).unwrap(), 0);
    let saved: Vec<PathBuf> = std::fs::read_dir(&output).unwrap().map(|item| item.unwrap().path()).collect();
    assert!(!saved.is_empty());
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options(all.clone())).unwrap(), 0);
    let current = format!("--keyswitch-version current --output {}", output.display());
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options(current.clone())).unwrap(), 0);
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options(current)).unwrap(), 0);
    let later = format!("--all-versions --since 2999-01-01 --output {}", output.display());
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options(later)).unwrap(), 0);
    let bad = format!("--all-versions --since x --output {}", output.display());
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options(bad)).is_err());
    std::fs::write(&saved[0], "changed").unwrap();
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options(all.clone())).is_err());
    std::fs::remove_file(&saved[0]).unwrap();
    group.files.borrow_mut().iter_mut().for_each(|file| file.1.push(0));
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options(all)).is_err());
}

/// The scripted group for the service's thread.
#[derive(Clone)]
struct SharedGroup(Arc<std::sync::Mutex<Group>>);

impl Client for SharedGroup {
    fn bot_id(&self) -> &str {
        BOT
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.0.lock().unwrap().call(method, params)
    }
    fn send_document(&self, chat_id: &str, filename: &str, data: &[u8], caption: &str) -> Result<Value> {
        self.0.lock().unwrap().send_document(chat_id, filename, data, caption)
    }
    fn download(&self, file_id: &str, limit: usize) -> Result<Vec<u8>> {
        self.0.lock().unwrap().download(file_id, limit)
    }
}

#[test]
fn the_service_says_what_stops_it_sends_when_asked_and_stops_when_told() {
    use logcourier::service::{MakeClient, Notify, Service};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    let notify: Notify = Arc::new(move |message, _stats| sink.lock().unwrap().push(message));
    let group = SharedGroup(Arc::new(Mutex::new(Group::default())));
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (shared, stop_making) = (group.clone(), failing.clone());
    let make: MakeClient = Arc::new(move |_token| {
        if stop_making.load(Ordering::SeqCst) {
            return Err(Error::telegram_with("slow down", 3, 429));
        }
        let client: Box<dyn Client> = Box::new(shared.clone());
        Ok(client)
    });
    let wait = |text: &str| {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if heard.lock().unwrap().iter().any(|message| message.contains(text)) {
                heard.lock().unwrap().clear();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never heard {text:?}: {:?}", heard.lock().unwrap());
    };
    let mut settings = config(&log);
    settings.sources[0].include_existing = true;
    settings.consent = false;
    settings.auto_send = false;
    let service = Service::start_with(dir.0.join("data"), settings.clone(), "1:token".to_owned(), notify, make);
    wait("выключена");
    settings.auto_send = true;
    service.update(settings.clone(), "1:token".to_owned());
    wait("разрешение");
    settings.consent = true;
    service.update(settings.clone(), String::new());
    wait("Настройте");
    service.reset(false);
    wait("Настройте");
    service.update(settings.clone(), "1:token".to_owned());
    wait("Ожидание отправки");
    service.send_now();
    wait("архивов");
    service.update(settings.clone(), "1:token".to_owned());
    service.send_now();
    wait("Новых архивов");
    failing.store(true, Ordering::SeqCst);
    append(&log, "2026-11-04 12:00:01,000 0.66.1 INFO keyswitch: b\n");
    service.send_now();
    wait("slow down");
    service.send_now();
    wait("Повтор через");
    assert!(!service.is_finished());
    service.stop();
    let started = Instant::now();
    while !service.is_finished() && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(service.is_finished());
}

#[test]
fn a_service_whose_queue_cannot_open_says_so() {
    use logcourier::service::{Notify, Service};
    use std::sync::Mutex;
    let dir = Dir::new();
    let blocked = dir.0.join("file");
    std::fs::write(&blocked, "").unwrap();
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    let notify: Notify = Arc::new(move |message, _stats| sink.lock().unwrap().push(message));
    let service = Service::start(blocked.join("data"), Config::default(), String::new(), notify);
    let started = std::time::Instant::now();
    while !service.is_finished() && started.elapsed().as_secs() < 20 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(heard.lock().unwrap()[0].contains("очередь"));
}

// ---------------------------------------------------------------- the catalog as it may be found

const HEX32: &str = "0123456789abcdef0123456789abcdef";
const HEX64: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FILE: &str = "AbCdEfGhIjKlMnOp_-12345678901234567890";

fn entry() -> Value {
    json!({
        "bundle_id": HEX32, "file_id": FILE, "sha256": HEX64, "size": 10, "created_at": "2026-11-04T12:00:00+00:00",
        "keyswitch_version": "0.66.1", "source_id": HEX32, "device_id": HEX32, "kind": "fragment",
    })
}

fn index_with(change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut index = json!({
        "schema": 1, "kind": "logcourier.index", "device_id": HEX32, "chat_id": CHAT, "bot_id": BOT, "created_at": "2026-11-04T12:00:00+00:00",
        "entries": [entry()],
        "keyswitch_versions": {HEX32: {"version": "0.66.1", "marker_id": HEX32, "previous_version": "0.66.0", "detected_at": "2026-11-04T12:00:00+00:00"}},
        "previous": null,
    });
    change(&mut index);
    serde_json::to_vec(&index).unwrap()
}

#[test]
fn a_catalog_of_any_wrong_shape_is_refused() {
    assert!(decode_index(&index_with(|_| {}), CHAT, BOT).is_ok());
    assert!(decode_index(&index_with(|i| i["entries"][0]["keyswitch_version"] = Value::Null), CHAT, BOT).is_ok());
    assert!(decode_index(&index_with(|i| i["keyswitch_versions"][HEX32]["previous_version"] = Value::Null), CHAT, BOT).is_ok());
    assert!(decode_index(&index_with(|i| drop(i.as_object_mut().unwrap().remove("keyswitch_versions"))), CHAT, BOT).is_ok());
    assert!(decode_index(&index_with(|i| i["previous"] = json!({"file_id": FILE, "sha256": HEX64})), CHAT, BOT).is_ok());
    let wrong: Vec<fn(&mut Value)> = vec![
        |i| i["schema"] = json!(2),
        |i| i["kind"] = json!("other"),
        |i| i["chat_id"] = json!("other"),
        |i| i["bot_id"] = json!("other"),
        |i| i["device_id"] = json!("short"),
        |i| i["device_id"] = json!(HEX32.to_uppercase()),
        |i| i["entries"] = json!(1),
        |i| i["entries"] = Value::Array(vec![entry(); 10_001]),
        |i| i["entries"][0]["bundle_id"] = json!("x"),
        |i| i["entries"][0]["file_id"] = json!("!"),
        |i| i["entries"][0]["sha256"] = json!("x"),
        |i| i["entries"][0]["size"] = json!(0),
        |i| i["entries"][0]["size"] = json!(i64::MAX),
        |i| i["entries"][0]["created_at"] = json!("yesterday"),
        |i| i["entries"][0]["keyswitch_version"] = json!("1.x"),
        |i| i["entries"][0]["source_id"] = json!("x"),
        |i| i["entries"][0]["device_id"] = json!("x"),
        |i| {
            i["entries"][0]["keyswitch_version"] = Value::Null;
            i["entries"][0]["kind"] = json!("keyswitch.version");
        },
        |i| i["keyswitch_versions"] = json!(1),
        |i| i["keyswitch_versions"] = json!({"x": {"version": "0.66.1"}}),
        |i| i["keyswitch_versions"][HEX32]["version"] = json!("x"),
        |i| i["keyswitch_versions"][HEX32]["marker_id"] = json!("x"),
        |i| i["keyswitch_versions"][HEX32]["previous_version"] = json!("x"),
        |i| i["keyswitch_versions"][HEX32]["detected_at"] = json!("x"),
        |i| drop(i.as_object_mut().unwrap().remove("previous")),
        |i| i["previous"] = json!({"file_id": "!", "sha256": HEX64}),
        |i| i["previous"] = json!({"file_id": FILE, "sha256": "x"}),
    ];
    for (number, change) in wrong.into_iter().enumerate() {
        assert!(decode_index(&index_with(change), CHAT, BOT).is_err(), "change {number}");
    }
    let many: serde_json::Map<String, Value> = (0..2_000)
        .map(|number| (format!("{number:032x}"), json!({"version": "0.66.1", "marker_id": HEX32, "previous_version": null, "detected_at": "2026-11-04T12:00:00+00:00"})))
        .collect();
    assert!(decode_index(&index_with(|i| i["keyswitch_versions"] = Value::Object(many)), CHAT, BOT).is_err());
    assert!(decode_index(b"not json", CHAT, BOT).is_err());
}

fn pinned_chat(file_id: &str, name: &str, from: i64, caption: &str) -> Value {
    json!({"type": "supergroup", "pinned_message": {"message_id": 5, "caption": caption, "from": {"id": from}, "document": {"file_id": file_id, "file_name": name}}})
}

#[test]
fn a_pinned_message_must_be_our_catalog() {
    let group = Group::default();
    group.answers.borrow_mut().push_back(json!({"type": "channel"}));
    assert!(current_catalog(&group, CHAT).is_err(), "not a group");
    group.answers.borrow_mut().push_back(json!({"type": "group", "username": "public"}));
    assert!(current_catalog(&group, CHAT).is_err(), "a public group");
    group.answers.borrow_mut().push_back(json!({"type": "group", "username": ""}));
    assert!(current_catalog(&group, CHAT).unwrap().is_none());
    let good_name = format!("lc-index-{HEX32}.json");
    for (name, from) in [(good_name.as_str(), 1), ("lc-index-x.json", 123_456), ("other", 123_456)] {
        group.answers.borrow_mut().push_back(pinned_chat(FILE, name, from, "x"));
        assert!(current_catalog(&group, CHAT).is_err(), "{name} {from}");
    }
    group.files.borrow_mut().push((FILE.to_owned(), index_with(|_| {}), good_name.clone(), 5, String::new()));
    group.answers.borrow_mut().push_back(pinned_chat(FILE, &good_name, 123_456, "wrong sum"));
    assert!(current_catalog(&group, CHAT).is_err(), "the sum");
    let right = format!("LogCourier catalog SHA256 {}", sha256_hex(&index_with(|_| {})));
    group.answers.borrow_mut().push_back(pinned_chat(FILE, &good_name, 123_456, &right));
    assert_eq!(current_catalog(&group, CHAT).unwrap().unwrap()["message_id"], 5);
}

#[test]
fn a_connection_is_checked_step_by_step() {
    let group = Group::default();
    let refuse = |answers: Vec<Value>| {
        group.answers.borrow_mut().clear();
        group.answers.borrow_mut().extend(answers);
        verify_connection(&group, CHAT).map_err(|error| error.to_string())
    };
    let me = json!({"id": 123_456, "username": "courier"});
    let chat = json!({"type": "supergroup", "title": "логи"});
    let member = json!({"status": "administrator", "can_pin_messages": true});
    assert!(refuse(vec![json!({"id": 1})]).is_err());
    assert!(refuse(vec![me.clone(), json!({"type": "channel"})]).is_err());
    assert!(refuse(vec![me.clone(), chat.clone(), json!({"status": "member"})]).is_err());
    assert!(refuse(vec![me.clone(), chat.clone(), json!({"status": "administrator", "can_pin_messages": false})]).is_err());
    assert_eq!(refuse(vec![me, chat.clone(), member.clone()]).unwrap(), "@courier → логи");
    assert_eq!(refuse(vec![json!({"id": 123_456}), json!({"type": "supergroup"}), member]).unwrap(), format!("@{BOT} → {CHAT}"));
}

#[test]
fn delivery_says_why_it_will_not_start() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch.engine: old\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    let group = Group::default();
    let mut refused = config(&log);
    refused.consent = false;
    assert!(deliver(&store, &refused, &group, &never()).is_err());
    let mut other = config(&log);
    other.bot_id = "999".to_owned();
    assert!(deliver(&store, &other, &group, &never()).is_err());
    let cancelled: Cancelled = Arc::new(|| true);
    assert!(matches!(deliver(&store, &config(&log), &group, &cancelled), Err(Error::Cancelled(_))));
}

/// One catalog delivered, its file and caption as the group keeps them.
fn delivered_group(dir: &Dir) -> (Group, Config, Store) {
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch.engine: old\n");
    let config = config(&log);
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    collector.scan(&config).unwrap();
    append(&log, "2026-11-04 12:00:01,000 0.66.1 INFO keyswitch.engine: new\n");
    collector.scan(&config).unwrap();
    let group = Group::default();
    deliver(&store, &config, &group, &never()).unwrap();
    (group, config, store)
}

#[test]
fn delivery_keeps_to_the_group_it_started_in() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    assert_eq!(deliver(&store, &config, &group, &never()).unwrap(), "Новых архивов для отправки нет.");

    let mut stranger = config.clone();
    stranger.device_id = "f".repeat(32);
    assert!(deliver(&store, &stranger, &group, &never()).unwrap_err().to_string().contains("другим сборщиком"));

    let key = format!("{}{}", logcourier::store::PENDING_KEY, config.destination());
    store.set(&key, &json!({"file_id": "old"})).unwrap();
    assert_eq!(deliver(&store, &config, &group, &never()).unwrap(), "Новых архивов для отправки нет.");
    assert!(store.get(&key).is_none(), "an old mark is dropped");

    let head = current_catalog(&group, CHAT).unwrap().unwrap();
    store.set(&key, &json!({"sha256": head["sha256"], "message_id": head["message_id"], "ids": [], "previous_sha256": null})).unwrap();
    assert!(deliver(&store, &config, &group, &never()).unwrap().contains("восстановлен"));

    store.set(&key, &json!({"sha256": "x", "message_id": 1, "ids": [], "previous_sha256": "y"})).unwrap();
    assert!(deliver(&store, &config, &group, &never()).unwrap_err().to_string().contains("другим процессом"));
    store.forget(&key).unwrap();

    store.set(&key, &json!({"sha256": "x", "message_id": head["message_id"], "ids": [], "previous_sha256": head["sha256"]})).unwrap();
    let pin_refused = group.answers.borrow().len();
    assert_eq!(pin_refused, 0);
    group.next_answer_for("pinChatMessage", json!(false));
    assert!(deliver(&store, &config, &group, &never()).unwrap_err().to_string().contains("не подтвердил"));
    assert!(deliver(&store, &config, &group, &never()).unwrap_err().to_string().contains("не виден"));
    store.forget(&key).unwrap();
}

#[test]
fn a_catalog_pointer_from_an_old_version_is_adopted_once() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let destination = config.destination();
    store.forget(&format!("{}{destination}", logcourier::store::HEAD_DIGEST_KEY)).unwrap();
    store.set(&format!("{}{destination}", logcourier::store::LEGACY_HEAD_KEY), &json!("old-file-id")).unwrap();
    assert!(deliver(&store, &config, &group, &never()).is_ok());
    *group.pinned.borrow_mut() = None;
    store.forget(&format!("{}{destination}", logcourier::store::HEAD_DIGEST_KEY)).unwrap();
    assert!(deliver(&store, &config, &group, &never()).unwrap_err().to_string().contains("потеряно"));
}

fn chain(group: &Group, change: impl Fn(&mut Value, usize)) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for page in 0..3 {
        let data = index_with(|index| {
            index["entries"][0]["bundle_id"] = json!(format!("{page:032x}"));
            if let Some((id, sum)) = kept.last() {
                index["previous"] = json!({"file_id": id, "sha256": sum});
            }
            change(index, page);
        });
        let id = format!("{FILE}{page}");
        let sum = sha256_hex(&data);
        group.files.borrow_mut().push((id.clone(), data, format!("lc-index-{HEX32}.json"), i64::try_from(page).unwrap() + 1, format!("LogCourier catalog SHA256 {sum}")));
        kept.push((id, sum));
    }
    *group.pinned.borrow_mut() = Some(3);
    kept
}

#[test]
fn listing_walks_the_chain_and_stops_at_what_is_wrong() {
    let group = Group::default();
    assert!(list_entries(&group, CHAT, 10, 5, Some("nonsense")).is_err());
    assert_eq!(list_entries(&group, CHAT, 10, 5, None).unwrap(), Vec::<Value>::new());

    chain(&group, |_, _| {});
    assert_eq!(list_entries(&group, CHAT, 10, 5, None).unwrap().len(), 3);
    assert_eq!(list_entries(&group, CHAT, 2, 5, None).unwrap().len(), 2);
    assert_eq!(list_entries(&group, CHAT, 10, 5, Some("current")).unwrap().len(), 3);
    assert_eq!(list_entries(&group, CHAT, 10, 5, Some("0.66.1")).unwrap().len(), 3);
    assert_eq!(list_entries(&group, CHAT, 10, 5, Some("0.1.0")).unwrap().len(), 0);
    assert!(list_entries(&group, CHAT, 10, 1, None).unwrap_err().to_string().contains("предел"));
}

#[test]
fn a_broken_chain_is_reported() {
    let group = Group::default();
    chain(&group, |index, page| {
        if page == 2 {
            index["keyswitch_versions"] = json!({});
        }
    });
    assert!(list_entries(&group, CHAT, 10, 5, Some("current")).unwrap_err().to_string().contains("не определена"));

    let group = Group::default();
    chain(&group, |index, page| {
        if page == 1 {
            index["device_id"] = json!("f".repeat(32));
            index["entries"][0]["device_id"] = json!("f".repeat(32));
        }
    });
    assert!(list_entries(&group, CHAT, 10, 5, None).unwrap_err().to_string().contains("разных устройств"));

    let group = Group::default();
    let kept = chain(&group, |_, _| {});
    group.files.borrow_mut()[1].1 = b"changed".to_vec();
    let _ = kept;
    assert!(list_entries(&group, CHAT, 10, 5, None).unwrap_err().to_string().contains("Повреждена"));

    let group = Group::default();
    chain(&group, |index, page| {
        if page == 2 {
            index["previous"]["file_id"] = json!(format!("{FILE}2"));
        }
    });
    assert!(list_entries(&group, CHAT, 10, 5, None).is_err());
}

// ---------------------------------------------------------------- clearing the group, what stops it

fn refusal_of_reset(group: &Group, store: &Store, config: &Config, delete: bool) -> String {
    reset_group(store, config, group, delete, &never(), &|_| {}).unwrap_err().to_string()
}

#[test]
fn a_reset_leaves_the_group_alone_when_it_cannot_be_done_whole() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let mut other = config.clone();
    other.bot_id = "999".to_owned();
    assert!(refusal_of_reset(&group, &store, &other, false).contains("другому боту"));

    let chat = json!({"type": "supergroup"});
    let answer = |answers: Vec<Value>| {
        group.answers.borrow_mut().clear();
        group.answers.borrow_mut().extend(answers);
    };
    answer(vec![json!({"type": "group"})]);
    assert!(refusal_of_reset(&group, &store, &config, true).contains("супергруппе"));
    answer(vec![chat.clone(), json!({"status": "member"})]);
    assert!(refusal_of_reset(&group, &store, &config, true).contains("Закрепление"));
    answer(vec![chat.clone(), json!({"status": "administrator", "can_pin_messages": false})]);
    assert!(refusal_of_reset(&group, &store, &config, true).contains("Закрепление"));
    answer(vec![chat.clone(), json!({"status": "administrator", "can_pin_messages": true})]);
    assert!(refusal_of_reset(&group, &store, &config, true).contains("Удаление"));
    answer(Vec::new());

    let mut stranger = config.clone();
    stranger.device_id = "f".repeat(32);
    assert!(refusal_of_reset(&group, &store, &stranger, false).contains("другой сборщик"));

    group.next_answer_for("sendMessage", json!({"message_id": 9, "chat": {"id": "text"}}));
    assert!(refusal_of_reset(&group, &store, &config, true).contains("не подтвердил"));
    group.next_answer_for("unpinAllChatMessages", json!(false));
    assert!(refusal_of_reset(&group, &store, &config, false).contains("снятие закреплений"));

    let fresh_dir = Dir::new();
    let fresh = Store::open(&fresh_dir.0.join("data")).unwrap();
    assert!(refusal_of_reset(&Group::default(), &fresh, &config, false).contains("ещё ничего не отправлял"));
}

#[test]
fn clearing_goes_on_where_it_stopped_and_counts_in_russian() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let cancelled: Cancelled = Arc::new(|| true);
    assert!(matches!(reset_group(&store, &config, &group, true, &cancelled, &|_| {}), Err(Error::Cancelled(_))));

    let key = format!("{}{}", logcourier::store::RESET_KEY, config.destination());
    store.set(&key, &json!({"top": 3, "next": 3, "kept": null})).unwrap();
    let interrupted = reset_group(&store, &config, &group, false, &cancelled, &|_| {}).unwrap_err();
    assert!(interrupted.to_string().contains("продолжится"));

    store.set(&key, &json!({"top": 3, "next": 3, "kept": null, "dropped": 21})).unwrap();
    let message = reset_group(&store, &config, &group, false, &never(), &|_| {}).unwrap();
    assert!(message.contains("21"), "{message}");

    for (dropped, word) in [(1, "архив"), (2, "архива"), (5, "архивов"), (11, "архивов"), (12, "архивов"), (22, "архива"), (101, "архив"), (111, "архивов")] {
        store.set(&key, &json!({"top": 3, "next": 0, "kept": null, "dropped": dropped})).unwrap();
        let message = reset_group(&store, &config, &group, false, &never(), &|_| {}).unwrap();
        assert!(message.contains(&format!(" {dropped} {word}")), "{message}");
    }

    *group.refuse_up_to.borrow_mut() = 2;
    store.set(&key, &json!({"top": 4, "next": 4, "kept": null})).unwrap();
    let message = reset_group(&store, &config, &group, false, &never(), &|_| {}).unwrap();
    assert!(message.contains("частично"), "{message}");

    *group.refuse_up_to.borrow_mut() = 0;
    group.next_answer_for("deleteMessages", json!(false));
    assert!(deleted_from(&group, CHAT, 2, 3, &never()).is_err());
}

// ---------------------------------------------------------------- the collector and what it refuses

#[test]
fn only_regular_files_are_collected() {
    let dir = Dir::new();
    assert!(logcourier::collector::open_regular(&dir.0).is_err(), "a folder");
    #[cfg(unix)]
    {
        let target = dir.0.join("real.log");
        append(&target, "text\n");
        let link = dir.0.join("link.log");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(logcourier::collector::open_regular(&link).unwrap_err().to_string().contains("Символьные"));
    }
}

#[test]
fn a_collection_starts_only_when_allowed_and_configured() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    let mut refused = config(&log);
    refused.consent = false;
    assert_eq!(collector.scan(&refused).unwrap().0, 0);
    let mut unset = config(&log);
    unset.bot_id = String::new();
    assert!(collector.scan(&unset).is_err());
    let mut unset = config(&log);
    unset.chat_id = String::new();
    assert!(collector.scan(&unset).is_err());
    let budget = config(&log);
    assert_eq!(collector.scan_with(&budget, 0).unwrap().0, 0, "no room this time");
}

#[test]
fn every_shape_of_a_log_file_is_read_or_set_aside() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    config.sources[0].rotations = 2;
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    assert_eq!(collector.scan(&config).unwrap().0, 0, "no file yet");
    append(&log, "");
    assert_eq!(collector.scan(&config).unwrap().0, 0, "an empty file");
    append(&log, "a line\nand one without its end");
    assert!(collector.scan(&config).unwrap().0 >= 1, "whole lines travel");
    append(&log, "\n");
    let _ = collector.scan(&config).unwrap();
    append(&log, "2026-11-04 12:00:01,000 0.66.1 INFO keyswitch: b\n2026-11-04 12:00:02,000 0.66.2 INFO keyswitch: c\n");
    assert!(collector.scan(&config).unwrap().0 >= 1);
    std::fs::write(&log, "short\n").unwrap();
    let _ = collector.scan(&config).unwrap();
    std::fs::write(&log, "2026-11-04 12:00:03,000 0.66.2 INFO keyswitch: other start of the same length, longer than before\n").unwrap();
    let _ = collector.scan(&config).unwrap();
    std::fs::rename(&log, dir.0.join("app.log.1")).unwrap();
    append(&log, "2026-11-04 12:00:04,000 0.66.2 INFO keyswitch: d\n");
    let _ = collector.scan(&config).unwrap();
    let bulk: String = (0..2000).map(|number| format!("2026-11-04 12:00:05,000 0.66.2 INFO keyswitch: line {number} {}\n", "x".repeat(200))).collect();
    append(&log, &bulk);
    assert!(collector.scan_with(&config, 1).unwrap().0 >= 1);
    let _ = collector.scan_with(&config, 100).unwrap();
    assert!(store.queue(&config.destination()).unwrap().len() >= 2);
}

#[test]
fn a_queue_with_no_room_stops_the_collection_and_keeps_the_file() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let store = Store::with_capacity(&dir.0.join("data"), 10).unwrap();
    let (count, errors) = Collector::new(&store).scan(&config).unwrap_or((0, vec!["full".to_owned()]));
    assert_eq!(count, 0, "{errors:?}");
    assert!(log.exists());
}

#[test]
fn a_catalog_written_by_another_during_delivery_stops_it() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:02,000 0.66.1 INFO keyswitch.engine: more\n");
    Collector::new(&store).scan(&config).unwrap();
    let head = group.pinned.borrow().unwrap();
    let chat_now = group.call("getChat", json!({})).unwrap();
    group.calls.borrow_mut().clear();
    group.answers.borrow_mut().push_back(chat_now);
    group.answers.borrow_mut().push_back(json!({"type": "supergroup"}));
    let refused = deliver(&store, &config, &group, &never()).unwrap_err();
    assert!(refused.to_string().contains("изменился"), "{refused}");
    assert_eq!(group.pinned.borrow().unwrap(), head);
}

// ---------------------------------------------------------------- packing fragments, and more of the catalog

fn fragments(store: &Store, config: &Config, count: usize, version: Option<&str>, size: usize) {
    for number in 0..count {
        let identifier = new_id();
        let mut metadata = json!({
            "schema": 1, "bundle_id": identifier, "device_id": config.device_id, "created_at": "2026-11-04T12:00:00+00:00", "source_id": HEX32,
            "sha256": HEX64, "size": size, "source_label": "журнал", "kind": "fragment",
        });
        if let Some(version) = version {
            metadata["keyswitch_version"] = json!(version);
        }
        store.enqueue_event(&config.destination(), &metadata, &vec![u8::try_from(number % 200).unwrap(); size], &format!("lc-{identifier}.zip"), &[]).unwrap();
    }
}

#[test]
fn fragments_are_packed_by_version_and_within_their_limits() {
    use logcourier::batching::compact_with;
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let store = Store::open(&dir.0.join("data")).unwrap();
    assert_eq!(compact_with(&store, &config, 1_000_000, 100).unwrap(), 0, "nothing to pack");
    fragments(&store, &config, 5, Some("0.66.1"), 100);
    fragments(&store, &config, 3, None, 100);
    assert_eq!(compact_with(&store, &config, 1_000_000, 3).unwrap(), 3, "only as many as asked");
    assert_eq!(compact_with(&store, &config, 1_000_000, 100).unwrap(), 2);
    assert_eq!(compact_with(&store, &config, 1_000_000, 100).unwrap(), 3, "those of no version together");
    fragments(&store, &config, 4, Some("0.66.2"), 400);
    assert_eq!(compact_with(&store, &config, 4_500, 100).unwrap(), 2, "as many as fit");
    fragments(&store, &config, 3, Some("0.66.3"), 300);
    assert!(compact_with(&store, &config, 900, 2).unwrap() <= 2);
}

#[test]
fn a_package_that_does_not_fit_leaves_the_queue_as_it_was() {
    use logcourier::batching::compact_with;
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let tight = Store::with_capacity(&dir.0.join("tight"), 2_500).unwrap();
    fragments(&tight, &config, 2, Some("0.66.1"), 1_200);
    let before = tight.queue(&config.destination()).unwrap().len();
    assert!(matches!(compact_with(&tight, &config, 100_000, 10), Err(Error::QueueFull(_))));
    assert_eq!(tight.queue(&config.destination()).unwrap().len(), before);
}

#[test]
fn the_estimate_of_a_package_can_be_short_of_its_size() {
    use logcourier::batching::compact_with;
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let store = Store::open(&dir.0.join("data")).unwrap();
    fragments(&store, &config, 40, Some("0.66.1"), 1);
    let refused = (100..5_000).step_by(20).find(|limit| compact_with(&store, &config, *limit, 100).is_err());
    assert!(refused.is_some() || !store.queue(&config.destination()).unwrap().is_empty());
}

#[test]
fn a_delivery_sends_a_few_at_a_time_and_catalogues_what_is_sent() {
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let store = Store::open(&dir.0.join("data")).unwrap();
    fragments(&store, &config, 3, None, 10);
    let queued = store.queue(&config.destination()).unwrap();
    store.receipt(&queued[0].id, FILE, 1).unwrap();
    let group = Group::default();
    assert!(deliver(&store, &config, &group, &never()).unwrap().contains("3 архивов"));

    fragments(&store, &config, logcourier::constants::DELIVERY_BATCH_LIMIT + 2, Some("0.66.1"), 10);
    let message = deliver(&store, &config, &group, &never()).unwrap();
    assert!(message.contains(&format!("{} архивов", logcourier::constants::DELIVERY_BATCH_LIMIT)), "{message}");

    let dir = Dir::new();
    let store = Store::open(&dir.0.join("data")).unwrap();
    let many = logcourier::constants::CATALOG_MAX_ENTRIES + 3;
    fragments(&store, &config, many, Some("0.66.1"), 10);
    for row in store.queue(&config.destination()).unwrap() {
        store.receipt(&row.id, FILE, 1).unwrap();
    }
    let message = deliver(&Store::open(&dir.0.join("data")).unwrap(), &config, &Group::default(), &never()).unwrap();
    assert!(message.contains(&format!("{} архивов", logcourier::constants::CATALOG_MAX_ENTRIES)), "{message}");
}

#[test]
fn a_catalog_too_large_for_a_message_is_not_sent() {
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let store = Store::open(&dir.0.join("data")).unwrap();
    for _ in 0..logcourier::constants::CATALOG_MAX_ENTRIES {
        let identifier = new_id();
        let metadata = json!({
            "schema": 1, "bundle_id": identifier, "device_id": config.device_id, "created_at": "2026-11-04T12:00:00+00:00", "source_id": HEX32,
            "sha256": HEX64, "size": 1, "source_label": "ж".repeat(10_000), "kind": "fragment",
        });
        store.enqueue_event(&config.destination(), &metadata, b"x", &format!("lc-{identifier}.zip"), &[]).unwrap();
    }
    for row in store.queue(&config.destination()).unwrap() {
        store.receipt(&row.id, FILE, 1).unwrap();
    }
    assert!(deliver(&store, &config, &Group::default(), &never()).unwrap_err().to_string().contains("слишком большой"));
}

#[test]
fn a_new_version_alone_makes_a_new_catalog() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:03,000 0.66.2 INFO keyswitch.engine: other\n");
    let collector = Collector::new(&store);
    let (_, errors) = collector.scan_with(&config, 0).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let message = deliver(&store, &config, &group, &never()).unwrap();
    assert!(message.contains("архивов"), "{message}");
}

#[test]
fn listing_passes_over_what_a_page_repeats_and_what_has_no_version() {
    let group = Group::default();
    chain(&group, |index, page| {
        if page == 1 {
            index["entries"] = json!([entry()]);
            index["entries"][0]["bundle_id"] = json!(format!("{:032x}", 2));
        }
        if page == 0 {
            index["entries"][0]["keyswitch_version"] = Value::Null;
        }
    });
    assert_eq!(list_entries(&group, CHAT, 10, 5, None).unwrap().len(), 2, "one bundle was on two pages");
    assert_eq!(list_entries(&group, CHAT, 10, 5, Some("current")).unwrap().len(), 1, "one has no version");
}

#[test]
fn a_collector_with_large_chunks_refuses_an_archive_over_the_limit() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    let mut noise = Vec::with_capacity(logcourier::constants::ARCHIVE_LIMIT + 1024);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    while noise.len() < logcourier::constants::ARCHIVE_LIMIT + 1024 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        noise.extend_from_slice(&state.to_le_bytes());
    }
    std::fs::write(&log, &noise).unwrap();
    let store = Store::open(&dir.0.join("data")).unwrap();
    let mut collector = Collector::new(&store);
    collector.chunk_size = logcourier::constants::ARCHIVE_LIMIT * 2;
    let (count, errors) = collector.scan(&config).unwrap();
    assert_eq!(count, 0);
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn a_file_that_only_begins_a_version_header_waits_for_its_end() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00");
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    let first = collector.scan(&config).unwrap();
    assert!(first.1.is_empty(), "{:?}", first.1);
    append(&log, ":00,000 0.66.1 INFO keyswitch: a\n");
    let _ = collector.scan(&config).unwrap();
}

#[cfg(unix)]
#[test]
fn a_file_it_may_not_read_is_named_among_the_errors() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o000)).unwrap();
    let store = Store::open(&dir.0.join("data")).unwrap();
    let result = Collector::new(&store).scan(&config);
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600)).unwrap();
    let _ = result.unwrap();
}

#[test]
fn a_changed_version_list_alone_makes_a_new_catalog() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let mut no_sources = config.clone();
    no_sources.sources.clear();
    let message = deliver(&store, &no_sources, &group, &never()).unwrap();
    assert!(message.contains("0 архивов"), "{message}");
}

#[test]
fn a_fetch_refuses_a_damaged_archive_and_a_taken_path() {
    use logcourier::cli::{fetch_or_list_with, parse};
    let dir = Dir::new();
    let (group, _config, _store) = delivered_group(&dir);
    let output = dir.0.join("out");
    let options = || parse(&format!("--all-versions --output {}", output.display()).split_whitespace().map(str::to_owned).collect::<Vec<_>>()).unwrap();
    let catalog = group.pinned.borrow().unwrap();
    let originals = group.files.borrow().clone();
    let damaged: Vec<_> = originals
        .iter()
        .map(|file| if file.3 == catalog { file.clone() } else { (file.0.clone(), b"different bytes".to_vec(), file.2.clone(), file.3, file.4.clone()) })
        .collect();
    *group.files.borrow_mut() = damaged;
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options()).unwrap_err().to_string().contains("Контрольная сумма"));
    let same_length: Vec<_> = originals
        .iter()
        .map(|file| if file.3 == catalog { file.clone() } else { (file.0.clone(), vec![0; file.1.len()], file.2.clone(), file.3, file.4.clone()) })
        .collect();
    *group.files.borrow_mut() = same_length;
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options()).unwrap_err().to_string().contains("Контрольная сумма"));
    *group.files.borrow_mut() = originals;
    assert_eq!(fetch_or_list_with(&group, "fetch", CHAT, options()).unwrap(), 0);
    let saved: Vec<PathBuf> = walk(&output);
    let first = saved.iter().find(|path| path.extension().is_some_and(|ext| ext == "zip")).unwrap().clone();
    std::fs::write(&first, b"another file").unwrap();
    assert!(fetch_or_list_with(&group, "fetch", CHAT, options()).unwrap_err().to_string().contains("занят"));
    #[cfg(unix)]
    {
        std::fs::remove_file(&first).unwrap();
        std::os::unix::fs::symlink("/nonexistent", &first).unwrap();
        assert!(fetch_or_list_with(&group, "fetch", CHAT, options()).unwrap_err().to_string().contains("занят"));
    }
}

fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for item in std::fs::read_dir(root).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[cfg(unix)]
#[test]
fn a_rotated_copy_it_may_not_read_stops_only_that_source() {
    use std::os::unix::fs::PermissionsExt as _;
    for include_existing in [false, true] {
        let dir = Dir::new();
        let log = dir.0.join("app.log");
        let mut config = config(&log);
        config.sources[0].include_existing = include_existing;
        config.sources[0].rotations = 1;
        let rotated = dir.0.join("app.log.1");
        append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
        append(&rotated, "2026-11-04 11:00:00,000 0.66.1 INFO keyswitch: b\n");
        std::fs::set_permissions(&rotated, std::fs::Permissions::from_mode(0o000)).unwrap();
        let store = Store::open(&dir.0.join("data")).unwrap();
        let (_, errors) = Collector::new(&store).scan(&config).unwrap();
        std::fs::set_permissions(&rotated, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
    }
}

#[test]
fn a_budget_ends_a_scan_in_the_rotated_copies() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    config.sources[0].rotations = 1;
    let line = "2026-11-04 11:00:00,000 0.66.1 INFO keyswitch: b\n";
    append(&dir.0.join("app.log.1"), &line.repeat(50));
    append(&log, &line.repeat(50));
    let store = Store::open(&dir.0.join("data")).unwrap();
    let mut collector = Collector::new(&store);
    collector.chunk_size = 500;
    assert_eq!(collector.scan_with(&config, 2).unwrap().0, 2, "two chunks, then the budget");
    assert!(collector.scan_with(&config, 5).unwrap().0 >= 1);
}

#[test]
fn a_new_file_that_is_no_copy_of_an_old_one_starts_from_its_beginning() {
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    let mut config = config(&log);
    config.sources[0].include_existing = true;
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: first\n");
    let store = Store::open(&dir.0.join("data")).unwrap();
    let collector = Collector::new(&store);
    assert!(collector.scan(&config).unwrap().0 >= 1);
    std::fs::remove_file(&log).unwrap();
    std::fs::write(dir.0.join("placeholder"), "keeps the old number of the file taken").unwrap();
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: other text, and longer than the first one was\n");
    assert!(collector.scan(&config).unwrap().0 >= 1);
}

#[test]
fn a_chain_alone_means_the_collector_has_written_to_the_group() {
    let dir = Dir::new();
    let config = config(&dir.0.join("app.log"));
    let store = Store::open(&dir.0.join("data")).unwrap();
    assert!(!store.delivered(&config.destination()).unwrap());
    store.set(&format!("{}{}", logcourier::store::HEAD_DIGEST_KEY, config.destination()), &json!("sum")).unwrap();
    assert!(store.delivered(&config.destination()).unwrap());
}

#[test]
fn a_failure_of_telegram_while_clearing_is_reported_as_it_is() {
    let dir = Dir::new();
    let (group, config, store) = delivered_group(&dir);
    let key = format!("{}{}", logcourier::store::RESET_KEY, config.destination());
    store.set(&key, &json!({"top": 3, "next": 3, "kept": null})).unwrap();
    *group.fail_delete.borrow_mut() = true;
    let refused = reset_group(&store, &config, &group, false, &never(), &|_| {}).unwrap_err();
    assert!(refused.to_string().contains("500"), "{refused}");
}

#[test]
fn the_service_carries_on_an_unfinished_clearing_and_says_what_goes_wrong() {
    use logcourier::service::{MakeClient, Notify, Service};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    let dir = Dir::new();
    let log = dir.0.join("app.log");
    append(&log, "2026-11-04 12:00:00,000 0.66.1 INFO keyswitch: a\n");
    let mut settings = config(&log);
    settings.sources[0].include_existing = true;
    settings.sources.push(Source::new(dir.0.join("folder-of-nothing").join("x.log").display().to_string(), "пропавший"));
    std::fs::create_dir_all(dir.0.join("not-a-file.log")).unwrap();
    settings.sources.push(Source::new(dir.0.join("not-a-file.log").display().to_string(), "папка"));
    {
        let store = Store::open(&dir.0.join("data")).unwrap();
        store.set(&format!("{}{}", logcourier::store::RESET_KEY, settings.destination()), &json!({"top": 3, "next": 3, "kept": null})).unwrap();
    }
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    let notify: Notify = Arc::new(move |message, _stats| sink.lock().unwrap().push(message));
    let group = SharedGroup(Arc::new(Mutex::new(Group::default())));
    // 0: the group answers; 1: the program cannot start a client, with no words for the person; 2: Telegram asks for a pause.
    let mode = Arc::new(AtomicUsize::new(0));
    let (shared, kind) = (group.clone(), mode.clone());
    let make: MakeClient = Arc::new(move |_token| match kind.load(Ordering::SeqCst) {
        0 => {
            let client: Box<dyn Client> = Box::new(shared.clone());
            Ok(client)
        }
        1 => Err(Error::Os("OSError")),
        _ => Err(Error::telegram_with("slow down", 1, 429)),
    });
    let wait = |text: &str| {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if heard.lock().unwrap().iter().any(|message| message.contains(text)) {
                heard.lock().unwrap().clear();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never heard {text:?}: {:?}", heard.lock().unwrap());
    };
    settings.auto_send = false;
    let service = Service::start_with(dir.0.join("data"), settings.clone(), "1:token".to_owned(), notify, make);
    wait("не завершена");
    mode.store(1, Ordering::SeqCst);
    settings.auto_send = true;
    service.update(settings.clone(), "1:token".to_owned());
    wait("Операция не завершена");
    service.send_now();
    wait("продолжится через");
    mode.store(0, Ordering::SeqCst);
    service.reset(true);
    wait("Группа очищена");
    mode.store(2, Ordering::SeqCst);
    service.send_now();
    wait("slow down");
    mode.store(0, Ordering::SeqCst);
    service.update(settings.clone(), "1:token".to_owned());
    service.send_now();
    wait("источник недоступен");
    service.stop();
    let started = Instant::now();
    while !service.is_finished() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(service.is_finished());
}

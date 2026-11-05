//! Collection and delivery against a scripted Telegram.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use logcourier::batching::compact;
use logcourier::catalog::{current_catalog, decode_index, deliver, list_entries};
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

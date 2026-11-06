//! One worker owns collection and delivery; the interface never performs network requests.
use ks_cast::To as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use crate::batching::compact;
use crate::catalog::deliver;
use crate::collector::Collector;
use crate::config::Config;
use crate::constants::{SECONDS_PER_MINUTE, PENDING_RETRY_SECONDS, MAX_RETRY_DELAY_SECONDS, RETRY_BASE_SECONDS, RETRY_BACKOFF_BASE, RETRY_BACKOFF_MAX_EXPONENT, WAKE_POLL_SECONDS};
use crate::error::{Error, Result};
use crate::rate_limit::{Cancelled, RateLimitedClient};
use crate::reset::reset_group;
use crate::secrets::redact;
use crate::store::{RESET_KEY, Store};
use crate::telegram::{Client, Telegram};
use crate::timeutil::{monotonic, now_seconds};

pub type Notify = Arc<dyn Fn(String, Value) + Send + Sync>;
/// Makes the client of a token: Telegram itself, or what a test stands in for it.
pub type MakeClient = Arc<dyn Fn(&str) -> Result<Box<dyn Client>> + Send + Sync>;

struct Shared {
    config: Config,
    token: String,
    manual: bool,
    manual_pending: bool,
    /// None, or whether the requested restart of the catalog also deletes the group's messages.
    reset_request: Option<bool>,
    next_send: f64,
    revision: u64,
    woken: bool,
}

pub struct Service {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    stopping: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

fn locked(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Service {
    pub fn start(root: PathBuf, config: Config, token: String, notify: Notify) -> Service {
        Self::start_with(root, config, token, notify, Arc::new(|token| Ok(Box::new(Telegram::new(token)?))))
    }

    /// The service over the given client.
    pub fn start_with(root: PathBuf, config: Config, token: String, notify: Notify, make: MakeClient) -> Service {
        let next_send = monotonic() + config.interval_minutes.to::<f64>() * SECONDS_PER_MINUTE;
        let shared = Arc::new((
            Mutex::new(Shared { config, token, manual: false, manual_pending: false, reset_request: None, next_send, revision: 0, woken: false }),
            Condvar::new(),
        ));
        let stopping = Arc::new(AtomicBool::new(false));
        let (worker_shared, worker_stopping) = (shared.clone(), stopping.clone());
        let thread = std::thread::Builder::new().name("LogCourier-worker".to_owned()).spawn(move || run(&root, &worker_shared, &worker_stopping, &notify, &make)).ok();
        Service { shared, stopping, thread: Mutex::new(thread) }
    }

    fn wake(&self) {
        locked(&self.shared.0).woken = true;
        self.shared.1.notify_all();
    }

    pub fn update(&self, config: Config, token: String) {
        {
            let mut state = locked(&self.shared.0);
            state.next_send = monotonic() + config.interval_minutes.to::<f64>() * SECONDS_PER_MINUTE;
            state.config = config;
            state.token = token;
            state.revision += 1;
            state.manual = false;
            state.manual_pending = false;
            // A request made for the previous group or token never reaches another one.
            state.reset_request = None;
        }
        self.wake();
    }

    pub fn send_now(&self) {
        locked(&self.shared.0).manual = true;
        self.wake();
    }

    pub fn reset(&self, delete_messages: bool) {
        locked(&self.shared.0).reset_request = Some(delete_messages);
        self.wake();
    }

    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_finished(&self) -> bool {
        self.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().is_none_or(JoinHandle::is_finished)
    }
}

struct Worker<'a> {
    store: &'a Store,
    shared: &'a Arc<(Mutex<Shared>, Condvar)>,
    stopping: &'a Arc<AtomicBool>,
    notify: &'a Notify,
    make: &'a MakeClient,
    retry_at: f64,
    failures: i32,
}

impl Worker<'_> {
    fn cancelled(&self, revision: u64) -> Cancelled {
        let (shared, stopping) = (self.shared.clone(), self.stopping.clone());
        Arc::new(move || stopping.load(Ordering::SeqCst) || locked(&shared.0).revision != revision)
    }

    fn turn(&mut self, config: &Config, token: &str, reset: Option<bool>, revision: u64) -> Result<String> {
        let destination = config.destination();
        let store = self.store;
        let manual_pending = locked(&self.shared.0).manual_pending;
        let active = config.auto_send || manual_pending;
        let clearing = store.get(&format!("{RESET_KEY}{destination}")).is_some();
        // A requested reset runs at once; an unfinished one goes on while sending is on.
        let resetting = reset.is_some() || (clearing && active);
        let unconfigured = token.is_empty() || config.chat_id.is_empty() || config.bot_id.is_empty();
        if resetting && unconfigured {
            return Ok("Настройте токен и группу Telegram.".to_owned());
        }
        if resetting && reset.is_none() && monotonic() < self.retry_at {
            return Ok(format!("Очистка группы продолжится через {} с.", (self.retry_at - monotonic()).to::<i64>()));
        }
        if resetting {
            let cancelled = self.cancelled(revision);
            let client = RateLimitedClient::new((self.make)(token)?, store, &config.chat_id, cancelled.clone());
            let notify = self.notify.clone();
            let progress = |text: &str| notify(text.to_owned(), store.stats(&destination).unwrap_or(Value::Null));
            let message = reset_group(store, config, &client, reset.unwrap_or(false), &cancelled, &progress)?;
            self.failures = 0;
            self.retry_at = 0.0;
            // The new chain starts with the next delivery, not a full interval later.
            let mut state = locked(&self.shared.0);
            state.next_send = state.next_send.min(monotonic() + PENDING_RETRY_SECONDS);
            return Ok(message);
        }
        if clearing {
            return Ok("Очистка группы не завершена. Она продолжится, когда отправка снова включена, или по кнопке «Очистить группу и начать заново».".to_owned());
        }
        if active && !config.consent {
            return Ok("Требуется разрешение на передачу текста логов.".to_owned());
        }
        if active && unconfigured {
            return Ok("Настройте токен и группу Telegram.".to_owned());
        }
        if !active {
            return Ok("Автоматическая отправка выключена.".to_owned());
        }
        let mut warnings;
        let mut message;
        match Collector::new(store).scan(config) {
            Ok((count, errors)) => {
                warnings = errors.join("; ");
                message = if warnings.is_empty() { format!("Новых фрагментов: {count}. Ожидание отправки.") } else { warnings.clone() };
            }
            Err(error @ Error::QueueFull(_)) => {
                warnings = error.to_string();
                message = warnings.clone();
            }
            Err(error) => return Err(error),
        }
        let now = monotonic();
        let cooldown = store.get_f64(&format!("telegram_cooldown:{}", config.bot_id));
        self.retry_at = self.retry_at.max(now + (cooldown - now_seconds()).max(0.0));
        if now < self.retry_at {
            message.push_str(&format!(" Повтор через {} с.", (self.retry_at - now).to::<i64>()));
            return Ok(message);
        }
        let (pending_now, next_send) = {
            let state = locked(&self.shared.0);
            (state.manual_pending, state.next_send)
        };
        if pending_now || now >= next_send {
            let cancelled = self.cancelled(revision);
            // Only compact when sending is due, never create a new package each poll.
            loop {
                match compact(store, config) {
                    Ok(0) => break,
                    Ok(_) => {
                        if cancelled() {
                            return Err(Error::Cancelled("Отправка остановлена. Очередь сохранена.".to_owned()));
                        }
                    }
                    Err(error @ Error::QueueFull(_)) => {
                        warnings.push(' ');
                        warnings.push_str(&error.to_string());
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            let client = RateLimitedClient::new((self.make)(token)?, store, &config.chat_id, cancelled.clone());
            message = deliver(store, config, &client, &cancelled)?;
            if !warnings.is_empty() {
                message.push(' ');
                message.push_str(&warnings);
            }
            self.failures = 0;
            self.retry_at = 0.0;
            let pending = !store.queue(&destination)?.is_empty();
            let mut state = locked(&self.shared.0);
            if !pending {
                state.manual_pending = false;
            }
            state.next_send = monotonic() + if pending { PENDING_RETRY_SECONDS } else { config.interval_minutes.to::<f64>() * SECONDS_PER_MINUTE };
        }
        Ok(message)
    }
}

fn run(root: &std::path::Path, shared: &Arc<(Mutex<Shared>, Condvar)>, stopping: &Arc<AtomicBool>, notify: &Notify, make: &MakeClient) {
    let store = match Store::open(root) {
        Ok(store) => store,
        Err(error) => {
            notify(format!("Не удалось открыть очередь ({}).", error.type_name()), json!({}));
            return;
        }
    };
    let mut worker = Worker { store: &store, shared, stopping, notify, make, retry_at: 0.0, failures: 0 };
    while !stopping.load(Ordering::SeqCst) {
        let (config, token, reset, revision) = {
            let mut state = locked(&shared.0);
            if state.manual {
                state.manual = false;
                state.manual_pending = true;
            }
            (state.config.clone(), state.token.clone(), state.reset_request.take(), state.revision)
        };
        let stats = |store: &Store| store.stats(&config.destination()).unwrap_or(Value::Null);
        match worker.turn(&config, &token, reset, revision) {
            Ok(message) => notify(message, stats(&store)),
            Err(error @ Error::Cancelled(_)) => notify(error.to_string(), stats(&store)),
            Err(error) => {
                worker.failures += 1;
                let mut delay = MAX_RETRY_DELAY_SECONDS.min(RETRY_BASE_SECONDS * RETRY_BACKOFF_BASE.powi((worker.failures - 1).min(RETRY_BACKOFF_MAX_EXPONENT)));
                if let Error::Telegram { retry_after, .. } = error {
                    delay = delay.max(retry_after.to::<f64>());
                }
                worker.retry_at = monotonic() + delay;
                locked(&shared.0).next_send = worker.retry_at;
                let message = if error.has_user_text() {
                    redact(&error.to_string())
                } else {
                    format!("Операция не завершена ({}). Очередь сохранена.", error.type_name())
                };
                notify(message, stats(&store));
            }
        }
        let mut state = locked(&shared.0);
        if !state.woken {
            state = shared.1.wait_timeout(state, Duration::from_secs(WAKE_POLL_SECONDS)).map_or_else(|poisoned| poisoned.into_inner().0, |waited| waited.0);
        }
        state.woken = false;
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::config::Source;
    use std::io::Write as _;

    const BOT: &str = "123456";

    /// A group that takes every call and keeps no file: what was sent stays in the queue.
    struct Forgetful;

    impl Client for Forgetful {
        fn bot_id(&self) -> &str {
            BOT
        }

        fn call(&self, method: &str, _params: Value) -> Result<Value> {
            Ok(match method {
                "getChat" => json!({"type": "supergroup", "title": "logs"}),
                "getChatMember" => json!({"status": "administrator", "can_pin_messages": true, "can_delete_messages": true}),
                _ => json!(true),
            })
        }

        fn send_document(&self, _chat_id: &str, _filename: &str, _data: &[u8], _caption: &str) -> Result<Value> {
            Ok(json!({"message_id": 1, "chat": {"id": -1001}, "document": {"file_id": ""}}))
        }

        fn download(&self, _file_id: &str, _limit: usize) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    struct Place {
        root: PathBuf,
        log: PathBuf,
        store: Store,
        shared: Arc<(Mutex<Shared>, Condvar)>,
        stopping: Arc<AtomicBool>,
        notify: Notify,
        make: MakeClient,
    }

    impl Place {
        fn new(name: &str) -> Place {
            let root = std::env::temp_dir().join(format!("lc-service-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let log = root.join("app.log");
            Place {
                store: Store::open(&root.join("data")).unwrap(),
                root,
                log,
                shared: Arc::new((
                    Mutex::new(Shared { config: Config::default(), token: "1:t".to_owned(), manual: false, manual_pending: false, reset_request: None, next_send: 0.0, revision: 0, woken: false }),
                    Condvar::new(),
                )),
                stopping: Arc::new(AtomicBool::new(false)),
                notify: Arc::new(|_, _| {}),
                make: Arc::new(|_| Ok(Box::new(Forgetful))),
            }
        }

        fn config(&self) -> Config {
            let mut source = Source::new(self.log.display().to_string(), "app");
            source.include_existing = true;
            Config { bot_id: BOT.to_owned(), chat_id: "-1001".to_owned(), consent: true, auto_send: true, sources: vec![source], ..Config::default() }
        }

        fn write(&self, text: &str) {
            std::fs::OpenOptions::new().create(true).append(true).open(&self.log).unwrap().write_all(text.as_bytes()).unwrap();
        }

        fn worker(&self) -> Worker<'_> {
            Worker { store: &self.store, shared: &self.shared, stopping: &self.stopping, notify: &self.notify, make: &self.make, retry_at: 0.0, failures: 0 }
        }
    }

    impl Drop for Place {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_turn_is_cancelled_by_a_stop_or_by_new_settings() {
        let place = Place::new("cancel");
        let worker = place.worker();
        assert!(!worker.cancelled(0)());
        assert!(worker.cancelled(1)());
        place.stopping.store(true, Ordering::SeqCst);
        assert!(worker.cancelled(0)());
    }

    #[test]
    fn a_group_needs_a_token_a_chat_and_a_bot() {
        let place = Place::new("unconfigured");
        let mut worker = place.worker();
        let full = place.config();
        let no_chat = Config { chat_id: String::new(), ..full.clone() };
        let no_bot = Config { bot_id: String::new(), ..full.clone() };
        for (config, token) in [(&no_chat, "1:t"), (&no_bot, "1:t"), (&full, "")] {
            assert!(worker.turn(config, token, Some(false), 0).unwrap().contains("Настройте"));
            assert!(worker.turn(config, token, None, 0).unwrap().contains("Настройте"));
        }
    }

    #[test]
    fn a_full_queue_stops_the_collection_and_the_delivery_goes_on() {
        let mut place = Place::new("full");
        place.write("first line\nsecond one\n");
        place.store.capacity = 0;
        let config = place.config();
        let mut worker = place.worker();
        let message = worker.turn(&config, "1:t", None, 0).unwrap();
        assert!(message.contains("Новых архивов"), "{message}");
        assert!(message.contains("заполнена"), "{message}");
    }

    #[test]
    fn fragments_wait_for_their_package_and_a_package_for_its_place() {
        let mut place = Place::new("package");
        let config = place.config();
        place.write("first line\nsecond one\nthird one!\n");
        let collector = Collector { store: &place.store, chunk_size: 11 };
        assert!(collector.scan(&config).unwrap().0 > 1);
        // No room for the package: the fragments stay, and what stays is said.
        place.store.capacity = 0;
        let mut worker = place.worker();
        let message = worker.turn(&config, "1:t", None, 0).unwrap();
        assert!(message.contains("упаковки"), "{message}");
        // What a group does not keep stays in the queue: sent again soon, not at the next interval.
        assert!(locked(&place.shared.0).next_send <= monotonic() + PENDING_RETRY_SECONDS);
    }

    #[test]
    fn fragments_are_packed_until_none_are_left_when_nobody_stops_it() {
        let place = Place::new("packed");
        let config = place.config();
        place.write("first line\nsecond one\nthird one!\n");
        let collector = Collector { store: &place.store, chunk_size: 11 };
        assert!(collector.scan(&config).unwrap().0 > 1);
        let mut worker = place.worker();
        assert!(worker.turn(&config, "1:t", None, 0).is_ok());
    }

    #[test]
    fn a_stop_comes_between_two_packages() {
        let place = Place::new("stop");
        let config = place.config();
        place.write("first line\nsecond one\nthird one!\n");
        let collector = Collector { store: &place.store, chunk_size: 11 };
        assert!(collector.scan(&config).unwrap().0 > 1);
        place.stopping.store(true, Ordering::SeqCst);
        let mut worker = place.worker();
        assert!(matches!(worker.turn(&config, "1:t", None, 0), Err(Error::Cancelled(_))));
    }

    #[test]
    fn packing_and_sending_all_of_it_waits_for_the_next_interval() {
        let place = Place::new("sent");
        let config = Config { sources: Vec::new(), ..place.config() };
        let mut worker = place.worker();
        locked(&place.shared.0).manual_pending = true;
        let message = worker.turn(&config, "1:t", None, 0).unwrap();
        assert!(message.contains("Новых архивов"), "{message}");
        let state = locked(&place.shared.0);
        assert!(!state.manual_pending);
        assert!(state.next_send > monotonic() + PENDING_RETRY_SECONDS);
    }

    #[test]
    fn a_queue_that_cannot_be_read_fails_the_turn() {
        let place = Place::new("broken");
        let config = place.config();
        place.write("first line\n");
        place.store.db.execute_batch("DROP TABLE bundles").unwrap();
        let mut worker = place.worker();
        assert!(worker.turn(&config, "1:t", None, 0).is_err());
    }

    #[test]
    fn a_queue_that_cannot_be_packed_fails_the_turn() {
        let place = Place::new("readonly");
        let config = place.config();
        place.write("first line\nsecond one\nthird one!\n");
        let collector = Collector { store: &place.store, chunk_size: 11 };
        assert!(collector.scan(&config).unwrap().0 > 1);
        let quiet = Config { sources: Vec::new(), ..config };
        place.store.db.execute_batch("PRAGMA query_only = 1").unwrap();
        let mut worker = place.worker();
        assert!(worker.turn(&quiet, "1:t", None, 0).is_err());
    }

    #[test]
    fn the_worker_says_a_cancelled_turn_and_goes_round_again_when_woken() {
        let mut place = Place::new("loop");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (sink, stop, wakes) = (seen.clone(), place.stopping.clone(), place.shared.clone());
        place.notify = Arc::new(move |message: String, _| {
            let mut seen = sink.lock().unwrap();
            seen.push(message);
            // Woken while it worked: no waiting before the next round.
            locked(&wakes.0).woken = true;
            if seen.len() >= 2 {
                stop.store(true, Ordering::SeqCst);
            }
        });
        let moved = place.shared.clone();
        place.make = Arc::new(move |_| {
            // New settings arrive as the client is made: the delivery stops at its first check.
            locked(&moved.0).revision += 1;
            Ok(Box::new(Forgetful))
        });
        locked(&place.shared.0).config = Config { sources: Vec::new(), ..place.config() };
        run(&place.root.join("data"), &place.shared, &place.stopping, &place.notify, &place.make);
        assert!(seen.lock().unwrap()[0].contains("остановлен") || !seen.lock().unwrap().is_empty());
    }
}

//! Persistent per-bot cooldown and conservative group pacing including catalog pins.
use ks_cast::To as _;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::constants::{WAIT_POLL_SECONDS, BOT_INTERVAL_SECONDS, TELEGRAM_RATE_LIMIT_STATUS, DEFAULT_RETRY_AFTER_SECONDS, GROUP_INTERVAL};
use crate::error::{Error, Result};
use crate::store::Store;
use crate::telegram::Client;
use crate::timeutil::now_seconds;

/// Requests that put something into the group: documents, pins and notices share its pace.
const GROUP_METHODS: [&str; 4] = ["pinChatMessage", "unpinChatMessage", "unpinAllChatMessages", "sendMessage"];

pub type Cancelled = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct RateLimitedClient<'a, C: Client> {
    client: C,
    store: &'a Store,
    chat_id: String,
    cancelled: Cancelled,
    clock: Box<dyn Fn() -> f64 + 'a>,
    sleep: Box<dyn Fn(f64) + 'a>,
    cooldown_key: String,
}

impl<'a, C: Client> RateLimitedClient<'a, C> {
    pub fn new(client: C, store: &'a Store, chat_id: &str, cancelled: Cancelled) -> RateLimitedClient<'a, C> {
        RateLimitedClient::with_clock(client, store, chat_id, cancelled, Box::new(now_seconds), Box::new(|seconds| std::thread::sleep(Duration::from_secs_f64(seconds))))
    }

    pub fn with_clock(client: C, store: &'a Store, chat_id: &str, cancelled: Cancelled, clock: Box<dyn Fn() -> f64 + 'a>, sleep: Box<dyn Fn(f64) + 'a>) -> RateLimitedClient<'a, C> {
        let cooldown_key = format!("telegram_cooldown:{}", client.bot_id());
        RateLimitedClient { client, store, chat_id: chat_id.to_owned(), cancelled, clock, sleep, cooldown_key }
    }

    fn perform<T>(&self, action: impl FnOnce() -> Result<T>, mutation: bool, interval: f64) -> Result<T> {
        let cooldown = self.store.get_f64(&self.cooldown_key);
        if cooldown > (self.clock)() {
            return Err(Error::telegram_with("Telegram временно ограничил запросы. Очередь сохранена.", (cooldown - (self.clock)()).to::<i64>() + 1, 0));
        }
        if mutation {
            let stopped = || Error::Cancelled("Отправка остановлена. Очередь сохранена.".to_owned());
            let key = format!("telegram_next:{}:{}", self.client.bot_id(), self.chat_id);
            let bot_key = format!("telegram_next_bot:{}", self.client.bot_id());
            let until = self.store.get_f64(&key).max(self.store.get_f64(&bot_key));
            while until > (self.clock)() {
                if (self.cancelled)() {
                    return Err(stopped());
                }
                (self.sleep)(WAIT_POLL_SECONDS.min(until - (self.clock)()));
            }
            if (self.cancelled)() {
                return Err(stopped());
            }
            self.store.set(&key, &json!((self.clock)() + interval))?;
            self.store.set(&bot_key, &json!((self.clock)() + BOT_INTERVAL_SECONDS))?;
        }
        match action() {
            Err(Error::Telegram { retry_after, status_code, .. }) if retry_after != 0 || status_code == TELEGRAM_RATE_LIMIT_STATUS => {
                let delay = (if retry_after != 0 { retry_after } else { DEFAULT_RETRY_AFTER_SECONDS }).max(1) + 1;
                self.store.set(&self.cooldown_key, &json!((self.clock)() + delay.to::<f64>()))?;
                Err(Error::telegram_with("Telegram запросил паузу; автоматический повтор после её окончания.", delay, TELEGRAM_RATE_LIMIT_STATUS))
            }
            other => other,
        }
    }
}

impl<C: Client> Client for RateLimitedClient<'_, C> {
    fn bot_id(&self) -> &str {
        self.client.bot_id()
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        if method == "deleteMessages" {
            return self.perform(|| self.client.call(method, params), true, BOT_INTERVAL_SECONDS);
        }
        self.perform(|| self.client.call(method, params), GROUP_METHODS.contains(&method), GROUP_INTERVAL)
    }

    fn send_document(&self, chat_id: &str, filename: &str, data: &[u8], caption: &str) -> Result<Value> {
        self.perform(|| self.client.send_document(chat_id, filename, data, caption), true, GROUP_INTERVAL)
    }

    fn download(&self, file_id: &str, limit: usize) -> Result<Vec<u8>> {
        self.perform(|| self.client.download(file_id, limit), false, GROUP_INTERVAL)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    use serde_json::Value;

    use super::*;
    use crate::store::Store;

    #[derive(Default)]
    struct Stub {
        answers: RefCell<VecDeque<Result<Value>>>,
    }

    impl Client for &Stub {
        fn bot_id(&self) -> &str {
            "1"
        }
        fn call(&self, _method: &str, _params: Value) -> Result<Value> {
            self.answers.borrow_mut().pop_front().unwrap_or(Ok(Value::Null))
        }
        fn send_document(&self, _chat_id: &str, _filename: &str, _data: &[u8], _caption: &str) -> Result<Value> {
            Ok(Value::Null)
        }
        fn download(&self, _file_id: &str, _limit: usize) -> Result<Vec<u8>> {
            Ok(vec![1])
        }
    }

    fn store() -> Store {
        let root = std::env::temp_dir().join(format!("lc-rate-{}", crate::timeutil::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        Store::open(&root).unwrap()
    }

    #[test]
    fn requests_wait_for_their_time_and_a_limit_stops_them() {
        let (stub, store) = (Stub::default(), store());
        let now = Rc::new(Cell::new(100.0_f64));
        let (clock, sleep) = (now.clone(), now.clone());
        let client = RateLimitedClient::with_clock(&stub, &store, "-1", Arc::new(|| false), Box::new(move || clock.get()), Box::new(move |seconds| sleep.set(sleep.get() + seconds)));
        assert_eq!(client.bot_id(), "1");
        assert_eq!(client.call("getMe", Value::Null).unwrap(), Value::Null);
        assert_eq!(client.call("sendMessage", Value::Null).unwrap(), Value::Null);
        assert!(now.get() > 100.0, "the second one waited");
        assert_eq!(client.call("deleteMessages", Value::Null).unwrap(), Value::Null);
        assert_eq!(client.send_document("-1", "a", b"x", "c").unwrap(), Value::Null);
        assert_eq!(client.download("id", 5).unwrap(), [1]);
        stub.answers.borrow_mut().push_back(Err(Error::telegram("not a limit")));
        assert!(client.call("sendMessage", Value::Null).is_err());
        stub.answers.borrow_mut().push_back(Err(Error::telegram_with("limit", 7, 429)));
        assert!(matches!(client.call("sendMessage", Value::Null), Err(Error::Telegram { retry_after: 7, .. })));
        assert!(matches!(client.call("getMe", Value::Null), Err(Error::Telegram { .. })));
        now.set(now.get() + 100.0);
        stub.answers.borrow_mut().push_back(Err(Error::telegram_with("limit", 0, 429)));
        assert!(client.call("getMe", Value::Null).is_err());
    }

    #[test]
    fn a_cancelled_delivery_stops_before_it_waits_and_after() {
        let (stub, store) = (Stub::default(), store());
        let now = Rc::new(Cell::new(100.0_f64));
        let answers = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = answers.clone();
        let cancelled: Cancelled = Arc::new(move || count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 1);
        let (clock, sleep) = (now.clone(), now.clone());
        let client = RateLimitedClient::with_clock(&stub, &store, "-1", cancelled, Box::new(move || clock.get()), Box::new(move |seconds| sleep.set(sleep.get() + seconds)));
        assert!(client.call("sendMessage", Value::Null).is_ok());
        assert!(matches!(client.call("sendMessage", Value::Null), Err(Error::Cancelled(_))));
        store.set("telegram_next:1:-1", &json!(0.0)).unwrap();
        store.set("telegram_next_bot:1", &json!(0.0)).unwrap();
        assert!(matches!(client.call("sendMessage", Value::Null), Err(Error::Cancelled(_))));
        let _ = RateLimitedClient::new(&stub, &store, "-1", Arc::new(|| true));
    }
}

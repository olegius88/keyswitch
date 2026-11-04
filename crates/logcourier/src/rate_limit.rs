//! Persistent per-bot cooldown and conservative group pacing including catalog pins.
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::constants::*;
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
            return Err(Error::telegram_with("Telegram временно ограничил запросы. Очередь сохранена.", (cooldown - (self.clock)()) as i64 + 1, 0));
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
                self.store.set(&self.cooldown_key, &json!((self.clock)() + delay as f64))?;
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

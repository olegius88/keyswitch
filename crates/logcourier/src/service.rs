//! One worker owns collection and delivery; the interface never performs network requests.
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
use crate::constants::*;
use crate::error::{Error, Result};
use crate::rate_limit::{Cancelled, RateLimitedClient};
use crate::reset::reset_group;
use crate::secrets::redact;
use crate::store::{RESET_KEY, Store};
use crate::telegram::Telegram;
use crate::timeutil::{monotonic, now_seconds};

pub type Notify = Arc<dyn Fn(String, Value) + Send + Sync>;

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
        let next_send = monotonic() + config.interval_minutes as f64 * SECONDS_PER_MINUTE;
        let shared = Arc::new((
            Mutex::new(Shared { config, token, manual: false, manual_pending: false, reset_request: None, next_send, revision: 0, woken: false }),
            Condvar::new(),
        ));
        let stopping = Arc::new(AtomicBool::new(false));
        let (worker_shared, worker_stopping) = (shared.clone(), stopping.clone());
        let thread = std::thread::Builder::new().name("LogCourier-worker".to_owned()).spawn(move || run(&root, &worker_shared, &worker_stopping, &notify)).ok();
        Service { shared, stopping, thread: Mutex::new(thread) }
    }

    fn wake(&self) {
        locked(&self.shared.0).woken = true;
        self.shared.1.notify_all();
    }

    pub fn update(&self, config: Config, token: String) {
        {
            let mut state = locked(&self.shared.0);
            state.next_send = monotonic() + config.interval_minutes as f64 * SECONDS_PER_MINUTE;
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
            return Ok(format!("Очистка группы продолжится через {} с.", (self.retry_at - monotonic()) as i64));
        }
        if resetting {
            let cancelled = self.cancelled(revision);
            let client = RateLimitedClient::new(Telegram::new(token)?, store, &config.chat_id, cancelled.clone());
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
            message.push_str(&format!(" Повтор через {} с.", (self.retry_at - now) as i64));
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
            let client = RateLimitedClient::new(Telegram::new(token)?, store, &config.chat_id, cancelled.clone());
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
            state.next_send = monotonic() + if pending { PENDING_RETRY_SECONDS } else { config.interval_minutes as f64 * SECONDS_PER_MINUTE };
        }
        Ok(message)
    }
}

fn run(root: &std::path::Path, shared: &Arc<(Mutex<Shared>, Condvar)>, stopping: &Arc<AtomicBool>, notify: &Notify) {
    let store = match Store::open(root) {
        Ok(store) => store,
        Err(error) => {
            notify(format!("Не удалось открыть очередь ({}).", error.type_name()), json!({}));
            return;
        }
    };
    let mut worker = Worker { store: &store, shared, stopping, notify, retry_at: 0.0, failures: 0 };
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
                    delay = delay.max(retry_after as f64);
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

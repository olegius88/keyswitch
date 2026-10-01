from __future__ import annotations

import copy
import threading
import time

from .batching import compact
from .catalog import DeliveryCancelled, deliver
from .collector import Collector
from .config import Config
from .constants.timing import (
    MAX_RETRY_DELAY_SECONDS,
    PENDING_RETRY_SECONDS,
    RETRY_BACKOFF_BASE,
    RETRY_BACKOFF_MAX_EXPONENT,
    RETRY_BASE_SECONDS,
    SECONDS_PER_MINUTE,
    WAKE_POLL_SECONDS,
)
from .rate_limit import RateLimitedClient
from .reset import reset_group
from .secrets import redact
from .store import RESET_KEY, QueueFull, Store
from .telegram import Telegram, TelegramError


class Service:
    """One worker owns collection and delivery; UI never performs network requests."""

    def __init__(self, root, config: Config, token: str, notify):
        self.root, self.notify = root, notify
        self._config, self._token = copy.deepcopy(config), token
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.wake = threading.Event()
        self.manual = False
        self.manual_pending = False
        # None, or whether the requested restart of the catalog also deletes the group's messages
        self.reset_request: bool | None = None
        self.next_send = time.monotonic() + config.interval_minutes * SECONDS_PER_MINUTE
        self.retry_at = 0.0
        self.failures = 0
        self.revision = 0
        self.thread = threading.Thread(target=self.run, name="LogCourier-worker", daemon=True)

    def start(self):
        self.thread.start()

    def update(self, config: Config, token: str):
        with self.lock:
            self._config, self._token = copy.deepcopy(config), token
            self.next_send = time.monotonic() + config.interval_minutes * SECONDS_PER_MINUTE
            self.revision += 1
            self.manual = False
            self.manual_pending = False
            # A request made for the previous group or token never reaches another one.
            self.reset_request = None
        self.wake.set()

    def send_now(self):
        with self.lock:
            self.manual = True
        self.wake.set()

    def reset(self, delete_messages: bool):
        with self.lock:
            self.reset_request = delete_messages
        self.wake.set()

    def stop(self):
        self.stop_event.set()
        self.wake.set()

    def run(self):
        try:
            store = Store(self.root)
        except Exception as error:
            self.notify(f"Не удалось открыть очередь ({type(error).__name__}).", {})
            return
        collector = Collector(store)
        try:
            while not self.stop_event.is_set():
                with self.lock:
                    config, token = copy.deepcopy(self._config), self._token
                    manual, self.manual = self.manual, False
                    if manual:
                        self.manual_pending = True
                    reset, self.reset_request = self.reset_request, None
                    revision = self.revision
                message = "Автоматическая отправка выключена."
                try:
                    active = config.auto_send or self.manual_pending
                    clearing = store.get(RESET_KEY + config.destination) is not None
                    # A requested reset runs at once; an unfinished one goes on while sending is on.
                    resetting = reset is not None or (clearing and active)
                    if resetting and (not token or not config.chat_id or not config.bot_id):
                        message = "Настройте токен и группу Telegram."
                    elif resetting and reset is None and time.monotonic() < self.retry_at:
                        message = (
                            "Очистка группы продолжится через "
                            f"{int(self.retry_at - time.monotonic())} с."
                        )
                    elif resetting:
                        client = RateLimitedClient(
                            Telegram(token),
                            store,
                            config.chat_id,
                            lambda: self.stop_event.is_set() or self.revision != revision,
                        )
                        message = reset_group(
                            store,
                            config,
                            client,
                            bool(reset),
                            lambda: self.stop_event.is_set() or self.revision != revision,
                            lambda text: self.notify(text, store.stats(config.destination)),
                        )
                        self.failures = 0
                        self.retry_at = 0
                        # The new chain starts with the next delivery, not a full interval later.
                        self.next_send = min(
                            self.next_send, time.monotonic() + PENDING_RETRY_SECONDS
                        )
                    elif clearing:
                        message = (
                            "Очистка группы не завершена. Она продолжится, когда отправка "
                            "снова включена, или по кнопке «Очистить группу и начать заново»."
                        )
                    elif active and not config.consent:
                        message = "Требуется разрешение на передачу текста логов."
                    elif active and (not token or not config.chat_id or not config.bot_id):
                        message = "Настройте токен и группу Telegram."
                    elif active:
                        warnings = ""
                        try:
                            count, errors = collector.scan(config)
                            warnings = "; ".join(errors)
                            message = warnings or f"Новых фрагментов: {count}. Ожидание отправки."
                        except QueueFull as error:
                            message = warnings = str(error)
                        now = time.monotonic()
                        cooldown = store.get("telegram_cooldown:" + config.bot_id, 0)
                        self.retry_at = max(self.retry_at, now + max(0, cooldown - time.time()))
                        if now < self.retry_at:
                            message += f" Повтор через {int(self.retry_at - now)} с."
                        elif self.manual_pending or now >= self.next_send:
                            # Only compact when sending is due, never create a new package each poll.
                            try:
                                while compact(store, config):
                                    if self.stop_event.is_set() or self.revision != revision:
                                        raise DeliveryCancelled(
                                            "Отправка остановлена. Очередь сохранена."
                                        )
                            except QueueFull as error:
                                warnings += " " + str(error)
                            client = RateLimitedClient(
                                Telegram(token),
                                store,
                                config.chat_id,
                                lambda: self.stop_event.is_set() or self.revision != revision,
                            )
                            message = deliver(
                                store,
                                config,
                                client,
                                lambda: self.stop_event.is_set() or self.revision != revision,
                            )
                            if warnings:
                                message += " " + warnings
                            self.failures = 0
                            self.retry_at = 0
                            pending = bool(store.queue(config.destination))
                            if not pending:
                                self.manual_pending = False
                            self.next_send = time.monotonic() + (
                                PENDING_RETRY_SECONDS
                                if pending
                                else config.interval_minutes * SECONDS_PER_MINUTE
                            )
                    self.notify(message, store.stats(config.destination))
                except DeliveryCancelled as error:
                    self.notify(str(error), store.stats(config.destination))
                except Exception as error:
                    self.failures += 1
                    delay = min(
                        MAX_RETRY_DELAY_SECONDS,
                        RETRY_BASE_SECONDS
                        * RETRY_BACKOFF_BASE ** min(self.failures - 1, RETRY_BACKOFF_MAX_EXPONENT),
                    )
                    if isinstance(error, TelegramError):
                        delay = max(delay, error.retry_after)
                    self.retry_at = time.monotonic() + delay
                    self.next_send = self.retry_at
                    if isinstance(error, (ValueError, RuntimeError)):
                        message = redact(str(error))
                    else:
                        message = (
                            f"Операция не завершена ({type(error).__name__}). Очередь сохранена."
                        )
                    self.notify(message, store.stats(config.destination))
                self.wake.wait(WAKE_POLL_SECONDS)
                self.wake.clear()
        finally:
            store.close()

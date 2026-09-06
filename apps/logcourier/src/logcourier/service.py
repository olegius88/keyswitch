from __future__ import annotations

import copy
import threading
import time

from .batching import compact
from .catalog import DeliveryCancelled, deliver
from .collector import Collector
from .config import Config
from .rate_limit import RateLimitedClient
from .secrets import redact
from .store import QueueFull, Store
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
        self.next_send = time.monotonic() + config.interval_minutes * 60
        self.retry_at = 0.0
        self.failures = 0
        self.revision = 0
        self.thread = threading.Thread(target=self.run, name="LogCourier-worker", daemon=True)

    def start(self):
        self.thread.start()

    def update(self, config: Config, token: str):
        with self.lock:
            self._config, self._token = copy.deepcopy(config), token
            self.next_send = time.monotonic() + config.interval_minutes * 60
            self.revision += 1
            self.manual = False
            self.manual_pending = False
        self.wake.set()

    def send_now(self):
        with self.lock:
            self.manual = True
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
                    revision = self.revision
                message = "Автоматическая отправка выключена."
                try:
                    active = config.auto_send or self.manual_pending
                    if active and not config.consent:
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
                                5 if pending else config.interval_minutes * 60
                            )
                    self.notify(message, store.stats(config.destination))
                except DeliveryCancelled as error:
                    self.notify(str(error), store.stats(config.destination))
                except Exception as error:
                    self.failures += 1
                    delay = min(900, 15 * 2 ** min(self.failures - 1, 6))
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
                self.wake.wait(5)
                self.wake.clear()
        finally:
            store.close()

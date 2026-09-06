"""Persistent per-bot cooldown and conservative group pacing including catalog pins."""

import time

from .catalog import DeliveryCancelled
from .telegram import TelegramError

GROUP_INTERVAL = 4.0  # at most 15 operations/minute, below Telegram's 20 messages/minute


class RateLimitedClient:
    def __init__(self, client, store, chat_id, cancelled, clock=time.time, sleep=time.sleep):
        self.client, self.store, self.chat_id = client, store, chat_id
        self.bot_id = client.bot_id
        self.cancelled, self.clock, self.sleep = cancelled, clock, sleep
        self.cooldown_key = "telegram_cooldown:" + self.bot_id

    def _perform(self, action, mutation=False):
        cooldown = self.store.get(self.cooldown_key, 0)
        if cooldown > self.clock():
            raise TelegramError(
                "Telegram временно ограничил запросы. Очередь сохранена.",
                retry_after=int(cooldown - self.clock()) + 1,
            )
        if mutation:
            key = f"telegram_next:{self.bot_id}:{self.chat_id}"
            bot_key = "telegram_next_bot:" + self.bot_id
            until = max(self.store.get(key, 0), self.store.get(bot_key, 0))
            while until > self.clock():
                if self.cancelled():
                    raise DeliveryCancelled("Отправка остановлена. Очередь сохранена.")
                self.sleep(min(0.2, until - self.clock()))
            if self.cancelled():
                raise DeliveryCancelled("Отправка остановлена. Очередь сохранена.")
            self.store.set(key, self.clock() + GROUP_INTERVAL)
            self.store.set(bot_key, self.clock() + 1.0)
        try:
            return action()
        except TelegramError as error:
            if error.retry_after or error.status_code == 429:
                delay = max(1, error.retry_after or 60) + 1
                self.store.set(self.cooldown_key, self.clock() + delay)
                raise TelegramError(
                    "Telegram запросил паузу; автоматический повтор после её окончания.",
                    retry_after=delay,
                    status_code=429,
                ) from None
            raise

    def call(self, method, params=None):
        return self._perform(
            lambda: self.client.call(method, params),
            method in ("pinChatMessage", "unpinChatMessage"),
        )

    def send_document(self, *args, **kwargs):
        return self._perform(lambda: self.client.send_document(*args, **kwargs), mutation=True)

    def download(self, *args, **kwargs):
        return self._perform(lambda: self.client.download(*args, **kwargs))

import copy

import pytest

from logcourier.config import Config, Source
from logcourier.store import Store
from logcourier.telegram import TelegramError


@pytest.fixture
def configured(tmp_path):
    path = tmp_path / "input.log"
    path.write_bytes(b"old\n")
    config = Config(
        bot_id="123456",
        chat_id="-100123",
        consent=True,
        sources=[Source(str(path), include_existing=True)],
    )
    return config, path


@pytest.fixture
def store(tmp_path):
    instance = Store(tmp_path / "state")
    yield instance
    instance.close()


class FakeTelegram:
    bot_id = "123456"

    def __init__(self):
        self.files = {}
        self.messages = {}
        self.pinned = None
        self.fail_pin = False
        self.uploads = 0
        self.after_upload = lambda: None

    def call(self, method, params=None):
        if method == "getChat":
            chat = {"id": -100123, "type": "supergroup", "title": "Private logs"}
            if self.pinned:
                chat["pinned_message"] = copy.deepcopy(self.messages[self.pinned])
            return chat
        if method == "getMe":
            return {"id": int(self.bot_id), "username": "log_bot"}
        if method == "getChatMember":
            return {"status": "administrator", "can_pin_messages": True}
        if method == "pinChatMessage":
            if self.fail_pin:
                raise TelegramError("No pin permission")
            self.pinned = params["message_id"]
            return True
        raise AssertionError(method)

    def send_document(self, chat_id, filename, data, caption=""):
        self.uploads += 1
        file_id = f"file_{self.uploads}"
        self.files[file_id] = data
        message = {
            "message_id": self.uploads,
            "chat": {"id": int(chat_id)},
            "from": {"id": int(self.bot_id)},
            "caption": caption,
            "document": {"file_id": file_id, "file_name": filename},
        }
        self.messages[self.uploads] = message
        self.after_upload()
        return message

    def download(self, file_id, limit=10 * 1024 * 1024):
        result = self.files[file_id]
        assert len(result) <= limit
        return result


@pytest.fixture
def telegram():
    return FakeTelegram()

import copy

import pytest
from fixture_values.counts import FAKE_DOWNLOAD_LIMIT_BYTES
from fixture_values.keys import PRIVATE_LOGS_CHAT_ID

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
        self.aliases = {}
        self.messages = {}
        self.pinned = None
        self.fail_pin = False
        self.uploads = 0
        self.reissues = 0
        self.after_upload = lambda: None

    def call(self, method, params=None):
        if method == "getChat":
            chat = {"id": PRIVATE_LOGS_CHAT_ID, "type": "supergroup", "title": "Private logs"}
            if self.pinned:
                message = copy.deepcopy(self.messages[self.pinned])
                document = message.get("document")
                if document:
                    document["file_id"] = self.reissue(document["file_id"])
                chat["pinned_message"] = message
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

    def reissue(self, file_id):
        # Telegram names the same document with a new file_id once its reference ages.
        self.reissues += 1
        stored = self.aliases.get(file_id, file_id)
        self.aliases[f"{stored}_r{self.reissues}"] = stored
        return f"{stored}_r{self.reissues}"

    def content(self, file_id):
        return self.files[self.aliases.get(file_id, file_id)]

    def replace(self, file_id, data):
        self.files[self.aliases.get(file_id, file_id)] = data

    def download(self, file_id, limit=FAKE_DOWNLOAD_LIMIT_BYTES):
        result = self.content(file_id)
        assert len(result) <= limit
        return result


@pytest.fixture
def telegram():
    return FakeTelegram()

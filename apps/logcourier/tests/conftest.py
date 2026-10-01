import copy

import pytest
from fixture_values.counts import FAKE_DOWNLOAD_LIMIT_BYTES
from fixture_values.keys import PRIVATE_LOGS_CHAT_ID, SUPERGROUP_CREATION_MESSAGE_ID

from logcourier.config import Config, Source
from logcourier.constants.limits import DELETE_BATCH_MESSAGES
from logcourier.constants.telegram import TELEGRAM_BAD_REQUEST_STATUS
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
        self.messages = {
            SUPERGROUP_CREATION_MESSAGE_ID: {"message_id": SUPERGROUP_CREATION_MESSAGE_ID}
        }
        self.last_message_id = SUPERGROUP_CREATION_MESSAGE_ID
        # Messages Telegram refuses to delete; one of them makes it refuse the whole batch.
        self.kept = {SUPERGROUP_CREATION_MESSAGE_ID}
        self.pinned = None
        self.fail_pin = False
        self.uploads = 0
        self.reissues = 0
        self.after_upload = lambda: None
        self.chat_type = "supergroup"
        self.member = {"status": "administrator", "can_pin_messages": True}
        self.member["can_delete_messages"] = True
        self.notice_chat = PRIVATE_LOGS_CHAT_ID
        self.unpin_all_result = True
        self.unpin_all_calls = 0
        self.delete_result = True
        self.delete_failure = None
        self.delete_requests = []
        self.deleted = set()

    def call(self, method, params=None):
        if method == "getChat":
            chat = {"id": PRIVATE_LOGS_CHAT_ID, "type": self.chat_type, "title": "Private logs"}
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
            return dict(self.member)
        if method == "sendMessage":
            identifier = self.next_message_id()
            self.messages[identifier] = {"message_id": identifier, "text": params["text"]}
            return {"message_id": identifier, "chat": {"id": self.notice_chat}}
        if method == "unpinAllChatMessages":
            self.unpin_all_calls += 1
            self.pinned = None
            return self.unpin_all_result
        if method == "deleteMessages":
            return self.delete(params["message_ids"])
        if method == "pinChatMessage":
            if self.fail_pin:
                raise TelegramError("No pin permission")
            self.pinned = params["message_id"]
            return True
        raise AssertionError(method)

    def next_message_id(self):
        self.last_message_id += 1
        return self.last_message_id

    def delete(self, identifiers):
        assert 1 <= len(identifiers) <= DELETE_BATCH_MESSAGES
        self.delete_requests.append(list(identifiers))
        if self.delete_failure:
            failure, self.delete_failure = self.delete_failure, None
            raise failure
        if self.kept & set(identifiers):
            # As the live Bot API answers: nothing in the batch is deleted.
            raise TelegramError(
                "Telegram HTTP 400: Bad Request: message can't be deleted",
                status_code=TELEGRAM_BAD_REQUEST_STATUS,
            )
        self.deleted.update(identifiers)
        for identifier in identifiers:
            self.messages.pop(identifier, None)
            if self.pinned == identifier:
                self.pinned = None
        return self.delete_result

    def send_document(self, chat_id, filename, data, caption=""):
        self.uploads += 1
        file_id = f"file_{self.uploads}"
        self.files[file_id] = data
        identifier = self.next_message_id()
        message = {
            "message_id": identifier,
            "chat": {"id": int(chat_id)},
            "from": {"id": int(self.bot_id)},
            "caption": caption,
            "document": {"file_id": file_id, "file_name": filename},
        }
        self.messages[identifier] = message
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

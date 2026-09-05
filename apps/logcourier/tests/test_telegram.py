import io
import json
import urllib.error

import pytest

from logcourier.secrets import redact, token_bot_id
from logcourier.telegram import NoRedirect, Telegram, TelegramError

TOKEN = "123456:" + "A" * 30


class Opener:
    def __init__(self, *responses):
        self.responses = iter(responses)
        self.requests = []

    def open(self, request, timeout):
        assert timeout == 30
        self.requests.append(request)
        response = next(self.responses)
        if isinstance(response, Exception):
            raise response
        return io.BytesIO(response)


def ok(result):
    return json.dumps({"ok": True, "result": result}).encode()


def test_token_validation_and_redaction():
    assert token_bot_id(TOKEN) == "123456"
    assert TOKEN not in redact(f"url/bot{TOKEN}/getMe")
    with pytest.raises(ValueError):
        token_bot_id("invalid")


def test_send_document_request_and_receipt():
    message = {"document": {"file_id": "safe_file"}, "message_id": 1, "chat": {"id": -100}}
    opener = Opener(ok(message))
    client = Telegram(TOKEN, opener)
    assert client.send_document("-100", "archive.zip", b"abc") == message
    assert b"abc" in opener.requests[0].data
    with pytest.raises(ValueError):
        client.send_document("-100", "../escape", b"x")


def test_wrong_chat_receipt_is_not_accepted():
    message = {"document": {"file_id": "safe_file"}, "message_id": 1, "chat": {"id": -999}}
    with pytest.raises(TelegramError):
        Telegram(TOKEN, Opener(ok(message))).send_document("-100", "x.zip", b"x")


@pytest.mark.parametrize(
    "path", ["../secret", "/absolute", "https://attacker.invalid/file", "a/../../b"]
)
def test_reject_download_path(path):
    client = Telegram(TOKEN, Opener(ok({"file_path": path})))
    with pytest.raises(TelegramError):
        client.download("safe_id")


def test_download_and_limit():
    client = Telegram(
        TOKEN, Opener(ok({"file_path": "documents/file.zip", "file_size": 3}), b"abc")
    )
    assert client.download("safe_id") == b"abc"
    with pytest.raises(TelegramError, match="ограничение"):
        Telegram(TOKEN, Opener(b"12345"))._open(None, 4)


def test_http_retry_and_secret_not_leaked():
    error = urllib.error.HTTPError(
        "secret",
        429,
        "retry",
        {},
        io.BytesIO(json.dumps({"description": TOKEN, "parameters": {"retry_after": 42}}).encode()),
    )
    with pytest.raises(TelegramError) as caught:
        Telegram(TOKEN, Opener(error)).call("getMe")
    assert caught.value.retry_after == 42
    assert TOKEN not in str(caught.value)
    with pytest.raises(TelegramError) as caught:
        Telegram(TOKEN, Opener(urllib.error.URLError(TOKEN))).call("getMe")
    assert TOKEN not in str(caught.value)


def test_redirect_is_blocked():
    with pytest.raises(TelegramError):
        NoRedirect().redirect_request(None, None, 302, "", {}, "https://example.com")


def test_group_discovery_does_not_acknowledge_updates():
    chat = {"id": -100, "type": "supergroup", "title": "Logs"}
    opener = Opener(ok({"url": ""}), ok([{"message": {"chat": chat}}]))
    assert Telegram(TOKEN, opener).groups() == [chat]
    assert "offset" not in json.loads(opener.requests[-1].data)
    client = Telegram(TOKEN, Opener(ok({"url": "https://existing.invalid"})))
    with pytest.raises(TelegramError, match="webhook"):
        client.groups()


@pytest.mark.parametrize("body", [b"bad", b"[]", b'{"ok":false}', b'{"ok":true}'])
def test_invalid_response(body):
    with pytest.raises(TelegramError):
        Telegram._decode(body)

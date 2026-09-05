import pytest

from logcourier import secrets

TOKEN = "123456:" + "B" * 30


class Backend:
    def __init__(self):
        self.saved = {}

    def get_password(self, service, account):
        return self.saved.get((service, account))

    def set_password(self, service, account, value):
        self.saved[service, account] = value


def test_keyring_roundtrip(monkeypatch):
    backend = Backend()
    monkeypatch.setattr(secrets, "secure_backend", lambda: backend)
    assert secrets.read_token("") == ""
    assert secrets.read_token("123456") == ""
    secrets.store_token(TOKEN)
    assert secrets.read_token("123456") == TOKEN


def test_keyring_failure_no_plaintext_fallback(monkeypatch):
    def failure():
        raise OSError(TOKEN)

    monkeypatch.setattr(secrets, "secure_backend", failure)
    with pytest.raises(RuntimeError) as result:
        secrets.store_token(TOKEN)
    assert TOKEN not in str(result.value)
    with pytest.raises(RuntimeError) as result:
        secrets.read_token("123456")
    assert TOKEN not in str(result.value)

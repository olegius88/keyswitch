"""Strict GitHub Releases update discovery and verified package download."""

from __future__ import annotations

import hashlib
import json
import logging
import platform
import re
import subprocess
import threading
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum
from functools import partial
from pathlib import Path
from typing import Protocol, cast
from urllib.parse import quote, urlsplit
from urllib.request import Request, urlopen


LOGGER = logging.getLogger(__name__)
REPOSITORY = "olegius88/keyswitch"
LATEST_RELEASE_URL = f"https://api.github.com/repos/{REPOSITORY}/releases/latest"
RELEASE_PAGE_PREFIX = f"https://github.com/{REPOSITORY}/releases/tag/"
RELEASE_DOWNLOAD_PREFIX = f"https://github.com/{REPOSITORY}/releases/download/"
GITHUB_API_VERSION = "2026-03-10"
MAX_RELEASE_JSON_BYTES = 1_000_000
MAX_ASSET_BYTES = 300 * 1024 * 1024
DOWNLOAD_CHUNK_BYTES = 128 * 1024
_VERSION_PATTERN = re.compile(r"(?:v)?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
_DIGEST_PATTERN = re.compile(r"sha256:([0-9a-f]{64})")
_WINDOWS_INSTALLER_PATTERN = re.compile(
    r"KeySwitch-Setup-(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)-x64\.exe"
)


class UpdateError(RuntimeError):
    """An update feed, download or installer failed a required check."""


@dataclass(frozen=True, order=True)
class SemanticVersion:
    major: int
    minor: int
    patch: int

    @classmethod
    def parse(cls, value: str) -> SemanticVersion:
        match = _VERSION_PATTERN.fullmatch(value)
        if match is None:
            raise UpdateError(f"Некорректная версия выпуска: {value!r}")
        return cls(*(int(part) for part in match.groups()))

    def __str__(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"


class UpdateTarget(str, Enum):
    WINDOWS_X64 = "windows-x64"
    UBUNTU_AMD64 = "ubuntu-amd64"

    def asset_name(self, version: SemanticVersion) -> str:
        if self is UpdateTarget.WINDOWS_X64:
            return f"KeySwitch-Setup-{version}-x64.exe"
        return f"keyswitch_{version}_amd64.deb"


def detect_update_target(
    system: str | None = None,
    machine: str | None = None,
) -> UpdateTarget | None:
    operating_system = (system or platform.system()).casefold()
    architecture = (machine or platform.machine()).casefold()
    if architecture not in {"amd64", "x86_64"}:
        return None
    if operating_system == "windows":
        return UpdateTarget.WINDOWS_X64
    if operating_system == "linux":
        return UpdateTarget.UBUNTU_AMD64
    return None


@dataclass(frozen=True)
class ReleaseAsset:
    name: str
    url: str
    size: int
    sha256: str


@dataclass(frozen=True)
class UpdateRelease:
    version: SemanticVersion
    tag: str
    page_url: str
    notes: str
    asset: ReleaseAsset


class _Response(Protocol):
    def read(self, amount: int = -1) -> bytes: ...

    def geturl(self) -> str: ...

    def close(self) -> None: ...


class ResponseOpener(Protocol):
    def __call__(self, request: Request, timeout: float) -> _Response: ...


def _open_url(request: Request, timeout: float) -> _Response:
    return cast(_Response, urlopen(request, timeout=timeout))


def _object_mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise UpdateError(f"GitHub вернул некорректное поле {label}")
    source = cast(dict[object, object], value)
    result: dict[str, object] = {}
    for key, item in source.items():
        if not isinstance(key, str):
            raise UpdateError(f"GitHub вернул некорректный ключ в {label}")
        result[key] = item
    return result


def _required_string(mapping: dict[str, object], key: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        raise UpdateError(f"GitHub не вернул строковое поле {key}")
    return value


def _required_size(mapping: dict[str, object]) -> int:
    value = mapping.get("size")
    if isinstance(value, bool) or not isinstance(value, int):
        raise UpdateError("GitHub не вернул размер файла обновления")
    if value <= 0 or value > MAX_ASSET_BYTES:
        raise UpdateError(f"Недопустимый размер файла обновления: {value}")
    return value


def _read_limited(response: _Response, maximum: int) -> bytes:
    payload = response.read(maximum + 1)
    if len(payload) > maximum:
        raise UpdateError("Ответ сервера обновлений превышает допустимый размер")
    return payload


def _validate_api_response_url(value: str) -> None:
    if value != LATEST_RELEASE_URL:
        raise UpdateError("GitHub API перенаправил запрос на недоверенный адрес")


def _validate_download_response_url(value: str) -> None:
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "release-assets.githubusercontent.com"
        or not parsed.path.startswith("/github-production-release-asset/")
    ):
        raise UpdateError("GitHub перенаправил загрузку на недоверенный адрес")


class GitHubReleaseClient:
    """Read and validate one stable release from the public GitHub API."""

    def __init__(
        self,
        current_version: str,
        *,
        opener: ResponseOpener = _open_url,
        timeout: float = 20.0,
    ) -> None:
        self.current_version = current_version
        self.opener = opener
        self.timeout = timeout

    def latest_release(self, target: UpdateTarget) -> UpdateRelease:
        request = Request(
            LATEST_RELEASE_URL,
            headers={
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": GITHUB_API_VERSION,
                "User-Agent": f"KeySwitch/{self.current_version}",
            },
        )
        try:
            response = self.opener(request, self.timeout)
            try:
                _validate_api_response_url(response.geturl())
                raw = _read_limited(response, MAX_RELEASE_JSON_BYTES)
            finally:
                response.close()
            decoded: object = json.loads(raw.decode("utf-8"))
        except UpdateError:
            raise
        except (OSError, UnicodeError, ValueError) as error:
            raise UpdateError(f"Не удалось прочитать GitHub Releases: {error}") from error
        payload = _object_mapping(decoded, "release")
        if payload.get("draft") is not False or payload.get("prerelease") is not False:
            raise UpdateError("GitHub вернул черновик или предварительный выпуск")

        tag = _required_string(payload, "tag_name")
        version = SemanticVersion.parse(tag)
        if tag != f"v{version}":
            raise UpdateError(f"Некорректный тег выпуска: {tag}")
        page_url = _required_string(payload, "html_url")
        if page_url != f"{RELEASE_PAGE_PREFIX}{quote(tag, safe='')}":
            raise UpdateError("Страница выпуска не относится к репозиторию KeySwitch")

        expected_name = target.asset_name(version)
        assets_value = payload.get("assets")
        if not isinstance(assets_value, list):
            raise UpdateError("GitHub не вернул список файлов выпуска")
        assets = cast(list[object], assets_value)
        matching: list[dict[str, object]] = []
        for item in assets:
            asset = _object_mapping(item, "asset")
            if asset.get("name") == expected_name:
                matching.append(asset)
        if len(matching) != 1:
            raise UpdateError(f"В выпуске отсутствует единственный файл {expected_name}")

        asset_payload = matching[0]
        if asset_payload.get("state") != "uploaded":
            raise UpdateError("Файл обновления ещё не опубликован полностью")
        asset_url = _required_string(asset_payload, "browser_download_url")
        expected_url = (
            f"{RELEASE_DOWNLOAD_PREFIX}{quote(tag, safe='')}/"
            f"{quote(expected_name, safe='')}"
        )
        if asset_url != expected_url:
            raise UpdateError("Файл обновления ссылается на недоверенный адрес")
        digest = _required_string(asset_payload, "digest")
        digest_match = _DIGEST_PATTERN.fullmatch(digest)
        if digest_match is None:
            raise UpdateError("GitHub не вернул обязательную контрольную сумму SHA-256")
        notes_value = payload.get("body", "")
        notes = notes_value[:4000] if isinstance(notes_value, str) else ""
        return UpdateRelease(
            version,
            tag,
            page_url,
            notes,
            ReleaseAsset(
                expected_name,
                asset_url,
                _required_size(asset_payload),
                digest_match.group(1),
            ),
        )

    def download(
        self,
        release: UpdateRelease,
        directory: Path,
        progress: Callable[[int, int], None],
    ) -> Path:
        directory.mkdir(parents=True, exist_ok=True)
        destination = directory / release.asset.name
        partial_path = directory / f"{release.asset.name}.part"
        _unlink_if_exists(partial_path)
        request = Request(
            release.asset.url,
            headers={"User-Agent": f"KeySwitch/{self.current_version}"},
        )
        try:
            response = self.opener(request, self.timeout)
            try:
                _validate_download_response_url(response.geturl())
                digest = hashlib.sha256()
                downloaded = 0
                with partial_path.open("wb") as handle:
                    while True:
                        chunk = response.read(DOWNLOAD_CHUNK_BYTES)
                        if not chunk:
                            break
                        downloaded += len(chunk)
                        if downloaded > release.asset.size:
                            raise UpdateError("Размер загруженного обновления превышает объявленный")
                        handle.write(chunk)
                        digest.update(chunk)
                        progress(downloaded, release.asset.size)
            finally:
                response.close()
            if downloaded != release.asset.size:
                raise UpdateError("Размер загруженного обновления не совпадает с объявленным")
            if digest.hexdigest() != release.asset.sha256:
                raise UpdateError("Контрольная сумма обновления SHA-256 не совпала")
            partial_path.replace(destination)
            return destination
        except UpdateError:
            _unlink_if_exists(partial_path)
            raise
        except OSError as error:
            _unlink_if_exists(partial_path)
            raise UpdateError(f"Не удалось загрузить обновление: {error}") from error


def _unlink_if_exists(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


class ReleaseClient(Protocol):
    def latest_release(self, target: UpdateTarget) -> UpdateRelease: ...

    def download(
        self,
        release: UpdateRelease,
        directory: Path,
        progress: Callable[[int, int], None],
    ) -> Path: ...


class UpdatePhase(str, Enum):
    IDLE = "idle"
    CHECKING = "checking"
    CURRENT = "current"
    AVAILABLE = "available"
    DOWNLOADING = "downloading"
    INSTALLING = "installing"
    ERROR = "error"


@dataclass(frozen=True)
class UpdateSnapshot:
    phase: UpdatePhase
    message: str
    current_version: str
    available_version: str = ""
    release_url: str = ""
    progress: int = 0


UpdateObserver = Callable[[UpdateSnapshot], None]
BackgroundRunner = Callable[[Callable[[], None]], None]
InstallRequest = Callable[[Path], None]


def run_in_background(action: Callable[[], None]) -> None:
    threading.Thread(target=action, name="keyswitch-update", daemon=True).start()


class UpdateController(Protocol):
    @property
    def snapshot(self) -> UpdateSnapshot: ...

    def subscribe(self, callback: UpdateObserver) -> None: ...

    def check(self, *, automatic: bool, install_automatically: bool) -> bool: ...

    def install_available(self) -> bool: ...

    def installation_failed(self, error: Exception) -> None: ...


class UpdateManager:
    """Serialize background checks/downloads and publish immutable UI state."""

    def __init__(
        self,
        current_version: str,
        directory: Path,
        *,
        target: UpdateTarget | None = None,
        client: ReleaseClient | None = None,
        runner: BackgroundRunner = run_in_background,
        install_request: InstallRequest | None = None,
    ) -> None:
        self.current = SemanticVersion.parse(current_version)
        self.directory = directory
        self.target = target if target is not None else detect_update_target()
        self.client = client or GitHubReleaseClient(current_version)
        self.runner = runner
        self.install_request = install_request
        self._lock = threading.RLock()
        self._observers: list[UpdateObserver] = []
        self._release: UpdateRelease | None = None
        self._busy = False
        self._closed = False
        self._snapshot = UpdateSnapshot(
            UpdatePhase.IDLE,
            "Обновления ещё не проверялись",
            str(self.current),
        )

    @property
    def snapshot(self) -> UpdateSnapshot:
        with self._lock:
            return self._snapshot

    def subscribe(self, callback: UpdateObserver) -> None:
        with self._lock:
            if self._closed:
                return
            self._observers.append(callback)
            snapshot = self._snapshot
        callback(snapshot)

    def _notify(self, observers: tuple[UpdateObserver, ...], snapshot: UpdateSnapshot) -> None:
        for observer in observers:
            try:
                observer(snapshot)
            except Exception:
                LOGGER.exception("Ошибка подписчика состояния обновлений")

    def _transition(
        self,
        snapshot: UpdateSnapshot,
        *,
        busy: bool | None = None,
    ) -> bool:
        with self._lock:
            if self._closed:
                return False
            if busy is not None:
                self._busy = busy
            self._snapshot = snapshot
            observers = tuple(self._observers)
        self._notify(observers, snapshot)
        return True

    def _release_snapshot(
        self,
        phase: UpdatePhase,
        message: str,
        release: UpdateRelease,
        *,
        progress: int = 0,
    ) -> UpdateSnapshot:
        return UpdateSnapshot(
            phase,
            message,
            str(self.current),
            str(release.version),
            release.page_url,
            progress,
        )

    def check(self, *, automatic: bool, install_automatically: bool) -> bool:
        checking = UpdateSnapshot(
            UpdatePhase.CHECKING,
            "Проверяем GitHub Releases…",
            str(self.current),
        )
        with self._lock:
            if self._closed or self._busy:
                return False
            self._busy = True
            self._snapshot = checking
            observers = tuple(self._observers)
        self._notify(observers, checking)
        try:
            self.runner(
                partial(
                    self._check_worker,
                    automatic=automatic,
                    install_automatically=install_automatically,
                )
            )
        except Exception as error:
            self._fail(error)
            return False
        return True

    def _check_worker(self, *, automatic: bool, install_automatically: bool) -> None:
        target = self.target
        if target is None:
            self._fail(UpdateError("Автообновление не поддерживает эту архитектуру"))
            return
        try:
            release = self.client.latest_release(target)
        except Exception as error:
            self._fail(error)
            return
        if release.version <= self.current:
            with self._lock:
                self._release = None
            self._transition(
                UpdateSnapshot(
                    UpdatePhase.CURRENT,
                    f"Установлена актуальная версия {self.current}",
                    str(self.current),
                ),
                busy=False,
            )
            return
        with self._lock:
            self._release = release
        if not self._transition(
            self._release_snapshot(
                UpdatePhase.AVAILABLE,
                f"Доступна версия {release.version}",
                release,
            )
        ):
            return
        if (
            automatic
            and install_automatically
            and target is UpdateTarget.WINDOWS_X64
        ):
            self._download_worker(release)
            return
        self._set_busy(False)

    def _set_busy(self, value: bool) -> None:
        with self._lock:
            if not self._closed:
                self._busy = value

    def install_available(self) -> bool:
        with self._lock:
            release = self._release
            if (
                self._closed
                or self._busy
                or release is None
                or self.target is not UpdateTarget.WINDOWS_X64
            ):
                return False
            self._busy = True
        downloading = self._release_snapshot(
            UpdatePhase.DOWNLOADING,
            f"Загружаем версию {release.version}: 0%",
            release,
        )
        self._transition(downloading)
        try:
            self.runner(partial(self._download_worker, release))
        except Exception as error:
            self._fail(error)
            return False
        return True

    def _download_worker(self, release: UpdateRelease) -> None:
        self._transition(
            self._release_snapshot(
                UpdatePhase.DOWNLOADING,
                f"Загружаем версию {release.version}: 0%",
                release,
            )
        )
        try:
            path = self.client.download(
                release,
                self.directory,
                partial(self._download_progress, release),
            )
        except Exception as error:
            self._fail(error)
            return
        installing = self._release_snapshot(
            UpdatePhase.INSTALLING,
            f"Запускаем установку версии {release.version}",
            release,
            progress=100,
        )
        if not self._transition(installing, busy=False):
            return
        request = self.install_request
        if request is None:
            self._fail(UpdateError("Не настроен запуск установщика обновления"))
            return
        try:
            request(path)
        except Exception as error:
            self._fail(error)

    def _download_progress(
        self,
        release: UpdateRelease,
        downloaded: int,
        total: int,
    ) -> None:
        percentage = min(100, int(downloaded * 100 / max(1, total)))
        self._transition(
            self._release_snapshot(
                UpdatePhase.DOWNLOADING,
                f"Загружаем версию {release.version}: {percentage}%",
                release,
                progress=percentage,
            )
        )

    def installation_failed(self, error: Exception) -> None:
        self._fail(error)

    def _fail(self, error: Exception) -> None:
        self._transition(
            UpdateSnapshot(
                UpdatePhase.ERROR,
                f"Обновление не выполнено: {error}",
                str(self.current),
            ),
            busy=False,
        )

    def close(self) -> None:
        with self._lock:
            self._closed = True
            self._busy = False
            self._observers.clear()


ProcessSpawner = Callable[[list[str]], None]


def _spawn_process(arguments: list[str]) -> None:
    subprocess.Popen(arguments, close_fds=True)


def launch_windows_installer(
    path: Path,
    *,
    spawn: ProcessSpawner = _spawn_process,
) -> None:
    if not path.is_file():
        raise UpdateError(f"Файл установщика не найден: {path}")
    if _WINDOWS_INSTALLER_PATTERN.fullmatch(path.name) is None:
        raise UpdateError("Имя установщика обновления не прошло проверку")
    spawn(
        [
            str(path),
            "/SP-",
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
            "/NORESTARTAPPLICATIONS",
            "/KEYSWITCHUPDATE=1",
        ]
    )

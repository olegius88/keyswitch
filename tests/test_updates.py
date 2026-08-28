"""Security, state-machine and installer tests for application updates."""

from __future__ import annotations

import hashlib
import io
import json
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from urllib.request import Request
from unittest.mock import Mock, patch

from keyswitch import updates
from keyswitch.updates import (
    GitHubReleaseClient,
    ReleaseAsset,
    SemanticVersion,
    UpdateError,
    UpdateManager,
    UpdatePhase,
    UpdateRelease,
    UpdateSnapshot,
    UpdateTarget,
    detect_update_target,
    launch_windows_installer,
)


DOWNLOAD_URL = (
    "https://release-assets.githubusercontent.com/"
    "github-production-release-asset/1/verified"
)


class FakeResponse:
    def __init__(
        self,
        payload: bytes,
        url: str,
        *,
        read_error: OSError | None = None,
        error_after_reads: int = 0,
    ) -> None:
        self.stream = io.BytesIO(payload)
        self.url = url
        self.read_error = read_error
        self.error_after_reads = error_after_reads
        self.read_calls = 0
        self.closed = 0

    def read(self, amount: int = -1) -> bytes:
        self.read_calls += 1
        if self.read_error is not None and self.read_calls > self.error_after_reads:
            raise self.read_error
        return self.stream.read(amount)

    def geturl(self) -> str:
        return self.url

    def close(self) -> None:
        self.closed += 1


class FakeOpener:
    def __init__(self, *results: FakeResponse | OSError) -> None:
        self.results = list(results)
        self.calls: list[tuple[Request, float]] = []

    def __call__(self, request: Request, timeout: float) -> FakeResponse:
        self.calls.append((request, timeout))
        result = self.results.pop(0)
        if isinstance(result, OSError):
            raise result
        return result


def release_payload(
    version: str = "1.2.3",
    target: UpdateTarget = UpdateTarget.WINDOWS_X64,
    *,
    content: bytes = b"package",
) -> dict[str, object]:
    semantic = SemanticVersion.parse(version)
    tag = f"v{semantic}"
    name = target.asset_name(semantic)
    return {
        "draft": False,
        "prerelease": False,
        "tag_name": tag,
        "html_url": f"{updates.RELEASE_PAGE_PREFIX}{tag}",
        "body": "Release notes",
        "assets": [
            {
                "name": name,
                "state": "uploaded",
                "browser_download_url": (
                    f"{updates.RELEASE_DOWNLOAD_PREFIX}{tag}/{name}"
                ),
                "digest": f"sha256:{hashlib.sha256(content).hexdigest()}",
                "size": len(content),
            }
        ],
    }


def response_for(payload: object, *, url: str = updates.LATEST_RELEASE_URL) -> FakeResponse:
    return FakeResponse(json.dumps(payload).encode("utf-8"), url)


def checked_release(
    target: UpdateTarget = UpdateTarget.WINDOWS_X64,
    *,
    version: str = "1.2.3",
    content: bytes = b"package",
) -> UpdateRelease:
    opener = FakeOpener(response_for(release_payload(version, target, content=content)))
    return GitHubReleaseClient("0.4.0", opener=opener).latest_release(target)


class VersionAndValidationTests(unittest.TestCase):
    def test_versions_targets_and_platform_detection(self) -> None:
        self.assertEqual(str(SemanticVersion.parse("v1.2.3")), "1.2.3")
        self.assertLess(SemanticVersion.parse("0.9.9"), SemanticVersion.parse("1.0.0"))
        for invalid in ("", "1.2", "01.2.3", "1.2.3-beta"):
            with self.subTest(invalid=invalid), self.assertRaises(UpdateError):
                SemanticVersion.parse(invalid)
        version = SemanticVersion(2, 3, 4)
        self.assertEqual(
            UpdateTarget.WINDOWS_X64.asset_name(version),
            "KeySwitch-Setup-2.3.4-x64.exe",
        )
        self.assertEqual(
            UpdateTarget.UBUNTU_AMD64.asset_name(version),
            "keyswitch_2.3.4_amd64.deb",
        )
        self.assertIs(
            detect_update_target("Windows", "AMD64"), UpdateTarget.WINDOWS_X64
        )
        self.assertIs(
            detect_update_target("Linux", "x86_64"), UpdateTarget.UBUNTU_AMD64
        )
        self.assertIsNone(detect_update_target("Linux", "aarch64"))
        self.assertIsNone(detect_update_target("Darwin", "x86_64"))
        with (
            patch("keyswitch.updates.platform.system", return_value="Linux"),
            patch("keyswitch.updates.platform.machine", return_value="amd64"),
        ):
            self.assertIs(detect_update_target(), UpdateTarget.UBUNTU_AMD64)

    def test_low_level_validation_boundaries(self) -> None:
        with self.assertRaises(UpdateError):
            updates._object_mapping([], "item")
        with self.assertRaises(UpdateError):
            updates._object_mapping({1: "bad"}, "item")
        self.assertEqual(updates._object_mapping({"ok": 1}, "item"), {"ok": 1})
        for value in (None, "", 3):
            with self.subTest(value=value), self.assertRaises(UpdateError):
                updates._required_string({"field": value}, "field")
        for value in (True, "3", 0, updates.MAX_ASSET_BYTES + 1):
            with self.subTest(value=value), self.assertRaises(UpdateError):
                updates._required_size({"size": value})
        self.assertEqual(updates._required_size({"size": 3}), 3)

        oversized = FakeResponse(b"1234", updates.LATEST_RELEASE_URL)
        with self.assertRaises(UpdateError):
            updates._read_limited(oversized, 3)
        with self.assertRaises(UpdateError):
            updates._validate_api_response_url("https://example.invalid/latest")
        updates._validate_api_response_url(updates.LATEST_RELEASE_URL)

        updates._validate_download_response_url(DOWNLOAD_URL)
        for url in (
            DOWNLOAD_URL.replace("https", "http", 1),
            "https://example.invalid/github-production-release-asset/1/file",
            "https://release-assets.githubusercontent.com/untrusted/file",
        ):
            with self.subTest(url=url), self.assertRaises(UpdateError):
                updates._validate_download_response_url(url)

    def test_default_url_opener_and_background_runner_boundaries(self) -> None:
        response = FakeResponse(b"{}", updates.LATEST_RELEASE_URL)

        def open_url(request: Request, *, timeout: float) -> FakeResponse:
            self.assertEqual(request.full_url, updates.LATEST_RELEASE_URL)
            self.assertEqual(timeout, 7.0)
            return response

        with patch("keyswitch.updates.urlopen", side_effect=open_url):
            request = Request(updates.LATEST_RELEASE_URL)
            self.assertIs(updates._open_url(request, 7.0), response)

        thread = Mock()
        with patch("keyswitch.updates.threading.Thread", return_value=thread) as factory:
            action = Mock()
            updates.run_in_background(action)
        factory.assert_called_once_with(
            target=action,
            name="keyswitch-update",
            daemon=True,
        )
        thread.start.assert_called_once_with()


class GitHubReleaseClientTests(unittest.TestCase):
    def latest(self, payload: object, *, url: str = updates.LATEST_RELEASE_URL) -> UpdateRelease:
        opener = FakeOpener(response_for(payload, url=url))
        return GitHubReleaseClient("0.4.0", opener=opener, timeout=9).latest_release(
            UpdateTarget.WINDOWS_X64
        )

    def test_success_validates_request_and_platform_assets(self) -> None:
        payload = release_payload()
        payload["body"] = "x" * 5000
        response = response_for(payload)
        opener = FakeOpener(response)
        release = GitHubReleaseClient("0.4.0", opener=opener, timeout=9).latest_release(
            UpdateTarget.WINDOWS_X64
        )
        self.assertEqual(str(release.version), "1.2.3")
        self.assertEqual(len(release.notes), 4000)
        self.assertEqual(release.asset.size, 7)
        request, timeout = opener.calls[0]
        self.assertEqual((request.full_url, timeout), (updates.LATEST_RELEASE_URL, 9))
        self.assertEqual(request.get_header("User-agent"), "KeySwitch/0.4.0")
        self.assertEqual(response.closed, 1)

        linux_payload = release_payload(target=UpdateTarget.UBUNTU_AMD64)
        linux_payload["body"] = 123
        linux = GitHubReleaseClient(
            "0.4.0",
            opener=FakeOpener(response_for(linux_payload)),
        ).latest_release(UpdateTarget.UBUNTU_AMD64)
        self.assertEqual(linux.asset.name, "keyswitch_1.2.3_amd64.deb")
        self.assertEqual(linux.notes, "")

    def test_transport_encoding_and_response_limits_fail_closed(self) -> None:
        cases: tuple[FakeResponse | OSError, ...] = (
            OSError("offline"),
            FakeResponse(b"\xff", updates.LATEST_RELEASE_URL),
            FakeResponse(b"not-json", updates.LATEST_RELEASE_URL),
            FakeResponse(
                b"x" * (updates.MAX_RELEASE_JSON_BYTES + 1),
                updates.LATEST_RELEASE_URL,
            ),
            response_for(release_payload(), url="https://example.invalid/latest"),
        )
        for result in cases:
            with self.subTest(result=type(result).__name__), self.assertRaises(UpdateError):
                GitHubReleaseClient("0.4.0", opener=FakeOpener(result)).latest_release(
                    UpdateTarget.WINDOWS_X64
                )
            if isinstance(result, FakeResponse):
                self.assertEqual(result.closed, 1)

    def test_release_and_asset_schema_rejections(self) -> None:
        payloads: list[object] = [[], {"draft": True, "prerelease": False}]

        prerelease = release_payload()
        prerelease["prerelease"] = True
        payloads.append(prerelease)

        missing_tag = release_payload()
        missing_tag["tag_name"] = ""
        payloads.append(missing_tag)

        invalid_tag = release_payload()
        invalid_tag["tag_name"] = "v1.2"
        payloads.append(invalid_tag)

        unprefixed_tag = release_payload()
        unprefixed_tag["tag_name"] = "1.2.3"
        payloads.append(unprefixed_tag)

        bad_page = release_payload()
        bad_page["html_url"] = "https://example.invalid/v1.2.3"
        payloads.append(bad_page)

        no_assets = release_payload()
        no_assets["assets"] = {}
        payloads.append(no_assets)

        malformed_asset = release_payload()
        malformed_asset["assets"] = ["asset"]
        payloads.append(malformed_asset)

        missing_asset = release_payload()
        missing_asset["assets"] = []
        payloads.append(missing_asset)

        unrelated_asset = release_payload()
        unrelated_values = unrelated_asset["assets"]
        assert isinstance(unrelated_values, list)
        unrelated_item = unrelated_values[0]
        assert isinstance(unrelated_item, dict)
        unrelated_item["name"] = "unrelated.zip"
        payloads.append(unrelated_asset)

        duplicate = release_payload()
        assets = duplicate["assets"]
        assert isinstance(assets, list)
        assets.append(dict(assets[0]))
        payloads.append(duplicate)

        for field, value in (
            ("state", "new"),
            ("browser_download_url", "https://example.invalid/file.exe"),
            ("digest", "md5:bad"),
            ("size", True),
            ("size", 0),
        ):
            payload = release_payload()
            asset_values = payload["assets"]
            assert isinstance(asset_values, list)
            asset = asset_values[0]
            assert isinstance(asset, dict)
            asset[field] = value
            payloads.append(payload)

        for index, candidate in enumerate(payloads):
            with self.subTest(index=index), self.assertRaises(UpdateError):
                self.latest(candidate)

    def test_download_success_and_all_integrity_failures(self) -> None:
        content = b"verified package"
        release = checked_release(content=content)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            partial_path = directory / f"{release.asset.name}.part"
            partial_path.write_bytes(b"stale")
            response = FakeResponse(content, DOWNLOAD_URL)
            progress: list[tuple[int, int]] = []
            client = GitHubReleaseClient("0.4.0", opener=FakeOpener(response))
            installed = client.download(release, directory, lambda done, total: progress.append((done, total)))
            self.assertEqual(installed.read_bytes(), content)
            self.assertFalse(partial_path.exists())
            self.assertEqual(progress[-1], (len(content), len(content)))
            self.assertEqual(response.closed, 1)

            failures = (
                (
                    UpdateRelease(
                        release.version,
                        release.tag,
                        release.page_url,
                        release.notes,
                        ReleaseAsset(release.asset.name, release.asset.url, 2, release.asset.sha256),
                    ),
                    FakeResponse(content, DOWNLOAD_URL),
                ),
                (
                    UpdateRelease(
                        release.version,
                        release.tag,
                        release.page_url,
                        release.notes,
                        ReleaseAsset(release.asset.name, release.asset.url, len(content) + 1, release.asset.sha256),
                    ),
                    FakeResponse(content, DOWNLOAD_URL),
                ),
                (
                    UpdateRelease(
                        release.version,
                        release.tag,
                        release.page_url,
                        release.notes,
                        ReleaseAsset(release.asset.name, release.asset.url, len(content), "0" * 64),
                    ),
                    FakeResponse(content, DOWNLOAD_URL),
                ),
                (release, FakeResponse(content, "https://example.invalid/file")),
                (
                    release,
                    FakeResponse(
                        content,
                        DOWNLOAD_URL,
                        read_error=OSError("connection reset"),
                    ),
                ),
            )
            for failed_release, failed_response in failures:
                with self.subTest(size=failed_release.asset.size, url=failed_response.url):
                    failed_client = GitHubReleaseClient(
                        "0.4.0", opener=FakeOpener(failed_response)
                    )
                    with self.assertRaises(UpdateError):
                        failed_client.download(
                            failed_release,
                            directory,
                            lambda _done, _total: None,
                        )
                    self.assertFalse(partial_path.exists())
                    self.assertEqual(failed_response.closed, 1)

            offline = GitHubReleaseClient(
                "0.4.0", opener=FakeOpener(OSError("offline"))
            )
            with self.assertRaises(UpdateError):
                offline.download(release, directory, lambda _done, _total: None)

            with patch.object(Path, "replace", side_effect=OSError("readonly")):
                with self.assertRaises(UpdateError):
                    GitHubReleaseClient(
                        "0.4.0",
                        opener=FakeOpener(FakeResponse(content, DOWNLOAD_URL)),
                    ).download(release, directory, lambda _done, _total: None)

    def test_partial_cleanup_helper_handles_present_and_missing_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "partial"
            updates._unlink_if_exists(path)
            path.write_text("data", encoding="utf-8")
            updates._unlink_if_exists(path)
            self.assertFalse(path.exists())


class FakeReleaseClient:
    def __init__(self, release: UpdateRelease) -> None:
        self.release = release
        self.latest_error: Exception | None = None
        self.download_error: Exception | None = None
        self.latest_targets: list[UpdateTarget] = []
        self.downloads = 0
        self.latest_hook: Callable[[], None] | None = None
        self.download_hook: Callable[[], None] | None = None

    def latest_release(self, target: UpdateTarget) -> UpdateRelease:
        self.latest_targets.append(target)
        if self.latest_error is not None:
            raise self.latest_error
        if self.latest_hook is not None:
            self.latest_hook()
        return self.release

    def download(
        self,
        _release: UpdateRelease,
        directory: Path,
        progress: Callable[[int, int], None],
    ) -> Path:
        self.downloads += 1
        progress(5, 0)
        progress(20, 10)
        if self.download_hook is not None:
            self.download_hook()
        if self.download_error is not None:
            raise self.download_error
        return directory / self.release.asset.name


def immediate(action: Callable[[], None]) -> None:
    action()


class UpdateManagerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.release = checked_release(version="1.0.0")
        self.client = FakeReleaseClient(self.release)
        self.installed: list[Path] = []
        self.directory = Path("/tmp/keyswitch-update-tests")

    def manager(
        self,
        *,
        target: UpdateTarget = UpdateTarget.WINDOWS_X64,
        runner: Callable[[Callable[[], None]], None] = immediate,
        installer: Callable[[Path], None] | None = None,
    ) -> UpdateManager:
        return UpdateManager(
            "0.4.0",
            self.directory,
            target=target,
            client=self.client,
            runner=runner,
            install_request=installer,
        )

    def test_current_available_and_observer_isolation(self) -> None:
        current_client = FakeReleaseClient(checked_release(version="0.4.0"))
        manager = UpdateManager(
            "0.4.0",
            self.directory,
            target=UpdateTarget.UBUNTU_AMD64,
            client=current_client,
            runner=immediate,
        )
        states: list[UpdateSnapshot] = []
        manager.subscribe(states.append)
        observer_failed = False

        def bad_observer(_snapshot: UpdateSnapshot) -> None:
            nonlocal observer_failed
            if _snapshot.phase is not UpdatePhase.IDLE and not observer_failed:
                observer_failed = True
                raise RuntimeError("observer")

        manager.subscribe(bad_observer)
        with patch.object(updates.LOGGER, "exception") as logged:
            self.assertTrue(manager.check(automatic=False, install_automatically=False))
        logged.assert_called()
        self.assertEqual(manager.snapshot.phase, UpdatePhase.CURRENT)
        self.assertEqual(states[0].phase, UpdatePhase.IDLE)
        self.assertEqual(current_client.latest_targets, [UpdateTarget.UBUNTU_AMD64])

        current_client.release = self.release
        self.assertTrue(manager.check(automatic=True, install_automatically=True))
        self.assertEqual(manager.snapshot.phase, UpdatePhase.AVAILABLE)
        self.assertEqual(manager.snapshot.available_version, "1.0.0")
        self.assertFalse(manager.install_available())

    def test_automatic_and_manual_windows_install_paths(self) -> None:
        manager = self.manager(installer=self.installed.append)
        self.assertTrue(manager.check(automatic=True, install_automatically=True))
        self.assertEqual(manager.snapshot.phase, UpdatePhase.INSTALLING)
        self.assertEqual(manager.snapshot.progress, 100)
        self.assertEqual(self.client.downloads, 1)
        self.assertEqual(len(self.installed), 1)

        manual = self.manager(installer=self.installed.append)
        self.assertTrue(manual.check(automatic=False, install_automatically=True))
        self.assertEqual(manual.snapshot.phase, UpdatePhase.AVAILABLE)
        self.assertTrue(manual.install_available())
        self.assertEqual(manual.snapshot.phase, UpdatePhase.INSTALLING)
        self.assertEqual(self.client.downloads, 2)

    def test_busy_runner_and_client_errors(self) -> None:
        queued: list[Callable[[], None]] = []
        manager = self.manager(runner=queued.append)
        self.assertTrue(manager.check(automatic=False, install_automatically=False))
        self.assertFalse(manager.check(automatic=False, install_automatically=False))
        self.assertFalse(manager.install_available())
        queued.pop()()
        self.assertEqual(manager.snapshot.phase, UpdatePhase.AVAILABLE)

        def failed_runner(_action: Callable[[], None]) -> None:
            raise RuntimeError("thread unavailable")

        runner_failure = self.manager(runner=failed_runner)
        self.assertFalse(
            runner_failure.check(automatic=False, install_automatically=False)
        )
        self.assertEqual(runner_failure.snapshot.phase, UpdatePhase.ERROR)

        self.client.latest_error = UpdateError("feed failed")
        feed_failure = self.manager()
        self.assertTrue(feed_failure.check(automatic=False, install_automatically=False))
        self.assertIn("feed failed", feed_failure.snapshot.message)
        self.client.latest_error = None

        manual_runner_failure = self.manager()
        self.assertTrue(
            manual_runner_failure.check(
                automatic=False,
                install_automatically=False,
            )
        )
        manual_runner_failure.runner = failed_runner
        self.assertFalse(manual_runner_failure.install_available())
        self.assertEqual(manual_runner_failure.snapshot.phase, UpdatePhase.ERROR)

    def test_unsupported_closed_and_close_during_download(self) -> None:
        with patch("keyswitch.updates.detect_update_target", return_value=None):
            unsupported = UpdateManager(
                "0.4.0",
                self.directory,
                client=self.client,
                runner=immediate,
            )
        self.assertTrue(unsupported.check(automatic=False, install_automatically=False))
        self.assertIn("архитектуру", unsupported.snapshot.message)

        manager = self.manager(installer=self.installed.append)
        manager.close()
        manager.subscribe(Mock())
        self.assertFalse(manager.check(automatic=False, install_automatically=False))
        self.assertFalse(manager.install_available())
        self.assertFalse(
            manager._transition(
                UpdateSnapshot(UpdatePhase.ERROR, "ignored", "0.4.0")
            )
        )
        manager._set_busy(False)
        manager.installation_failed(RuntimeError("ignored"))

        closing = self.manager(installer=self.installed.append)
        self.client.download_hook = closing.close
        self.assertTrue(closing.check(automatic=True, install_automatically=True))
        self.assertEqual(len(self.installed), 0)
        self.client.download_hook = None

        closing_during_check = self.manager(installer=self.installed.append)
        self.client.latest_hook = closing_during_check.close
        self.assertTrue(
            closing_during_check.check(
                automatic=True,
                install_automatically=True,
            )
        )
        self.assertEqual(self.client.downloads, 1)
        self.client.latest_hook = None

    def test_download_and_install_failures(self) -> None:
        self.client.download_error = OSError("disk full")
        download_failure = self.manager(installer=self.installed.append)
        self.assertTrue(
            download_failure.check(automatic=True, install_automatically=True)
        )
        self.assertIn("disk full", download_failure.snapshot.message)
        self.client.download_error = None

        missing_installer = self.manager()
        self.assertTrue(
            missing_installer.check(automatic=True, install_automatically=True)
        )
        self.assertIn("Не настроен", missing_installer.snapshot.message)

        def failed_install(_path: Path) -> None:
            raise OSError("cannot execute")

        install_failure = self.manager(installer=failed_install)
        self.assertTrue(
            install_failure.check(automatic=True, install_automatically=True)
        )
        self.assertIn("cannot execute", install_failure.snapshot.message)
        install_failure.installation_failed(RuntimeError("reported failure"))
        self.assertIn("reported failure", install_failure.snapshot.message)

    def test_default_dependencies_are_composed(self) -> None:
        client = Mock()
        with (
            patch("keyswitch.updates.detect_update_target", return_value=UpdateTarget.UBUNTU_AMD64),
            patch("keyswitch.updates.GitHubReleaseClient", return_value=client) as factory,
        ):
            manager = UpdateManager("0.4.0", self.directory)
        self.assertIs(manager.client, client)
        self.assertIs(manager.target, UpdateTarget.UBUNTU_AMD64)
        factory.assert_called_once_with("0.4.0")


class WindowsInstallerLaunchTests(unittest.TestCase):
    def test_packaging_and_ci_keep_the_auto_update_relaunch_contract(self) -> None:
        project = Path(__file__).resolve().parents[1]
        installer_script = (project / "packaging/windows/KeySwitch.iss").read_text(
            encoding="utf-8"
        )
        for required in (
            "{param:KEYSWITCHUPDATE|0}",
            'Parameters: "--hidden"',
            "skipifnotsilent",
        ):
            self.assertIn(required, installer_script)
        for workflow_name in ("tests.yml", "release.yml"):
            workflow = (
                project / ".github/workflows" / workflow_name
            ).read_text(encoding="utf-8")
            for required in (
                "/KEYSWITCHUPDATE=1",
                "Auto-update installer did not relaunch KeySwitch",
                "tests/test_updates.py",
                "*/updates.py",
            ):
                self.assertIn(required, workflow)

    def test_installer_path_and_arguments_are_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(UpdateError):
                launch_windows_installer(root / "missing.exe", spawn=Mock())
            bad = root / "other.exe"
            bad.write_bytes(b"installer")
            with self.assertRaises(UpdateError):
                launch_windows_installer(bad, spawn=Mock())

            installer = root / "KeySwitch-Setup-1.2.3-x64.exe"
            installer.write_bytes(b"installer")
            calls: list[list[str]] = []
            launch_windows_installer(installer, spawn=calls.append)
            self.assertEqual(calls[0][0], str(installer))
            self.assertEqual(
                calls[0][1:],
                [
                    "/SP-",
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/CLOSEAPPLICATIONS",
                    "/NORESTARTAPPLICATIONS",
                    "/KEYSWITCHUPDATE=1",
                ],
            )

    def test_default_process_spawner_delegates_to_subprocess(self) -> None:
        with patch("keyswitch.updates.subprocess.Popen") as popen:
            updates._spawn_process(["installer.exe", "/SILENT"])
        popen.assert_called_once_with(
            ["installer.exe", "/SILENT"],
            close_fds=True,
        )


if __name__ == "__main__":
    unittest.main()

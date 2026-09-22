"""Tests for the APT repository that keeps an installed KeySwitch current."""

from __future__ import annotations

import gzip
import hashlib
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import apt_repository  # noqa: E402


CONTROL_STANZA = "\n".join(
    (
        "Package: keyswitch",
        "Version: 1.2.3",
        "Architecture: amd64",
        "Maintainer: Example <nobody@example.invalid>",
        "Description: example package",
        " A description that continues on its own line.",
    )
)


def _fake_run(command: list[str], **overrides: str) -> str:
    """Answer the dpkg-deb queries the builder makes about a package."""

    if command[:2] != ["dpkg-deb", "--field"]:
        raise AssertionError(f"unexpected command: {command}")
    if len(command) == 3:
        return CONTROL_STANZA + "\n"
    field = command[3]
    for line in CONTROL_STANZA.splitlines():
        if line.startswith(f"{field}:"):
            return line.split(":", 1)[1].strip() + "\n"
    raise AssertionError(f"unexpected field: {field}")


def _fake_sign(release_path: Path, passphrase_file: Path | None) -> None:
    """Stand in for GnuPG: wrap the release the way a clear-sign would."""

    release = release_path.read_text(encoding="utf-8")
    directory = release_path.parent
    (directory / apt_repository.INLINE_SIGNATURE_FILE_NAME).write_text(
        f"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n\n{release}"
        "-----BEGIN PGP SIGNATURE-----\n-----END PGP SIGNATURE-----\n",
        encoding="utf-8",
    )
    (directory / apt_repository.DETACHED_SIGNATURE_FILE_NAME).write_text(
        "-----BEGIN PGP SIGNATURE-----\n-----END PGP SIGNATURE-----\n",
        encoding="utf-8",
    )


class ShippedSourceTest(unittest.TestCase):
    """The stanza the package installs is the one thing both sides read."""

    def test_reads_the_publishing_coordinates(self) -> None:
        sources = apt_repository.read_sources()
        self.assertTrue(sources.uri.startswith("https://"))
        self.assertTrue(sources.uri.endswith("/"))
        self.assertTrue(sources.keyring_path.startswith("/etc/apt/keyrings/"))
        self.assertEqual(
            sources.index_directory,
            Path("dists")
            / sources.suite
            / sources.component
            / f"binary-{sources.architecture}",
        )

    def test_the_shipped_source_is_enabled(self) -> None:
        stanza = apt_repository.SOURCES_STANZA_PATH.read_text(encoding="utf-8")
        for line in stanza.splitlines():
            if line.startswith("Enabled:"):
                self.assertEqual(line.split(":", 1)[1].strip(), "yes")

    def test_the_shipped_key_carries_no_secret(self) -> None:
        key = apt_repository.PUBLIC_KEY_PATH.read_text(encoding="utf-8")
        self.assertIn("BEGIN PGP PUBLIC KEY BLOCK", key)
        self.assertNotIn("PRIVATE KEY BLOCK", key)

    def test_a_missing_field_is_named(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            apt_repository._stanza_field("Types: deb\n", "URIs")
        self.assertIn("URIs", str(raised.exception))

    def test_a_field_may_not_carry_two_values(self) -> None:
        with self.assertRaises(SystemExit):
            apt_repository._stanza_field("Suites: stable testing\n", "Suites")

    def test_comments_are_not_fields(self) -> None:
        with self.assertRaises(SystemExit):
            apt_repository._stanza_field("#Suites: stable\n", "Suites")


class BuildTest(unittest.TestCase):
    """The published tree has to be exactly what APT expects to fetch."""

    def _build(self, directory: Path) -> tuple[Path, apt_repository.RepositorySources]:
        package = directory / "keyswitch_1.2.3_amd64.deb"
        package.write_bytes(b"not really a package, but hashed like one")
        repository = directory / "site"
        with (
            patch.object(apt_repository, "_run", _fake_run),
            patch.object(apt_repository, "_sign", _fake_sign),
        ):
            sources = apt_repository.build(
                repository,
                [package],
                None,
                datetime(2026, 9, 22, 10, 0, 0, tzinfo=timezone.utc),
            )
        return repository, sources

    def test_publishes_the_pool_index_and_release(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repository, sources = self._build(Path(raw))
            pool = repository / sources.pool_directory / "keyswitch_1.2.3_amd64.deb"
            index = repository / sources.index_directory / "Packages"
            release = repository / sources.dists_directory / "Release"
            self.assertTrue(pool.is_file())
            payload = pool.read_bytes()
            entry = index.read_text(encoding="utf-8")
            self.assertIn(f"Filename: {sources.pool_directory}/{pool.name}", entry)
            self.assertIn(f"Size: {len(payload)}", entry)
            self.assertIn(f"SHA256: {hashlib.sha256(payload).hexdigest()}", entry)
            self.assertIn(" A description that continues on its own line.\n", entry)
            self.assertIn("Date: Tue, 22 Sep 2026 10:00:00 UTC", release.read_text())

    def test_the_compressed_index_repeats_the_plain_one(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repository, sources = self._build(Path(raw))
            index_directory = repository / sources.index_directory
            plain = (index_directory / "Packages").read_bytes()
            compressed = (index_directory / "Packages.gz").read_bytes()
            self.assertEqual(gzip.decompress(compressed), plain)

    def test_the_release_covers_both_indices(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repository, sources = self._build(Path(raw))
            apt_repository._verify_release_digests(repository, sources)
            index = repository / sources.index_directory / "Packages"
            index.write_bytes(index.read_bytes() + b"Package: intruder\n")
            with self.assertRaises(SystemExit) as raised:
                apt_repository._verify_release_digests(repository, sources)
            self.assertIn("misstates", str(raised.exception))

    def test_a_signature_over_another_release_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repository, sources = self._build(Path(raw))
            inline = (
                repository
                / sources.dists_directory
                / apt_repository.INLINE_SIGNATURE_FILE_NAME
            )
            inline.write_text("-----BEGIN PGP SIGNED MESSAGE-----\n", encoding="utf-8")
            with self.assertRaises(SystemExit) as raised:
                apt_repository._verify_release_digests(repository, sources)
            self.assertIn("inline signature", str(raised.exception))

    def test_publishes_the_key_and_a_page_that_names_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repository, sources = self._build(Path(raw))
            keyring = repository / apt_repository.KEYRING_FILE_NAME
            self.assertEqual(
                keyring.read_bytes(), apt_repository.PUBLIC_KEY_PATH.read_bytes()
            )
            page = (repository / apt_repository.LANDING_FILE_NAME).read_text(
                encoding="utf-8"
            )
            self.assertIn(sources.uri, page)
            self.assertIn(sources.keyring_path, page)
            self.assertIsNone(apt_repository.PLACEHOLDER_PATTERN.search(page))

    def test_refuses_a_package_built_for_another_architecture(self) -> None:
        def wrong_architecture(command: list[str], **overrides: str) -> str:
            if command[-1] == "Architecture":
                return "arm64\n"
            return _fake_run(command, **overrides)

        with tempfile.TemporaryDirectory() as raw:
            package = Path(raw) / "keyswitch_1.2.3_arm64.deb"
            package.write_bytes(b"other architecture")
            with (
                patch.object(apt_repository, "_run", wrong_architecture),
                patch.object(apt_repository, "_sign", _fake_sign),
                self.assertRaises(SystemExit) as raised,
            ):
                apt_repository.build(
                    Path(raw) / "site",
                    [package],
                    None,
                    datetime(2026, 9, 22, 10, 0, 0, tzinfo=timezone.utc),
                )
            self.assertIn("arm64", str(raised.exception))

    def test_refuses_to_publish_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            with self.assertRaises(SystemExit):
                apt_repository.build(
                    Path(raw) / "site",
                    [],
                    None,
                    datetime(2026, 9, 22, 10, 0, 0, tzinfo=timezone.utc),
                )


if __name__ == "__main__":
    unittest.main()

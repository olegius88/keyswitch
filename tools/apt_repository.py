#!/usr/bin/env python3
"""Build and verify the signed APT repository that serves KeySwitch updates.

The repository layout, its suite, component and architecture are not written
here: they are read from ``packaging/debian/keyswitch.sources``, the same file
the Debian package installs into ``/etc/apt/sources.list.d``. A typo there
cannot make the published repository disagree with the installed source,
because both come from that one stanza.

``build`` writes ``dists``/``pool`` and signs the release with the key in the
ambient GnuPG home. ``verify`` proves the result against the public key that
ships inside the package: it checks the signatures with ``gpgv``, checks every
hash the release claims and then makes APT itself read the repository from a
``file:`` URI, which is the same code path an installed system takes.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

PROJECT_DIR = Path(__file__).resolve().parent.parent
SOURCES_STANZA_PATH = PROJECT_DIR / "packaging" / "debian" / "keyswitch.sources"
PUBLIC_KEY_PATH = PROJECT_DIR / "packaging" / "keyswitch-archive-keyring.asc"
LANDING_TEMPLATE_PATH = PROJECT_DIR / "packaging" / "apt-index.html"
PACKAGE_NAME = "keyswitch"
ORIGIN = "KeySwitch"
LABEL = "KeySwitch"
RELEASE_DESCRIPTION = "KeySwitch packages for Ubuntu"
RELEASE_DATE_FORMAT = "%a, %d %b %Y %H:%M:%S UTC"
INDEX_FILE_NAME = "Packages"
RELEASE_FILE_NAME = "Release"
INLINE_SIGNATURE_FILE_NAME = "InRelease"
DETACHED_SIGNATURE_FILE_NAME = "Release.gpg"
KEYRING_FILE_NAME = "keyswitch-archive-keyring.asc"
LANDING_FILE_NAME = "index.html"
PLACEHOLDER_PATTERN = re.compile(r"@[A-Z_]+@")
DIGEST_FIELDS = (("MD5Sum", "md5"), ("SHA256", "sha256"))
COMPUTED_INDEX_FIELDS = ("Filename", "Size", "MD5sum", "SHA1", "SHA256")


@dataclass(frozen=True)
class RepositorySources:
    """The publishing coordinates the installed APT source agrees on."""

    uri: str
    suite: str
    component: str
    architecture: str
    keyring_path: str

    @property
    def dists_directory(self) -> Path:
        return Path("dists") / self.suite

    @property
    def index_directory(self) -> Path:
        return self.dists_directory / self.component / f"binary-{self.architecture}"

    @property
    def pool_directory(self) -> Path:
        return Path("pool") / self.component / PACKAGE_NAME[0] / PACKAGE_NAME


def _stanza_field(stanza: str, name: str) -> str:
    """Return the single value of ``name`` in a deb822 stanza."""

    prefix = f"{name}:"
    for line in stanza.splitlines():
        if line.startswith("#") or not line.startswith(prefix):
            continue
        values = line[len(prefix) :].split()
        if len(values) != 1:
            raise SystemExit(
                f"{SOURCES_STANZA_PATH} must give exactly one {name} value"
            )
        return values[0]
    raise SystemExit(f"{SOURCES_STANZA_PATH} is missing the {name} field")


def read_sources() -> RepositorySources:
    """Read the publishing coordinates from the shipped APT source."""

    stanza = SOURCES_STANZA_PATH.read_text(encoding="utf-8")
    uri = _stanza_field(stanza, "URIs")
    if not uri.startswith("https://") or not uri.endswith("/"):
        raise SystemExit(f"{SOURCES_STANZA_PATH} must serve HTTPS from a directory")
    if _stanza_field(stanza, "Types") != "deb":
        raise SystemExit(f"{SOURCES_STANZA_PATH} must declare a binary repository")
    return RepositorySources(
        uri=uri,
        suite=_stanza_field(stanza, "Suites"),
        component=_stanza_field(stanza, "Components"),
        architecture=_stanza_field(stanza, "Architectures"),
        keyring_path=_stanza_field(stanza, "Signed-By"),
    )


def _run(command: list[str], **overrides: str) -> str:
    """Run a command that must succeed and return its standard output."""

    environment = dict(os.environ)
    environment.update(overrides)
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        env=environment,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stdout)
        sys.stderr.write(completed.stderr)
        raise SystemExit(
            f"Command failed with status {completed.returncode}: {command[0]}"
        )
    return completed.stdout


def _digests(payload: bytes) -> dict[str, str]:
    return {
        "md5": hashlib.md5(payload).hexdigest(),
        "sha1": hashlib.sha1(payload).hexdigest(),
        "sha256": hashlib.sha256(payload).hexdigest(),
    }


def _index_entry(package_path: Path, pool_path: str) -> str:
    """Return the Packages stanza describing one built package."""

    control = _run(["dpkg-deb", "--field", str(package_path)]).rstrip("\n")
    for field in COMPUTED_INDEX_FIELDS:
        for line in control.splitlines():
            if line.startswith(f"{field}:"):
                raise SystemExit(
                    f"{package_path} carries a {field} control field of its own"
                )
    payload = package_path.read_bytes()
    digests = _digests(payload)
    return "\n".join(
        (
            control,
            f"Filename: {pool_path}",
            f"Size: {len(payload)}",
            f"MD5sum: {digests['md5']}",
            f"SHA1: {digests['sha1']}",
            f"SHA256: {digests['sha256']}",
        )
    )


def _release_stanza(
    sources: RepositorySources,
    indices: dict[str, bytes],
    moment: datetime,
) -> bytes:
    """Return the Release file covering every index under its suite."""

    lines = [
        f"Origin: {ORIGIN}",
        f"Label: {LABEL}",
        f"Suite: {sources.suite}",
        f"Codename: {sources.suite}",
        f"Architectures: {sources.architecture}",
        f"Components: {sources.component}",
        f"Date: {moment.strftime(RELEASE_DATE_FORMAT)}",
        f"Description: {RELEASE_DESCRIPTION}",
        "Acquire-By-Hash: no",
    ]
    for field, algorithm in DIGEST_FIELDS:
        lines.append(f"{field}:")
        for name in sorted(indices):
            payload = indices[name]
            digest = _digests(payload)[algorithm]
            lines.append(f" {digest} {len(payload)} {name}")
    return ("\n".join(lines) + "\n").encode("utf-8")


def _landing_page(sources: RepositorySources) -> str:
    """Render the page a person sees when opening the repository URL."""

    page = LANDING_TEMPLATE_PATH.read_text(encoding="utf-8")
    for placeholder, value in (
        ("@URI@", sources.uri),
        ("@SUITE@", sources.suite),
        ("@COMPONENT@", sources.component),
        ("@ARCHITECTURE@", sources.architecture),
        ("@KEYRING_PATH@", sources.keyring_path),
        ("@KEYRING_FILE@", KEYRING_FILE_NAME),
    ):
        page = page.replace(placeholder, value)
    remaining = PLACEHOLDER_PATTERN.search(page)
    if remaining is not None:
        raise SystemExit(f"{LANDING_TEMPLATE_PATH} keeps {remaining.group()} unfilled")
    return page


def _sign(release_path: Path, passphrase_file: Path | None) -> None:
    """Write the inline and detached signatures APT looks for."""

    directory = release_path.parent
    common = ["gpg", "--batch", "--yes", "--armor"]
    if passphrase_file is not None:
        common += [
            "--pinentry-mode",
            "loopback",
            "--passphrase-file",
            str(passphrase_file),
        ]
    _run(
        common
        + [
            "--output",
            str(directory / INLINE_SIGNATURE_FILE_NAME),
            "--clearsign",
            str(release_path),
        ]
    )
    _run(
        common
        + [
            "--output",
            str(directory / DETACHED_SIGNATURE_FILE_NAME),
            "--detach-sign",
            str(release_path),
        ]
    )


def build(
    repository: Path,
    packages: list[Path],
    passphrase_file: Path | None,
    moment: datetime,
) -> RepositorySources:
    """Write and sign the complete repository into ``repository``."""

    sources = read_sources()
    if not packages:
        raise SystemExit("The repository needs at least one package")
    pool = repository / sources.pool_directory
    index_directory = repository / sources.index_directory
    for directory in (pool, index_directory):
        directory.mkdir(parents=True, exist_ok=True)

    entries: list[str] = []
    for package_path in sorted(packages, key=lambda path: path.name):
        architecture = _run(
            ["dpkg-deb", "--field", str(package_path), "Architecture"]
        ).strip()
        if architecture != sources.architecture:
            raise SystemExit(
                f"{package_path} is built for {architecture}, "
                f"not {sources.architecture}"
            )
        if _run(["dpkg-deb", "--field", str(package_path), "Package"]).strip() != (
            PACKAGE_NAME
        ):
            raise SystemExit(f"{package_path} is not a {PACKAGE_NAME} package")
        pool_path = f"{sources.pool_directory}/{package_path.name}"
        shutil.copyfile(package_path, repository / pool_path)
        entries.append(_index_entry(package_path, pool_path))

    index = ("\n\n".join(entries) + "\n").encode("utf-8")
    compressed = gzip.compress(index, compresslevel=9, mtime=0)
    (index_directory / INDEX_FILE_NAME).write_bytes(index)
    (index_directory / f"{INDEX_FILE_NAME}.gz").write_bytes(compressed)

    relative = sources.index_directory.relative_to(sources.dists_directory)
    release = _release_stanza(
        sources,
        {
            f"{relative}/{INDEX_FILE_NAME}": index,
            f"{relative}/{INDEX_FILE_NAME}.gz": compressed,
        },
        moment,
    )
    release_path = repository / sources.dists_directory / RELEASE_FILE_NAME
    release_path.write_bytes(release)
    _sign(release_path, passphrase_file)
    shutil.copyfile(PUBLIC_KEY_PATH, repository / KEYRING_FILE_NAME)
    (repository / LANDING_FILE_NAME).write_text(
        _landing_page(sources), encoding="utf-8"
    )
    return sources


def _verify_signatures(repository: Path, sources: RepositorySources) -> None:
    """Check both signatures against the key the package installs."""

    dists = repository / sources.dists_directory
    with tempfile.TemporaryDirectory(prefix="keyswitch-apt-verify.") as raw_directory:
        keyring = Path(raw_directory) / "trusted.gpg"
        _run(
            [
                "gpg",
                "--batch",
                "--yes",
                "--output",
                str(keyring),
                "--dearmor",
                str(PUBLIC_KEY_PATH),
            ],
            GNUPGHOME=raw_directory,
        )
        for arguments in (
            [str(dists / DETACHED_SIGNATURE_FILE_NAME), str(dists / RELEASE_FILE_NAME)],
            [str(dists / INLINE_SIGNATURE_FILE_NAME)],
        ):
            _run(["gpgv", "--keyring", str(keyring)] + arguments)


def _verify_release_digests(repository: Path, sources: RepositorySources) -> None:
    """Check that every hash the release claims matches the published file."""

    dists = repository / sources.dists_directory
    release = (dists / RELEASE_FILE_NAME).read_text(encoding="utf-8")
    inline = (dists / INLINE_SIGNATURE_FILE_NAME).read_text(encoding="utf-8")
    if release.strip() not in inline:
        raise SystemExit("The inline signature does not cover the release file")
    checked = 0
    algorithm = ""
    for line in release.splitlines():
        for field, name in DIGEST_FIELDS:
            if line == f"{field}:":
                algorithm = name
        if not line.startswith(" "):
            continue
        if not algorithm:
            raise SystemExit("The release file lists a digest before naming it")
        digest, size, name = line.split()
        payload = (dists / name).read_bytes()
        if _digests(payload)[algorithm] != digest or len(payload) != int(size):
            raise SystemExit(f"The release file misstates {name}")
        checked += 1
    if checked != len(DIGEST_FIELDS) * 2:
        raise SystemExit("The release file does not cover both indices")


def _apt_reads(repository: Path, sources: RepositorySources) -> str:
    """Let APT itself read the repository and report the candidate version."""

    with tempfile.TemporaryDirectory(prefix="keyswitch-apt-state.") as raw_directory:
        state = Path(raw_directory)
        for relative in ("lists/partial", "cache/archives/partial", "etc"):
            (state / relative).mkdir(parents=True, exist_ok=True)
        (state / "status").write_text("", encoding="utf-8")
        source_file = state / "etc" / "keyswitch.sources"
        source_file.write_text(
            "\n".join(
                (
                    "Types: deb",
                    f"URIs: file://{repository.resolve()}/",
                    f"Suites: {sources.suite}",
                    f"Components: {sources.component}",
                    f"Architectures: {sources.architecture}",
                    f"Signed-By: {PUBLIC_KEY_PATH}",
                    "",
                )
            ),
            encoding="utf-8",
        )
        options = [
            "-o",
            f"Dir::Etc::sourcelist={source_file}",
            "-o",
            "Dir::Etc::sourceparts=/dev/null",
            "-o",
            "Dir::Etc::trusted=/dev/null",
            "-o",
            "Dir::Etc::trustedparts=/dev/null",
            "-o",
            f"Dir::State::lists={state / 'lists'}",
            "-o",
            f"Dir::State::status={state / 'status'}",
            "-o",
            f"Dir::Cache={state / 'cache'}",
            "-o",
            f"APT::Architecture={sources.architecture}",
            "-o",
            f"APT::Architectures::={sources.architecture}",
            "-o",
            "APT::Get::List-Cleanup=0",
        ]
        _run(["apt-get"] + options + ["update"])
        policy = _run(["apt-cache"] + options + ["policy", PACKAGE_NAME])
    for line in policy.splitlines():
        candidate = line.strip()
        if candidate.startswith("Candidate:"):
            return candidate.split(":", 1)[1].strip()
    raise SystemExit("APT found no candidate version in the published repository")


def verify(repository: Path, expected_version: str) -> None:
    """Prove the published repository the way an installed system reads it."""

    sources = read_sources()
    _verify_signatures(repository, sources)
    _verify_release_digests(repository, sources)
    candidate = _apt_reads(repository, sources)
    if candidate != expected_version:
        raise SystemExit(
            f"APT offers {candidate} where the release publishes {expected_version}"
        )
    print(
        f"APT_REPOSITORY_OK uri={sources.uri} suite={sources.suite} "
        f"component={sources.component} architecture={sources.architecture} "
        f"candidate={candidate}"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    builder = commands.add_parser("build", help="write and sign the repository")
    builder.add_argument("repository", type=Path)
    builder.add_argument("packages", type=Path, nargs="+")
    builder.add_argument("--passphrase-file", type=Path, default=None)

    verifier = commands.add_parser("verify", help="check the published repository")
    verifier.add_argument("repository", type=Path)
    verifier.add_argument("--expect-version", required=True)

    arguments = parser.parse_args(argv)
    if arguments.command == "build":
        sources = build(
            arguments.repository,
            list(arguments.packages),
            arguments.passphrase_file,
            datetime.now(timezone.utc),
        )
        print(
            f"Built {sources.suite}/{sources.component} for "
            f"{sources.architecture} in {arguments.repository}"
        )
        return 0
    verify(arguments.repository, arguments.expect_version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

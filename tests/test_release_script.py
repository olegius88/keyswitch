"""Tests for the one-command release driver."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import release as driver  # noqa: E402
import release_pipeline as pipeline  # noqa: E402


CHANGELOG_TEXT = """# Changelog

## Unreleased

- First change.
- Second change.

## 0.9.0 — 2026-09-03

- Older change.
"""

NOTES_TEXT = """# KeySwitch 0.9.1

- keyswitch_0.9.1_amd64.deb
- KeySwitch-Setup-0.9.1-x64.exe
- KeySwitch-0.9.1-windows-x64.zip
"""


def options(**overrides: object) -> driver.Options:
    base: dict[str, object] = {
        "version": "0.9.1",
        "branch": "main",
        "remote": "origin",
        "profile": "release",
        "message_file": None,
        "skip_pipeline": False,
        "skip_ci": False,
        "dry_run": False,
        "ci_timeout": 60.0,
    }
    base.update(overrides)
    message_file = base["message_file"]
    return driver.Options(
        version=str(base["version"]),
        branch=str(base["branch"]),
        remote=str(base["remote"]),
        profile=str(base["profile"]),
        message_file=message_file if isinstance(message_file, Path) else None,
        skip_pipeline=bool(base["skip_pipeline"]),
        skip_ci=bool(base["skip_ci"]),
        dry_run=bool(base["dry_run"]),
        ci_timeout=float(str(base["ci_timeout"])),
    )


class VersionSiteTests(unittest.TestCase):
    def test_a_site_rewrites_every_occurrence_and_can_be_asked_not_to(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "pyproject.toml"
            path.write_text('name = "x"\nversion = "0.9.1"\n', encoding="utf-8")
            site = driver.VersionSite(
                path, r'^version = "{version}"$', 'version = "{version}"'
            )
            self.assertEqual(site.apply("0.9.1", "0.9.2", write=False), 1)
            self.assertIn('version = "0.9.1"', path.read_text(encoding="utf-8"))
            self.assertEqual(site.apply("0.9.1", "0.9.2"), 1)
            self.assertIn('version = "0.9.2"', path.read_text(encoding="utf-8"))

    def test_a_site_without_the_previous_version_is_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "keyswitch.1"
            path.write_text('.TH KEYSWITCH 1 "KeySwitch 0.1.0"\n', encoding="utf-8")
            site = driver.VersionSite(path, r'"KeySwitch {version}"', '"KeySwitch {version}"')
            with self.assertRaisesRegex(driver.ReleaseError, "does not mention version"):
                site.apply("0.9.1", "0.9.2")

    def test_every_declared_site_exists_in_the_repository(self) -> None:
        sites = driver.version_sites()
        self.assertTrue(sites)
        for site in sites:
            self.assertTrue(site.path.is_file(), site.path)

    def test_the_repository_declares_one_consistent_version(self) -> None:
        driver.verify_version_consistency(pipeline.project_version())
        with self.assertRaisesRegex(driver.ReleaseError, "do not declare 42.0.0"):
            driver.verify_version_consistency("42.0.0")


class ChangelogTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.path = Path(self.temporary.name) / "CHANGELOG.md"
        self.path.write_text(CHANGELOG_TEXT, encoding="utf-8")
        patcher = patch.object(driver, "CHANGELOG", self.path)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_unreleased_entries_move_under_a_dated_heading(self) -> None:
        driver.close_changelog("0.9.1")
        text = self.path.read_text(encoding="utf-8")
        self.assertRegex(text, r"## Unreleased\n\n## 0\.9\.1 — \d{4}-\d{2}-\d{2}\n\n- First")
        # Running again finds the section and leaves the file alone.
        before = text
        driver.close_changelog("0.9.1")
        self.assertEqual(self.path.read_text(encoding="utf-8"), before)

    def test_a_dry_run_reports_without_writing(self) -> None:
        driver.close_changelog("0.9.1", dry_run=True)
        self.assertEqual(self.path.read_text(encoding="utf-8"), CHANGELOG_TEXT)

    def test_an_empty_unreleased_section_stops_the_release(self) -> None:
        self.path.write_text("# Changelog\n\n## Unreleased\n\n## 0.9.0 — x\n", encoding="utf-8")
        with self.assertRaisesRegex(driver.ReleaseError, "neither a section"):
            driver.close_changelog("0.9.1")

    def test_a_changelog_without_the_heading_is_an_error(self) -> None:
        self.path.write_text("# Changelog\n\n## 0.9.0 — x\n\n- old\n", encoding="utf-8")
        with self.assertRaisesRegex(driver.ReleaseError, "neither a section"):
            driver.close_changelog("0.9.1")

    def test_the_commit_message_is_built_from_the_section(self) -> None:
        driver.close_changelog("0.9.1")
        message = driver.build_commit_message(options(), Path("/runs/latest"))
        self.assertTrue(message.startswith("Release KeySwitch 0.9.1\n\n"))
        self.assertIn("- First change.", message)
        self.assertIn("--profile release run", message)

        without_run = driver.build_commit_message(options(), None)
        self.assertIn("run.", without_run)

    def test_a_missing_section_leaves_nothing_to_describe(self) -> None:
        with self.assertRaisesRegex(driver.ReleaseError, "no entries under 1.0.0"):
            driver.build_commit_message(options(version="1.0.0"), None)

    def test_an_explicit_message_file_is_used_verbatim(self) -> None:
        path = Path(self.temporary.name) / "message.txt"
        path.write_text("Release KeySwitch 0.9.1\n\nBody.\n", encoding="utf-8")
        self.assertEqual(
            driver.build_commit_message(options(message_file=path), None),
            "Release KeySwitch 0.9.1\n\nBody.\n",
        )
        missing = Path(self.temporary.name) / "gone.txt"
        with self.assertRaisesRegex(driver.ReleaseError, "commit message file is missing"):
            driver.build_commit_message(options(message_file=missing), None)


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.path = Path(self.temporary.name) / "RELEASE_NOTES.md"
        self.path.write_text(NOTES_TEXT, encoding="utf-8")
        patcher = patch.object(driver, "RELEASE_NOTES", self.path)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_notes_for_this_version_naming_the_packages_pass(self) -> None:
        driver.check_release_notes("0.9.1")

    def test_notes_for_another_version_stop_the_release(self) -> None:
        with self.assertRaisesRegex(driver.ReleaseError, "describes 0.9.1, not 1.0.0"):
            driver.check_release_notes("1.0.0")
        self.path.write_text("Nothing here\n", encoding="utf-8")
        with self.assertRaisesRegex(driver.ReleaseError, "describes nothing"):
            driver.check_release_notes("0.9.1")

    def test_notes_without_the_published_file_names_stop_the_release(self) -> None:
        self.path.write_text("# KeySwitch 0.9.1\n\nNo downloads listed.\n", encoding="utf-8")
        with self.assertRaisesRegex(driver.ReleaseError, "does not name the published files"):
            driver.check_release_notes("0.9.1")


class PipelineResultTests(unittest.TestCase):
    def test_a_missing_or_failing_summary_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            summary = Path(temporary) / "summary.json"
            with self.assertRaisesRegex(driver.ReleaseError, "wrote no summary"):
                driver.pipeline_status(summary)
            self.assertEqual(driver.failed_phase_summary(summary), "")

            summary.write_text(
                json.dumps(
                    {
                        "pipeline": {"status": "failed"},
                        "phases": [
                            {"name": "coverage", "status": "failed"},
                            {"name": "typecheck", "status": "passed"},
                            "not an object",
                        ],
                    }
                ),
                encoding="utf-8",
            )
            self.assertEqual(driver.pipeline_status(summary), "failed")
            self.assertEqual(driver.failed_phase_summary(summary), "; failed phases: coverage")

            summary.write_text(json.dumps({"pipeline": {"status": "passed"}}), encoding="utf-8")
            self.assertEqual(driver.failed_phase_summary(summary), "")
            summary.write_text(json.dumps({"phases": {}}), encoding="utf-8")
            self.assertEqual(driver.failed_phase_summary(summary), "")
            summary.write_text("{", encoding="utf-8")
            self.assertEqual(driver.failed_phase_summary(summary), "")

    def test_a_failing_contour_names_the_summary_and_phases(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            run_directory = Path(temporary) / "run"
            run_directory.mkdir()
            (run_directory / "summary.json").write_text(
                json.dumps(
                    {
                        "pipeline": {"status": "failed"},
                        "phases": [{"name": "e2e-x11", "status": "failed"}],
                    }
                ),
                encoding="utf-8",
            )
            completed = Mock(returncode=3)
            with (
                patch.object(pipeline, "run_identifier", return_value="run"),
                patch.object(pipeline, "DEFAULT_PIPELINE_ROOT", Path(temporary)),
                patch("release.subprocess.run", return_value=completed),
                self.assertRaisesRegex(driver.ReleaseError, "failed phases: e2e-x11"),
            ):
                driver.run_pipeline(options())

    def test_a_passing_contour_returns_its_run_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            run_directory = Path(temporary) / "run"
            run_directory.mkdir()
            (run_directory / "summary.json").write_text(
                json.dumps({"pipeline": {"status": "passed"}, "phases": []}),
                encoding="utf-8",
            )
            with (
                patch.object(pipeline, "run_identifier", return_value="run"),
                patch.object(pipeline, "DEFAULT_PIPELINE_ROOT", Path(temporary)),
                patch("release.subprocess.run", return_value=Mock(returncode=0)),
            ):
                self.assertEqual(driver.run_pipeline(options()), run_directory)

    def test_a_contour_that_ends_unresolved_is_not_a_release(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            run_directory = Path(temporary) / "run"
            run_directory.mkdir()
            (run_directory / "summary.json").write_text(
                json.dumps({"pipeline": {"status": "aborted"}, "phases": []}),
                encoding="utf-8",
            )
            with (
                patch.object(pipeline, "run_identifier", return_value="run"),
                patch.object(pipeline, "DEFAULT_PIPELINE_ROOT", Path(temporary)),
                patch("release.subprocess.run", return_value=Mock(returncode=0)),
                self.assertRaisesRegex(driver.ReleaseError, "reported 'aborted'"),
            ):
                driver.run_pipeline(options())


class GitHubTests(unittest.TestCase):
    def test_malformed_json_from_gh_is_reported(self) -> None:
        with self.assertRaisesRegex(driver.ReleaseError, "did not return valid JSON"):
            driver.load_json_text("{", "gh run list")
        with self.assertRaisesRegex(driver.ReleaseError, "did not return a JSON array"):
            driver.parse_run_list("{}")

    def test_the_release_workflow_run_is_found_by_tag(self) -> None:
        payload = json.dumps(
            [
                {"databaseId": 1, "workflowName": "Tests", "event": "push"},
                {
                    "databaseId": 2,
                    "workflowName": "Build Linux and Windows packages and release",
                    "event": "push",
                },
            ]
        )
        with patch.object(driver, "gh", return_value=payload):
            self.assertEqual(driver.workflow_run_id("v0.9.1"), "2")

    def test_a_run_that_never_appears_stops_the_release(self) -> None:
        payload = json.dumps([{"databaseId": 1, "workflowName": "Tests", "event": "push"}])
        with (
            patch.object(driver, "gh", return_value=payload),
            patch("release.time.monotonic", side_effect=[0.0, driver.CI_APPEARANCE_TIMEOUT]),
            self.assertRaisesRegex(driver.ReleaseError, "no release workflow run appeared"),
        ):
            driver.workflow_run_id("v0.9.1")

    def test_a_failing_workflow_points_at_its_log(self) -> None:
        with (
            patch.object(driver, "workflow_run_id", return_value="7"),
            patch("release.subprocess.run", return_value=Mock(returncode=1)),
            self.assertRaisesRegex(driver.ReleaseError, "gh run view 7 --log-failed"),
        ):
            driver.wait_for_workflow(options(), "v0.9.1")

    def test_a_successful_workflow_is_accepted(self) -> None:
        with (
            patch.object(driver, "workflow_run_id", return_value="7"),
            patch("release.subprocess.run", return_value=Mock(returncode=0)) as run,
        ):
            driver.wait_for_workflow(options(), "v0.9.1")
        self.assertEqual(run.call_args.args[0][:3], ["gh", "run", "watch"])

    def test_the_published_release_must_carry_every_package(self) -> None:
        complete = json.dumps(
            {
                "url": "https://example.invalid/v0.9.1",
                "assets": [
                    {"name": "keyswitch_0.9.1_amd64.deb"},
                    {"name": "KeySwitch-Setup-0.9.1-x64.exe"},
                    {"name": "KeySwitch-0.9.1-windows-x64.zip"},
                    {"name": "SHA256SUMS"},
                ],
            }
        )
        with patch.object(driver, "gh", return_value=complete):
            self.assertEqual(
                driver.verify_published_release("0.9.1", "v0.9.1"),
                "https://example.invalid/v0.9.1",
            )

        partial = json.dumps({"url": "u", "assets": [{"name": "SHA256SUMS"}]})
        with (
            patch.object(driver, "gh", return_value=partial),
            self.assertRaisesRegex(driver.ReleaseError, "missing: keyswitch_0.9.1_amd64.deb"),
        ):
            driver.verify_published_release("0.9.1", "v0.9.1")

        with (
            patch.object(driver, "gh", return_value=json.dumps({"url": "u"})),
            self.assertRaisesRegex(driver.ReleaseError, "lists no assets"),
        ):
            driver.verify_published_release("0.9.1", "v0.9.1")

    def test_a_failing_gh_call_is_reported_with_its_error(self) -> None:
        completed = Mock(returncode=1, stderr="not logged in", stdout="")
        with (
            patch("release.subprocess.run", return_value=completed),
            self.assertRaisesRegex(driver.ReleaseError, "not logged in"),
        ):
            driver.gh("release", "view")

    def test_a_failing_git_call_is_reported_with_its_error(self) -> None:
        completed = Mock(returncode=1, stderr="", stdout="bad revision")
        with (
            patch("release.subprocess.run", return_value=completed),
            self.assertRaisesRegex(driver.ReleaseError, "bad revision"),
        ):
            driver.git("rev-parse", "HEAD")


class CommandLineTests(unittest.TestCase):
    def test_the_version_defaults_to_the_repository_and_must_be_a_triple(self) -> None:
        parsed = driver.parse_arguments([])
        self.assertEqual(parsed.version, pipeline.project_version())
        self.assertEqual((parsed.branch, parsed.remote, parsed.profile), ("main", "origin", "release"))

        chosen = driver.parse_arguments(["--version", "1.2.3", "--skip-ci", "--dry-run"])
        self.assertEqual(chosen.version, "1.2.3")
        self.assertTrue(chosen.skip_ci and chosen.dry_run)

        with self.assertRaisesRegex(driver.ReleaseError, "--ci-timeout must be positive"):
            driver.parse_arguments(["--ci-timeout", "0"])

    def test_a_bad_version_is_refused_before_anything_is_written(self) -> None:
        with self.assertRaisesRegex(driver.ReleaseError, "not MAJOR.MINOR.PATCH"):
            driver.check_preconditions(options(version="0.9"))

    def test_main_turns_a_release_error_into_a_message_and_exit_code(self) -> None:
        with patch.object(driver, "release", side_effect=driver.ReleaseError("boom")):
            self.assertEqual(driver.main([]), 1)
        with patch.object(
            driver, "release", side_effect=pipeline.PhaseFailure("phase")
        ):
            self.assertEqual(driver.main([]), 1)
        with patch.object(
            driver,
            "release",
            side_effect=subprocess.TimeoutExpired(["gh"], 1.0),
        ):
            self.assertEqual(driver.main([]), 1)
        with patch.object(driver, "release", side_effect=KeyboardInterrupt()):
            self.assertEqual(driver.main([]), 130)
        with patch.object(driver, "release") as released:
            self.assertEqual(driver.main(["--dry-run"]), 0)
        released.assert_called_once()

    def test_the_github_cli_is_required_unless_the_wait_is_skipped(self) -> None:
        with (
            patch.object(driver, "git", return_value="main"),
            patch.object(driver, "command_exists", return_value=False),
            self.assertRaisesRegex(driver.ReleaseError, "GitHub CLI"),
        ):
            driver.check_preconditions(options())

    def test_releasing_from_another_branch_needs_saying_so(self) -> None:
        with (
            patch.object(driver, "git", return_value="feature"),
            self.assertRaisesRegex(driver.ReleaseError, "not 'main'"),
        ):
            driver.check_preconditions(options())

    def test_command_exists_answers_for_a_real_and_a_missing_tool(self) -> None:
        self.assertTrue(driver.command_exists("git"))
        self.assertFalse(driver.command_exists("keyswitch-nonexistent-tool"))


if __name__ == "__main__":
    unittest.main()

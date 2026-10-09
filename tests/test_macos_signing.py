"""The macOS build signs again only while Apple's timestamp service does not answer."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SIGNING = ROOT / "packaging/macos-signing.sh"
TIMESTAMP_FAILURE = "keyswitch-bin: A timestamp was expected but was not found."


def run_signing(script: str, attempts: int) -> tuple[subprocess.CompletedProcess[str], int]:
    """Runs `script` through run_signing; returns the result and how many times the script ran. The script
    finds the number of runs so far, its own included, in the file $COUNTER."""
    with tempfile.TemporaryDirectory() as directory:
        counter = Path(directory) / "runs"
        counter.write_text("", encoding="utf-8")
        command = f'set -euo pipefail; source "{SIGNING}"; run_signing bash -c \'echo run >> "$COUNTER"; {script}\''
        environment = {**os.environ, "COUNTER": str(counter), "KEYSWITCH_SIGN_ATTEMPTS": str(attempts),
                       "KEYSWITCH_SIGN_RETRY_SECONDS": "0"}
        result = subprocess.run(["bash", "-c", command], capture_output=True, text=True, env=environment, check=False)
        return result, len(counter.read_text(encoding="utf-8").splitlines())


class MacosSigningTests(unittest.TestCase):
    def test_a_missing_timestamp_is_signed_again_until_it_succeeds(self) -> None:
        # Fails for want of a timestamp on the first run only.
        result, runs = run_signing(
            'if [ "$(grep -c run "$COUNTER")" -eq 1 ]; then echo "' + TIMESTAMP_FAILURE + '"; exit 1; fi; echo signed',
            len("abc"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(runs, len("ab"))
        self.assertIn("signed", result.stdout)
        self.assertIn(TIMESTAMP_FAILURE, result.stdout)
        self.assertIn("timestamp service did not answer (attempt 1 of", result.stderr)

    def test_a_timestamp_that_never_comes_stops_after_the_last_attempt(self) -> None:
        attempts = len("ab")
        result, runs = run_signing(f'echo "{TIMESTAMP_FAILURE}"; exit 1', attempts)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(runs, attempts)

    def test_any_other_failure_stops_at_once(self) -> None:
        result, runs = run_signing('echo "errSecInternalComponent"; exit 1', len("abc"))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(runs, 1)
        self.assertNotIn("timestamp service did not answer", result.stderr)

    def test_the_build_signs_through_it_both_in_nuitka_and_when_it_signs_the_bundle_again(self) -> None:
        build = (ROOT / "packaging/build-macos.sh").read_text(encoding="utf-8")
        self.assertIn('source "$project_dir/packaging/macos-signing.sh"', build)
        self.assertIn('run_signing env PYTHONPATH="${nuitka_path:+$nuitka_path:}$project_dir/src" \\\n"$python_bin" -m nuitka', build)
        self.assertIn("run_signing codesign --force --deep --options runtime --timestamp", build)


if __name__ == "__main__":
    unittest.main()

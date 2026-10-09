"""The model evidence replays run side by side, each in a leg of its own, in the test and release workflows."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPLAYS = ("context", "ortho", "boundary")


def replays_job(workflow: str) -> str:
    text = (ROOT / ".github/workflows" / workflow).read_text(encoding="utf-8")
    return text[text.index("\n  replays:\n"):text.index("\n  prefix-replay:\n")]


class WorkflowReplayTests(unittest.TestCase):
    def test_each_replay_runs_only_in_its_own_leg_of_the_matrix(self) -> None:
        for workflow in ("tests.yml", "release.yml"):
            job = replays_job(workflow)
            with self.subTest(workflow=workflow):
                self.assertIn("        replay: [" + ", ".join(REPLAYS) + "]", job)
                self.assertIn("name: Model evidence replay (${{ matrix.replay }})", job)
                starts = [job.index(f"      - name: Look up a verified {name} replay of this exact tree") for name in REPLAYS]
                # Before the first replay: the checkout, the tools and the fingerprint every leg needs.
                self.assertNotIn("matrix.replay ==", job[:starts[0]])
                for name, start, end in zip(REPLAYS, starts, [*starts[1:], len(job)], strict=True):
                    steps = re.split(r"\n      - name: ", job[start:end])
                    for step in steps:
                        condition = re.search(r"\n        if: (.*)", "\n" + step)
                        self.assertIsNotNone(condition, step.splitlines()[0])
                        assert condition is not None
                        self.assertTrue(condition.group(1).startswith("${{ matrix.replay == '" + name + "' && "),
                                        step.splitlines()[0])


if __name__ == "__main__":
    unittest.main()

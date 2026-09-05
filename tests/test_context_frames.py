"""Split and physical-token integrity of model-blind action frames."""
from __future__ import annotations

import unittest
import sys
from pathlib import Path

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

from context_corpus import AssignedPhrase, Phrase
from context_frames import build, word_spans


class FrameTests(unittest.TestCase):
    def test_no_partial_unicode_or_identifier_supervision(self) -> None:
        for text in ("über", "3D", "foo_bar", "someone@example.org", "C:\\Windows", "ещё́", "по‑русски"):
            self.assertEqual(word_spans(text, 0), [], text)
            self.assertEqual(word_spans(text, 1), [], text)
        self.assertEqual([m.group() for m in word_spans("We don’t mind open-source.", 0)], ["We", "don’t", "mind", "open-source"])

    def test_reserve_is_not_expanded_and_interventions_preserve_exact_alternatives(self) -> None:
        reserved = AssignedPhrase(Phrase(1, "rus", "У этого есть смысл.", ""), "first", "reserve")
        self.assertEqual(build([reserved]), [])
        source = AssignedPhrase(Phrase(2, "rus", "У этого есть смысл.", ""), "second", "train")
        rows = build([source])
        self.assertTrue(rows)
        for correct, wrong in zip(rows[::2], rows[1::2]):
            self.assertEqual(correct.action, "keep")
            self.assertEqual(correct.original, wrong.alternative)
            self.assertEqual(correct.alternative, wrong.original)
            self.assertEqual(correct.group, 1 - wrong.group)
            self.assertEqual(correct.cluster, wrong.cluster)
            self.assertEqual(correct.before, wrong.before)
            self.assertEqual(correct.after, wrong.after)


if __name__ == "__main__":
    unittest.main()

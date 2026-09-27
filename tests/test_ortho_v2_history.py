"""The rejected ortho-v2 evidence stays verifiable against archived sources and never ships."""
from __future__ import annotations

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import ortho_v2_corpus as corpus  # noqa: E402
import ortho_v2_known as known  # noqa: E402
import ortho_v2_verified as verified  # noqa: E402
import train_ortho_v2 as trainer  # noqa: E402
import verify_ortho_v2 as verifier  # noqa: E402
from historical_sources import ORTHO_V2  # noqa: E402
from keyswitch.constants.file_formats import SHA256_HEX_CHARACTERS  # noqa: E402

CHANGED_DIGEST = "0" * SHA256_HEX_CHARACTERS


def recorded(path: Path) -> dict[str, object]:
    payload: object = json.loads(path.read_bytes())
    assert isinstance(payload, dict)
    return payload


class OrthoV2HistoryTests(unittest.TestCase):
    def test_historical_evidence_verifies_without_claiming_the_current_runtime(self) -> None:
        result = verifier.verify()
        self.assertIs(result["historical_evidence_verified"], True)
        self.assertIs(result["current_runtime_verified"], False)
        self.assertIs(result["promoted"], False)
        self.assertIsNone(result["installed_artifact"])
        self.assertEqual(result["scope"], verifier.SCOPE)

    def test_anchors_fail_closed(self) -> None:
        checksum = trainer.checksum
        for name in verifier.ANCHORS:
            target = trainer.DIRECTORY / name

            def altered(path: Path, target: Path = target) -> str:
                return CHANGED_DIGEST if path == target else checksum(path)

            with self.subTest(anchor=name), patch.object(verifier, "checksum", side_effect=altered):
                with self.assertRaisesRegex(ValueError, "anchor changed"):
                    verifier.verify()

    def test_seal_and_receipt_pins_resolve_to_archived_copies_only(self) -> None:
        seal = recorded(trainer.SEAL)
        pins = seal["provenance"]
        assert isinstance(pins, dict)
        self.assertEqual(trainer.recorded_provenance(seal), pins)
        # The shared runtime module and the installed model's files are archived, not read live.
        for shared in ("src/keyswitch/ortho_model.py", "model/ortho_v1/candidate.json", "model/ortho_v1/tokens.jsonl.gz"):
            self.assertIn(shared, ORTHO_V2.sources)
        for change in ({**pins, "src/keyswitch/ortho_model.py": CHANGED_DIGEST},
                       {name: digest for name, digest in pins.items() if name != "model/ortho_v1/candidate.json"}):
            with self.subTest(pins=sorted(change)), self.assertRaises(ValueError):
                trainer.recorded_provenance({**seal, "provenance": change})
        receipt = recorded(corpus.RECEIPT)
        sources = receipt["provenance"]
        assert isinstance(sources, dict)
        self.assertEqual(corpus.recorded_provenance(receipt), sources)
        with self.assertRaisesRegex(ValueError, "unapproved"):
            corpus.recorded_provenance({**receipt, "provenance": {**sources, "tools/context_corpus.py": CHANGED_DIGEST}})
        read = verifier.read_object

        def altered_seal(path: Path) -> dict[str, object]:
            return {**seal, "provenance": {**pins, "tools/train_ortho_v2.py": CHANGED_DIGEST}} if path == trainer.SEAL else read(path)

        with patch.object(verifier, "read_object", side_effect=altered_seal), self.assertRaisesRegex(ValueError, "unapproved"):
            verifier.verify()

    def test_replayed_receipts_keep_the_recorded_tool_digests(self) -> None:
        known_receipt, labels_receipt = recorded(known.RECEIPT), recorded(verified.RECEIPT)
        self.assertEqual(known.generator_digest(), known_receipt["generator_sha256"])
        self.assertEqual(verified.tool_digests(), (labels_receipt["generator_sha256"], labels_receipt["corpus_tool_sha256"]))
        with tempfile.TemporaryDirectory(prefix="keyswitch-ortho-v2-receipt-") as temporary:
            directory = Path(temporary)
            # A receipt about to be frozen records the live tools.
            self.assertEqual(known.generator_digest(directory / "missing.json"), known.checksum(Path(known.__file__)))
            self.assertEqual(verified.tool_digests(directory / "missing.json"),
                             (verified.checksum(Path(verified.__file__)), verified.checksum(ROOT / verified.CORPUS_TOOL)))
            tampered = directory / "receipt.json"
            tampered.write_text(json.dumps({**known_receipt, "generator_sha256": CHANGED_DIGEST}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unapproved"):
                known.generator_digest(tampered)
            tampered.write_text(json.dumps({**labels_receipt, "corpus_tool_sha256": None}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "invalid historical source pin"):
                verified.tool_digests(tampered)
            tampered.write_text("[]", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "invalid historical source pin"):
                known.generator_digest(tampered)

    def test_an_installed_candidate_fails(self) -> None:
        with tempfile.TemporaryDirectory(prefix="keyswitch-ortho-v2-installed-") as temporary:
            installed = Path(temporary) / "ortho_v2.json"
            shutil.copyfile(trainer.CANDIDATE, installed)
            with patch.object(verifier, "ARTIFACT", installed), self.assertRaisesRegex(ValueError, "installed"):
                verifier.verify()


if __name__ == "__main__":
    unittest.main()

"""The CUDA back end of the context-v3 trainer computes the features the trainer's Python computes.

Runs only where CuPy and a CUDA device are present; the cpu back end needs neither. On a corpus,
`tools/context_action_cuda.py --corpus DIR` runs the same comparison over many more frames.
"""

from __future__ import annotations

from pathlib import Path
import sys
from typing import cast
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
# A plain flag, not a platform check mypy evaluates: on other platforms it would call the rest unreachable.
ON_LINUX = sys.platform.startswith("linux")
if not ON_LINUX:
    raise unittest.SkipTest("the context-v3 trainer runs on Linux: it forks its workers and needs NumPy")

import context_action_pipeline as pipeline
import train_context_action_model as trainer
from freeze_context_action_corpus import CorpusRow
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN
from keyswitch.constants.models import FNV1A64_OFFSET_BASIS
from keyswitch.constants.training import (
    CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES,
    CONTEXT_ACTION_CUDA_RECORD_FLAGS,
    CONTEXT_ACTION_NAME_HASH_SEED,
)
from keyswitch.input_context import FieldContext
from keyswitch.intent_model import LinearNgramModel
from keyswitch.ortho_model import OrthoModel
from reference_lexicon import reference_models


def cuda_available() -> bool:
    try:
        import cupy

        return int(cupy.cuda.runtime.getDeviceCount()) > 0
    except (ImportError, RuntimeError):  # CuPy missing, or no driver: the test does not apply here
        return False


# Words of both languages typed right and in the wrong layout, short and long, with and without
# neighbours, punctuation, capitals, digits and an apostrophe.
WORDS = (("привет", 1), ("ghbdtn", 0), ("hello", 0), ("руддщ", 1), ("он", 1), ("jy", 0), ("Москва", 1), ("Vjcrdf", 0),
         ("don't", 0), ("тест123", 1), ("API", 0), ("ФЗШ", 1), ("обсуждение", 1), ("j,ce;ltybt", 0), ("the", 0), ("еру", 1))
CONTEXTS = (("", ""), ("мы обсуждаем ", " потом"), ("we discussed ", "!  later"), ("git ", ""))


def corpus_row(number: int, original: str, group: int, before: str, after: str) -> CorpusRow:
    identifier = f"cuda-fixture:{number}"
    return CorpusRow(
        identifier=identifier, original=original, group=group, before=before, after=after, lemma=original,
        family=identifier, document="doc:" + identifier, language="ru" if group == 1 else "en", source="authored-fixture",
        source_file="fixture.conllu", source_sentence=identifier, source_token="1", spacing="", space_before=" ",
        literal_tail="", upos="NOUN", features="_", misc="_", alignment="fixture", layout_representable=True,
        split="development",
    )


@unittest.skipUnless(cuda_available(), "CuPy and a CUDA device are required")
class CudaParityTest(unittest.TestCase):
    def test_every_frame_kind_gets_the_python_features(self) -> None:
        import context_action_cuda

        rows = [corpus_row(number, original, group, before, after)
                for number, ((original, group), (before, after)) in enumerate((word, context) for word in WORDS for context in CONTEXTS)]
        frames = trainer.action_rows(rows)
        # A frame with a planned next word, as the lookahead curricula make them.
        frames.append(trainer.ActionRow("planned", "jy", 0, FieldContext("Telegram", "fixture", "", "привет", "text"), "space", "",
                                        "convert", "fixture", " ", 1.0, "planned_next_conversion"))
        options = trainer.recipe()
        inputs = trainer.FitInputs(options, {split: [] for split in (TRAIN, DEVELOPMENT)}, {TRAIN: frames, DEVELOPMENT: frames},
                                   LinearNgramModel.load(trainer.ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm"),
                                   OrthoModel.load(trainer.ROOT / "src/keyswitch/resources/models/ortho_v1.json"),
                                   {spelling: reference_models(spelling) for spelling in (False, True)},
                                   cast(list[str], options["profiles"]), frozenset())
        with pipeline.bound_process_state(inputs):
            result = context_action_cuda.check_parity({TRAIN: frames, DEVELOPMENT: frames}, inputs)
        self.assertEqual(result.mismatches, [])
        self.assertEqual(result.frames, len(frames) * len(inputs.profiles) * len((TRAIN, DEVELOPMENT)))

    def test_the_generated_header_names_every_bit_and_count(self) -> None:
        import context_action_cuda

        header = context_action_cuda.constants_header()
        for name in CONTEXT_ACTION_CUDA_CHARACTER_PROPERTIES:
            self.assertIn(f"#define P_{name.upper()} ", header)
        for name in CONTEXT_ACTION_CUDA_RECORD_FLAGS:
            self.assertIn(f"#define RF_{name.upper()} ", header)
        self.assertIn("static __device__ const long long KS_INTENT_NGRAM_ORDERS[]", header)
        self.assertEqual(context_action_cuda.name_hashes(""), (FNV1A64_OFFSET_BASIS, CONTEXT_ACTION_NAME_HASH_SEED))


if __name__ == "__main__":
    unittest.main()

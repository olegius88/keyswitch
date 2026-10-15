"""The heads keep the latest epoch whose development balance is within EPOCH_NET_BENEFIT_TOLERANCE of the best
(train_context_action_model.settled_epoch): a difference of a row or two on development is noise."""

from __future__ import annotations

import sys
import unittest
from array import array
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from action_epoch_selection import EpochSelection  # noqa: E402
from keyswitch.constants.training import EPOCH_NET_BENEFIT_TOLERANCE  # noqa: E402
import train_context_action_model as trainer  # noqa: E402


def selection(minimum: int, net: int) -> EpochSelection:
    return EpochSelection(threshold=1.0, minimum_recall=1.0, pooled_recall=1.0, loss=1.0, by_profile={},
                          net_benefit=net, minimum_net_benefit=minimum)


class EpochSettlingTests(unittest.TestCase):
    def test_the_latest_epoch_within_the_tolerance_of_the_best_is_kept(self) -> None:
        best = selection(minimum=EPOCH_NET_BENEFIT_TOLERANCE * len("abcd"), net=EPOCH_NET_BENEFIT_TOLERANCE * len("abcdefgh"))
        near = selection(minimum=best.minimum_net_benefit - EPOCH_NET_BENEFIT_TOLERANCE, net=best.net_benefit - 1)
        far = selection(minimum=best.minimum_net_benefit - EPOCH_NET_BENEFIT_TOLERANCE - 1, net=best.net_benefit)
        # Epochs one to four: near the best, the best, near it again, then one too far below it.
        ranked = (near, best, near, far)
        epochs = [(number, kept, array("d", [float(number)]), 1.0) for number, kept in enumerate(ranked, 1)]
        epoch, chosen, weights, _loss = trainer.settled_epoch(epochs)
        self.assertEqual((epoch, chosen, list(weights)), (len("abc"), near, [float(len("abc"))]))
        # Alone, the best epoch is kept; of exact ties the latest.
        self.assertEqual(trainer.settled_epoch(epochs[1:len("ab")])[0], len("ab"))
        ties = [(number, best, array("d"), 1.0) for number in range(1, len("abc"))]
        self.assertEqual(trainer.settled_epoch(ties)[0], len("ab"))


if __name__ == "__main__":
    unittest.main()

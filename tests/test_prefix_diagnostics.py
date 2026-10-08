"""A support report names all three models: the intent model, the context model and the prefix model.

The diagnostics named the first two only. The owner read `intent-v1-...` next to a context model of
feature version 3 and asked whether the version was wrong (08.10.2026): the model that switches the
layout while a word is typed was not in the report at all.
"""

from __future__ import annotations

import unittest
from unittest.mock import patch

from keyswitch.prefix_schema import VersionedPrefixModel, prefix_diagnostics


class PrefixDiagnosticsTests(unittest.TestCase):
    def test_the_report_names_the_installed_prefix_model_or_its_absence(self) -> None:
        model = VersionedPrefixModel.default()
        assert model is not None
        self.assertEqual(prefix_diagnostics(), {"available": True, "status": model.version})
        self.assertTrue(model.version.startswith("prefix-v"))
        with patch.object(VersionedPrefixModel, "default", return_value=None):
            self.assertEqual(prefix_diagnostics(), {"available": False, "status": ""})


if __name__ == "__main__":
    unittest.main()

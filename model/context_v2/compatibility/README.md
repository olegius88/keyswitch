# Historical context-v2 source identity

`generation-sources/` contains exact bytes from Git commit `80534ec` for the
three source files referenced by the original candidate seal or engine report
that changed during context-v3 development. Their SHA-256 values are checked
against reviewed constants in `tools/verify_context_v2_history.py`.

The original rejected candidate, seal, numeric report, engine report and
history remain unchanged. All other recorded source/data pins still resolve
to the original files. The archived Python files are read as evidence; they
are not imported as application code.

`tools/verify_context_v2.py` verifies these historical anchors and rejects
installing the research candidate. It explicitly reports
`current_runtime_verified: false`; acceptance of the installed model belongs
to `tools/verify_context_model.py`.

`--verify-frozen` reconstructs the original four numeric evaluation tracks
from the already observed public phrase data and frozen lexical evidence,
using unchanged weights. It compares the complete canonical report with the
original bytes and preserves the rejected result. It does not train, write
new reports, open the prospective context-v3 test, or replay the current
engine. Before replay, a conservative AST check verifies that the current
feature-2 normalization, extractor, softmax, constants and prediction path remain compatible
with the archived source. Only branches explicitly guarded by
`self.feature_version == 3` are omitted from that comparison.

The version-dispatch additions in `ContextModel.__init__` and `load` have
separate reviewed AST hashes: feature-2 weights, decoding and thresholds
cannot change silently. The hashes omit source coordinates and empty optional
AST fields to avoid treating Python AST metadata changes as numerical changes.
This is a scoped source compatibility check, not a cryptographic proof of
the Python interpreter, imported libraries or a hostile source tree.

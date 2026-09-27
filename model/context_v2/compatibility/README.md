# Historical context-v2 source identity

The candidate seal, the lexical receipt, the corpus receipt and the engine
report of this rejected experiment pin, by SHA-256, the source files that
produced them. Those outside `model/context_v2/` keep changing: the runtime
moved its numbers into named constants, the intent model was retrained, and
the installed ortho-v1 and boundary-v2 models will be retrained. So
`generation-sources/` keeps an exact copy of every such file, stored under its
repository path: `src/keyswitch/context_model.py`, `context_policy.py` and
`engine.py` from Git commit `80534ec`, all others from Git commit `2c090c0`,
each equal to its pin. The reviewed digests are declared in
`tools/historical_sources.py`; a pin is checked against its archived copy and
never against the live file, and a pin outside `model/context_v2/` without an
archived copy fails the check.

The original rejected candidate, seal, numeric report, engine report and
history remain unchanged. The archived files are evidence: they are never
imported as application code. The feature-2 check below executes the archived
`context_model.py` as a private, separate module to use its behaviour as the
reference. The archived intent model (`layout_intent_v1.ksm` and
`model/intent_v1/config.json`) is the one the frozen lexical cache was
computed with; nothing claims that the installed intent model is equivalent
to it.

`tools/verify_context_v2.py` verifies these historical anchors and rejects
installing the research candidate. It explicitly reports
`current_runtime_verified: false`; acceptance of the installed model belongs
to `tools/verify_context_model.py`.

`--verify-frozen` reconstructs the original four numeric evaluation tracks
from the already observed public phrase data and frozen lexical evidence,
using unchanged weights. It rebuilds the phrase frames with the current
corpus tools and compares the complete canonical report with the original
bytes, so it also shows that those tools still produce the same frames and
metrics; it preserves the rejected result. It does not train, write new
reports, open the prospective context-v3 test, or replay the current engine.
`tools/train_context_v2.py verify` additionally refits the candidate and
compares candidate, seal (with its recorded pins) and report byte for byte.

## Feature-2 behavioural equivalence

Before replay, the current `src/keyswitch/context_model.py` must reproduce the
archived feature-2 numerics exactly. Source text is not compared: moving
numbers into named constants, reformatting and feature-3-only changes pass;
any numerical difference fails. Both files are executed side by side as
private modules (the archive's only sibling import, `FieldContext`, comes from
the live `input_context.py`; the engine report pins its archived bytes, and
the live class has the same fields) and compared on:

- the values and types of `ACTIONS`, `FEATURE_VERSION`, `MAX_ARTIFACT_BYTES`,
  `MAX_FEATURES` and the fields of `ContextPrediction`;
- `softmax` on score vectors built from every number of the archived
  extractor and loader, their signs, neighbouring floats and IEEE extremes;
- `extract_context_features` on the whole evidence corpus: the words of
  `feature-two-probes.json` in both layouts over rotating fields,
  applications, roles, triggers and flags, plus one probe below, at and above
  every archived text window, word count, application-part count and
  score clipping bound, and Unicode normalization probes. The live evidence
  also varies its serving-only fields, which feature 2 must ignore. Feature
  dictionaries are compared with their order, the order in which scores are
  summed;
- `ContextModel.load` on valid and invalid artifacts at every archived loader
  bound (name length, value count, weight magnitude, threshold, version
  length, feature count, file size), wrong feature versions, actions,
  checksums and a tampered historical baseline. A payload that does not
  declare feature schema 2 may be rejected or loaded as another schema by
  the current loader, never loaded as a feature-2 model the archive did not
  produce identically;
- `ContextModel.predict` of an empty model, of both historical artifacts and
  of the installed artifact while it declares feature schema 2, on the whole
  corpus (action, every probability as an exact float, support flag), plus
  thresholds equal to an observed conversion probability and the next float.

Every number of the probe plan comes from the archived source.
`feature-two-probes.json` is its frozen seed: the development word lists of
`model/context_v1/scenarios.json` at release 0.32.0 and authored contexts.
The canonical plan is pinned by SHA-256 in the verifier, so a changed or
shortened plan fails closed. This is a behavioural check on these probes, not
a proof for all inputs, nor for the Python interpreter, imported libraries or
a hostile source tree.

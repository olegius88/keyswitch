# Public source snapshot

`tatoeba-cc0-en-ru.tsv.gz` contains the English (`eng`) and Russian (`rus`)
rows of Tatoeba's **CC0** export, downloaded on 2026-09-05 from the export
dated 2026-08-29. This is not Tatoeba's default CC-BY sentence export.

- Source: <https://downloads.tatoeba.org/exports/sentences_CC0.tar.bz2>
- License declaration and format: <https://tatoeba.org/en/downloads>
- CC0 contribution policy: <https://en.wiki.tatoeba.org/articles/show/cc0-contributions>
- CC0 1.0: <https://creativecommons.org/publicdomain/zero/1.0/>
- Original archive SHA-256:
  `0b4b58b84239c4c194096040258deececa4f6a1ac3b93da16b64ae477b70f98e`.
- Filtered gzip SHA-256:
  `1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793`.

The four tab-separated columns are source ID, language, literal sentence,
last modification timestamp. Original IDs, spelling, case, spacing and
punctuation are retained; only language filtering and numeric ID sorting
are performed for this snapshot. The gzip timestamp is zero. The checked-in
snapshot, not a mutable latest download, is used for replay. No private
KeySwitch log, field content, personal word or user message is included.

`lexical-evidence.json.gz` is generated numeric evidence for these public
tokens and the project-authored safety curriculum. The dictionary-present
profile uses the exact Hunspell `.dic` / `.aff` hashes in
`model/intent_v1/config.json`; the portable profile disables Hunspell. Both
use the frozen Onboard frequency lexicons and the certified Layout Intent
artifact. Their license/provenance evidence is retained in
`model/intent_v1/sources/COPYRIGHT.onboard-data` and its existing manifest.
This generated cache follows the project's GPL-3.0-or-later license. It is
not a new unrestricted license grant for the upstream lexical resources.

The numeric cache is a fixed training input so replay does not silently
use user/system dictionaries from the replay host. It does not establish
equivalence of every Hunspell implementation or custom dictionary.

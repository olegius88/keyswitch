# Changelog

All notable changes to KeySwitch are documented in this file.

## Unreleased

## 0.31.1 — 2026-09-24

- Let a word in doubt wait for its neighbour as long as the text around it stays
  context. The wait lapsed after ten seconds, so "tot", a pause to think and "ghbdtn"
  twelve seconds later gave "tot привет": "привет" was converted, but "tot" was no
  longer waiting to be decided with it. A wait now lasts the 45 seconds the engine
  keeps the neighbouring text as context, and still ends at once on another window, a
  caret move, Backspace or a changed field. Nothing else changes; the models are the
  same as in 0.31.0.
- Give every number in the code, the tests and the tools a name. A number written in
  place hides what it means and lets two places that must agree drift apart without
  anything failing: the ten-second wait above lived next to a forty-five-second context
  as two unrelated literals. Every number other than 0 and 1 is now a named constant,
  declared once in the module it belongs to and imported by the others; the defaults of
  the settings, for instance, are named in `config.py` and used both by the settings
  and by every fallback. `tools/check_named_values.py`, run by the test suite, fails on
  a new unnamed number. The files whose bytes are pinned by the seals of the shipped
  models follow when those models are trained and sealed again; until then they are
  listed in the checker. Behaviour does not change.

## 0.31.0 — 2026-09-23

- Decide a word in doubt together with the word after it. "еще" typed in the English
  layout at the start of a message is "tot", an English word too, so the context model
  asks to wait for the next word; the engine only waited for words of one or two
  letters, and "tot" stayed even when "тест" after it was converted. A word the model
  asks to wait for now waits whatever its length. A word it only suggests converting
  waits for its neighbour as well, and is asked again when the neighbour arrives but
  never at a pause, which brings no new context. When the next word is not converted,
  it is asked once more after the waiting word's other reading: "d" after "tot" is a
  lone letter, after "еще" it is "в". The pair converts only if the model converts both
  words, so "tot d " becomes "еще в " and "tot is " stays.
- Retrain the completed-word context model on scenarios where such a word is followed
  by a converted Russian word, each paired with the same word followed by nothing, so
  that only the next word teaches the decision. The new model is
  `context-v1-a5859ddfb70e`, artifact SHA-256
  `bf894c17fd33d8d0c1dc9baee5b5e9ba32e010f4512883e1a8c0f9fa9b98d4b7`: 0 false
  conversions on the 51,575 holdout rows and 22,999 desired conversions against 19,714
  by the detector alone. On 500 new sentences from film subtitles (300 Russian, 200
  English), each typed through the program in five kinds of fields, this release
  restored 2,304 sentences typed in the wrong layout where 0.30.0 restored 2,216, and
  spoiled 24 correctly typed sentences out of 2,500 where 0.30.0 spoiled 26; no
  sentence 0.30.0 got right came out wrong. Most of the gain is sentences that open
  with a short word typed in the wrong layout ("ш" for "i", "еру" for "the", "ye" for
  "ну"). After code or mostly English text "tot" still stays "tot": there the model
  takes it for the English word.

## 0.30.0 — 2026-09-23

- Keep Russian words typed after English text or code. The completed-word context
  model sees the script that dominates the text in front of the caret, and it had
  learned that a Russian word after mostly English text is a layout error: in a chat
  under an editor or in a code comment "еще" became "tot" and "ты" became "ns", even
  right after "все" or "почему". The scenarios now add conversational Russian words,
  including the everyday spelling "еще", the short English words they lacked
  entirely, and fields holding code, logs and English prose in which correctly typed
  Russian and English words stay and either one typed in the other layout is
  converted, in equal measure, so the script of the field no longer decides on its
  own. The training takes the language of the previous word from the last word in
  front of the caret, as the engine does, rather than from the whole field.
  The new model is `context-v1-79354d57e0b7`, artifact SHA-256
  `e95ae15b408192a8b93297790c665fc32bfcbec9b17a0ad37065a2d538c824c6`: 0 false
  conversions on the 50,495 holdout rows and 22,477 desired conversions against
  19,714 by the detector alone. On 500 sentences from film subtitles that no model
  had seen (300 Russian, 200 English), each typed through the engine in five kinds
  of fields, it spoiled 8 correctly typed sentences out of 2,500 where the previous
  model spoiled 23 (11 of those were "еще" turned into "tot"), and restored 2,181
  sentences typed in the wrong layout against 2,172. One case got worse: "мы" typed
  in the English layout as the first word right after English text or code now
  stays "vs" (24 of the 1,500 Russian sentences); Pause converts it.

## 0.29.0 — 2026-09-23

- Let letters added to a finished word belong to it. Deleting the space after a word
  and typing on used to start a new word from the added letters alone, so "создаш",
  a Backspace and "ь" were judged as the lone "ь", which no dictionary knows and which
  the models happily turned into "m". One Backspace over the boundary now reopens the
  word it ended: the added letters are judged with it, a further Backspace keeps
  erasing it, and Pause converts it as the current word. Deleting the space alone does
  not get the word judged a second time; only a new letter does. Nothing is reopened
  after a caret move, a second boundary, literal punctuation before the space, or Enter.

- Leave a word alone when a correction is abandoned under it. A correction waits for
  its boundary key to come up, and by then the next word has often begun; when the
  field then turned out to have changed, the letters already typed were discarded and
  the rest of that word was judged on its own, so "все" could end up as "вct". The rest
  of such a word is now left untouched until its own boundary.

- Clear learned rules without losing the rejections. Clearing local learning used to
  remove the rejections recorded when a false correction was undone along with the
  rules, so every cleared list brought the rejected corrections back. Rules and
  rejections are now cleared separately in the settings of every platform, and the
  Windows and macOS window can also delete a single entry from its list.

## 0.28.0 — 2026-09-22

- Keep Ubuntu up to date through APT. A new Linux version used to mean opening the
  release page, downloading the `.deb` and installing it by hand; the notification the
  application showed was as far as it went. The package now installs its own APT source
  and the public key that signs it, so Software Updater and `sudo apt upgrade` offer
  KeySwitch alongside every other package. Each release publishes the signed repository
  at <https://olegius88.github.io/keyswitch/>, and APT trusts that key for this
  repository alone rather than system-wide. The source is an ordinary conffile: adding
  `Enabled: no` to it or deleting it stops the updates, the decision survives later
  upgrades, and purging the package removes the file. Installing straight from the
  repository, without downloading a package first, is described in the README.

## 0.27.0 — 2026-09-21

- Learn a rule only from Enter. A manual conversion used to write the word into the rules
  the moment it ran, and the prompt that followed merely offered to finish the job, so
  carrying on typing, clicking elsewhere or letting the prompt expire left a confirmation
  nobody had given; two such conversions of the same word made the rule active on their
  own. Over thirteen days of collected logs that produced seventeen recorded rules against
  a single Enter. A conversion now only asks. Enter records the rule at once, and every
  other outcome leaves the rules exactly as they were, which is what Escape always did.
  Rules left half-confirmed by earlier versions stay inactive until they are confirmed or
  the learning store is cleared.

- Let the word decide what a quote in Telegram was. A quote typed in the Russian layout
  used to become the `@` of a mention the moment the key came up, which turned every
  quotation into a mention and switched the layout to English in the middle of a Russian
  sentence; pressing Pause afterwards repeated the same replacement instead of undoing it.
  The quote is now judged with the word that follows it, and the word alone is analysed:
  `ощрт` means nothing in Russian while `john` is a name, so `"john` becomes `@john`, while
  `"привет` stays a quotation. A quote typed against the end of a word closes a quotation,
  so `"john"` keeps both quotes and only corrects the name. A quote after a caret move or
  after the layout was chosen by hand is left alone, because those already stop the engine
  from touching the word.

- Keep the Windows accessibility reader alive. A focused element that offers no text —
  the ordinary state of a Chromium or Qt window until its accessibility tree is up —
  arrived as a null pattern and was treated as a broken provider: one such field marked
  the bridge unavailable, spent all three recovery attempts and switched reads off for the
  rest of the session. In collected logs that left 46 successful reads out of 1628, none
  of them in the editor or the chat client. Such a field is now an unreadable field, not a
  failure; retries slow down to one a minute instead of stopping for good; a bridge that
  cannot be closed is released rather than reused; and the request timeouts, previously
  tighter than a cold Chromium window can answer, are 200 ms. The log now names the
  exception class, the provider's HRESULT where there is one, and how long the last read
  took, still without formatting the exception itself, and `--diagnose` reports whether the
  focused field offers text at all.

- Keep LogCourier delivering after a long pause. The pinned catalog is now recognised by
  the SHA-256 in its caption instead of the `file_id` that Telegram reissues for the same
  document over time: a collector idle for days read a new identifier for the catalog it
  had pinned itself, reported the pin as lost or replaced, and stopped sending anything
  with no way back. A store written by an earlier version adopts the catalog pinned in the
  group, while a group with nothing pinned is still refused, so a chain of catalogs never
  restarts silently. Its installer release is separate.

## 0.26.1 — 2026-09-20

- Never replace a word with something that is not one. Six Russian letters sit on
  punctuation keys, so the Latin reading of an abbreviation such as `збс` is `p,c`; the
  detector's own conversions were already refused on that ground, but a conversion the
  context model asked for was not, and the shipped model asked for exactly that one in
  every application it knows. The refusal now covers every layer and is logged as a
  safety decision next to the model's unchanged opinion.
- Read the text in front of the caret on Windows. The accessibility reader had never
  opened on any machine but the one that built the release: comtypes writes the
  modification time of `UIAutomationCore.dll` into the interface module it generates
  and refuses that module wherever the file differs, unless the program declares itself
  frozen, which a Nuitka build does not. KeySwitch now declares it for the length of
  that one call, so the context model sees what is already in the field instead of only
  the keys typed since the last click.

## 0.26.0 — 2026-09-19

- Correct the layout on macOS. A Quartz event tap watches the keyboard, the text in
  front of the caret is read from the accessibility tree, the layout is chosen among the
  text input sources, and the word is typed again - the same engine and the same two
  models as on the other systems, which never knew which one they were running on.
  Typing `ghbdtn ` into a document now leaves `привет ` behind. The menu bar carries the
  current layout and the switches behind it, and the settings window is the one Windows
  already had: it moved out of the Windows frontend and takes each platform's answers as
  an object, so neither system keeps a private copy of the wording.
- Deliver a withheld Enter or Tab on macOS without the caution Windows needs. An event
  tap may delete the key it is given, so the key is genuinely withheld rather than raced;
  the whole difficulty that shaped 0.25.2 and 0.25.3 does not arise there.
- Ask for the keyboard permission instead of starting without it. macOS grants nothing on
  request, so KeySwitch shows the system's own dialog, opens the pane where the switch
  lives, and waits: the moment the permission is given it carries on, with no relaunch. A
  program that looks alive and corrects nothing is the hardest fault to report, so the
  engine is not built at all until the permission is there.
- Sign the macOS application with a Developer ID and carry one entitlement with it. The
  hardened runtime that notarization requires forbids unsigned executable memory, and
  Python's foreign-function layer builds its call thunks there; without the entitlement
  the program does not fail, it hangs before printing anything.

## 0.25.3 — 2026-09-18

- Forget the keys a window change left held. A key still down when the focus moves
  reports its release to the window that took over, and often to nobody the hook can
  see, so the press sits in the engine's books until the stale timer drops it three
  seconds later. A withheld Enter gives up after two, so it was thrown away while
  waiting for a key that would never come up - the cause behind the lost Enter presses
  that 0.25.2 worked around at the deadline. The press is now forgotten at the focus
  change itself, the moment it stops meaning anything; a key that really is still down
  loses nothing, because its release simply finds nothing to clear. The technical log
  names the keycodes released this way.

## 0.25.2 — 2026-09-18

- Deliver a withheld Enter or Tab that was waiting behind a key whose release was never
  seen. The engine holds the key so the corrected word is submitted rather than the
  wrong one, and it waits for every key to come up first; a press whose release is lost
  is only forgotten after three seconds, while the action gave up after two, so the
  keystroke was dropped one second before the obstacle would have cleared itself. On a
  Windows session that cost 16 Enter presses, and the user saw a keyboard whose Enter
  had stopped working until KeySwitch was closed. A key already held when the Enter
  arrived and still held at the deadline is now forgotten and the key goes through; the
  cautious answer is kept where it belongs - when the Enter's own release was never
  seen, or when a key pressed after it is still down.

## 0.25.1 — 2026-09-18

- Let an erase cancel the hold a caret move puts on the next word. The rule exists
  because nobody knows what stands in front of a caret that was moved into finished
  text; a user who then rubs that text out has answered the question, and an editor
  emptied with Home, Shift+End and BackSpace is the commonest way to ask it. Without
  this, the word typed after clearing a field was left in the layout it was typed in.

## 0.25.0 — 2026-09-18

- Keep the context-v1 + prefix-v1 pair. The context-v3 + prefix-v2 candidate
  `context-v3-d13d10b579b3` + `prefix-v2-1988a5741fe6` passed its sealed evaluation on
  512 rows of Tatoeba sentences and the Fedora Rawhide package index, restoring 228 of
  252 wrongly typed rows against 210 for the shipped pair and corrupting fewer correct
  rows in both settings modes. The end-to-end tests then found what those aggregates
  cannot show: three words of the curated short-word table (`мы`, `вы`, `if`) are
  offered as a suggestion instead of being switched. The release ships the proven pair;
  the candidate waits for the rule that decides those words to be repaired.
- Choose the serving threshold inside a band the pinned cases define instead of by the
  calibration counts alone. The counts are nearly flat across the interval that matters
  while the two kinds of error are not, so the band names both ends: below it a command
  name wins over an ordinary word (`лут` became the command `ken`), above it a plain
  word typed in the wrong layout is left alone (`кот` stayed `rjn` when the user
  paused). This decides which model is served, not how one is judged.
- Leave the version literal out of the fingerprints a model receipt carries, and check
  the shape of that one module instead: a docstring and one string assignment, nothing
  else. Every release rewrites the version by construction, so pinning it tied a model's
  evidence to the release number it was sealed under - a pair could not ship in a later
  version without spending a sealed test on a string.
- Decide a word once more when the pause arrives and the next word never did. The engine
  postpones a short word whose neighbour should settle its direction; if the neighbour
  never comes, the postponement used to end in silence and the word stood as typed.
- Convert a lone `z`, `f`, `b`, `c`, `d`, `r`, `e` or `j` at the start of a message
  the moment the space is typed, instead of holding it until the next word arrives.
  Those eight keys carry the eight one-letter Russian words, and a message that opens
  with one used to sit in the wrong layout until something followed it - or until it
  was sent. The letter is also taught to the models now, so a later pair decides it
  rather than inheriting the rule.
- Decide command names that carry digits by the models instead of leaving them out of
  the analysis. `зь2` becomes `pm2`, and `pm2` typed as itself stays, which the blanket
  exemption for code-like tokens could not tell apart.
- Drop 467 two-letter forms from the Russian lexicon supplement. Every real two-letter
  Russian word is in the onboard vocabulary already, and what the frequency list added
  at that length was subtitle noise - `зь`, `пм`, `ср`, `фы` - each of which disguised
  a short Latin command as a Russian word.
- Leave the word that follows an arrow key, `Home` or `Page Up` alone. The caret then
  sits where the program has not watched anything being typed, and it is usually moved
  back into existing text to finish or fix a word; analysing that word as the opening
  of an empty field is the one reading it is least likely to have. `Pause` still
  converts it by hand, the new setting turns the behaviour off, and when the active
  field can be read the real text before the caret is used instead.
- Register the packaged executable in the Windows startup list even when the running
  build reports an interpreter as its own program. The startup entry could end up
  pointing at `python.exe`, where Windows shows the entry as "Python" with no
  publisher and starting it does not start KeySwitch. The command is now taken from
  the installed layout - `KeySwitch.exe` beside the package directory - and only a
  source checkout, which has no executable there, keeps the interpreter command.
- Report the startup state in the diagnostics window, not only in
  `KeySwitch.exe --diagnose`: the command that is registered, whether Windows blocks
  it and whether its target still exists. That is the report people copy when
  autostart does not work.
- Make the public receipt verifier ask for what the gate policy it publishes says. Two
  of its checks still demanded no calibration error and no corrupted row at all, a rule
  the policy replaced with a measured balance against the installed pair; a verifier
  stricter than the policy it prints refuses the candidates that policy accepts, and
  does it without saying so.

## 0.24.0 — 2026-09-17

- Keep the context-v1 + prefix-v1 pair. The schema-3 candidate
  `context-v3-adc0a5782e2d` + `prefix-v2-d1ee002d9ab2` passed its sealed evaluation on
  treebank text (no corrupted rows against 7-28, 301 of 301 preserved) and then, with an
  authored expectation changed for it, fell short on a second sealed evaluation built
  from Tatoeba sentences and the Ubuntu package index by two to three restorations
  out of 247. The gates are not lowered; the pair stays a candidate, and the release
  ships the runtime changes below on the proven pair. The engine, the prefix gate and
  the Windows validator can now install a later-schema pair once one is accepted.
- Rewrite a Telegram quote as `@` on the keystroke instead of after the idle pause.
  A mention is typed as `@` and the nickname straight after, so waiting a second
  before the `@` appeared defeated the point. Pressing the same key again now
  writes the two quotes after all and returns the layout, which is what a user
  who meant a quotation does.
- A word may no longer be replaced by something that is not a word. Six Russian
  letters sit on punctuation keys, so the other reading of a mistyped `рукх` was
  `her[`, and the character models, scoring the letters alone, could prefer it.
  The shared automatic decision now refuses a replacement that spells punctuation
  for a source that spelt only letters; the opposite direction, `[jhjij` to
  `хорошо`, is untouched. With that in place the wider unknown-word recognition is
  on by default: on the frozen prefix population it now changes no correctly typed
  row that the default did not, where before it changed one more.
- Convert a lone letter at the start of a message. `z` and `b` typed in the
  English layout are `я` and `и`, but the single-letter rule required a Russian
  word before them, so the first word of a message never converted. In the
  Russian training treebank 14% of sentences open with one of the curated
  single-letter words; in the English one no sentence opens with a lone
  f/b/c/d/r/e/j/z, while inside English sentences those letters do occur. So the
  letter converts with no previous word and still stays after an English one. A
  user who keeps typing without a pause makes that immediate correction abort as
  unsafe; the letter then waits for its next word and is decided with it. The rule
  is explicit for the new context model as well, where the other curated short-word
  rules are the model's to weigh: the corpus holds too few examples per letter for
  weights to learn what the treebank counts state outright.
- Let an accepted prefix model of the second schema be installed. The engine
  loads the installed artifact through the schema-aware loader, the prefix gate
  accepts a later schema by the context-action pair receipt instead of the
  prefix-v1 evidence, and the Windows validator compares the bundled model with
  the artifact the gate accepted rather than with the prefix-v1 seal. The frozen
  prefix-v1 module and its evidence are untouched.
- Explain the two recognition settings in plain words: what the wider recognition
  decides and what it protects against, and why the early layout switch is off by
  default with the numbers from the independent evaluation.

## 0.23.1 — 2026-09-16

- Publish what 0.23.0 could not. Its tag exists, but the release job stopped at the
  Windows coverage gate: the new autostart diagnosis has a branch that only runs
  where the registry is unreachable, which on Windows is never, so the gate counted
  it as untested. The branch now has its own test on both platforms, and nothing
  else in the release changed.

## 0.23.0 — 2026-09-16

- Stop converting correctly typed Russian words that the shipped lexicon does not
  know. The packaged supplement of 199,680 Russian forms is now part of the
  lexicon the engine serves (`LanguageModel.load(locale, supplement_words(locale))`),
  not just of the training pipeline. Five of the six disclosed regressions of the
  installed model pair are gone with the same weights: `гифку`, `флуд`, the chat
  sequences and the code-editor comment keep their spelling. The sixth, an
  isolated word that neither the onboard lexicon nor the supplement knows (`дюп`),
  is still converted and stays in the suite as a disclosed expected failure.
  The authored sequence tests now load exactly what the product loads, so a
  lexicon change can no longer look better in the tests than it is for a user.
- Turn the early layout switch off by default. On the development population it
  corrupts 13 correctly typed words out of 256 while restoring 116 of 229 wrong
  ones; without it the same pair corrupts 4 and restores 109. Corrupting text the
  user typed correctly is the worse failure, and the completed-word decision
  recovers most of what the early switch was restoring. The feature stays one
  switch away in the settings, and its description now says why it is off.
- Report the Windows startup entry honestly. Task Manager's Startup tab and
  Settings keep an approval byte per `Run` value under
  `Explorer\StartupApproved\Run`; a value disabled there is skipped at every
  logon however often it is rewritten, and KeySwitch rewrote it at every launch
  while reporting autostart as enabled. The autostart manager now reads that
  record and the command's target: `enabled()` answers whether the next logon
  will really start KeySwitch, `--diagnose` prints the command, the block and a
  missing target, and the settings window explains both cases. Toggling the
  switch in KeySwitch clears a Windows block, while the automatic sync at every
  launch no longer overrules a choice the user made in Windows.
- Read the active field by default. This is what gives the context model the
  text actually around the caret instead of the engine's own recollection, which
  is the evidence the model was built to use. It reads through the operating
  system's accessibility interface, recognised protected fields are skipped, and
  the switch is one click away for anyone who would rather it stayed off.
  The wider unknown-word recognition stays off, now with a measurement behind
  that: it is not model evidence but a lower bar for the character-level
  fallback, and on the frozen prefix population it changed one more correctly
  typed row in every profile while restoring nothing extra (11 changed rows
  against 12, 127 and 128 restorations either way); on 490 development sequences
  it changed no outcome at all.
- Add application quirks: rules about one application's own syntax rather than
  about language, each its own setting under a new "Особенности программ"
  section in both interfaces. The first one is Telegram: a quote typed in the
  Russian layout and left alone becomes the `@` of a mention after the same idle
  pause the completed-word decision uses, because `@name` is how Telegram
  addresses somebody and the quote sits on that very key. Two quotes in a row
  stay quotes, so does a quote the user kept typing after, another application is
  untouched, and the undo key puts the quote back.
- Make the sequence protocol's `early_off` control mode run the engine's idle
  callbacks like the default mode. Without them a document that ends without a
  boundary key never reached the completed-word decision at all, so the control
  measured the early switch alone for every single-token document: 0 of 116
  technical rows restored, against 97 to 112 once the callbacks run. Gates and
  thresholds are unchanged; the audited protocol digest is updated and the three
  consumed sealed tests are unaffected by it.

## 0.22.0 — 2026-09-15

- Publish everything since 0.20.0. The 0.21.0 section below was tagged but its
  release job never produced assets, so this is the first published release
  that carries those changes: the v23 layout-intent seal
  (`intent-v1-b2a2ec8caa8d`), the frozen Hunspell inputs, the environment
  probe and the orthotactic gating rules.
- Re-fit the completed-word context policy against the v23 intent baseline:
  `context-v1-a24683995ca4`, artifact SHA-256
  `121e32d42939a9be6c9e6837bb7c4a5939778fcbf0c12b248e168225dba15f47`. Same
  author-written scenarios, same gates: 36,618 synthetic rows, 16,539 of 17,658
  desired conversions restored, 0 false conversions, against 13,905 restored by
  the detector alone. Its report pins the runtime module it was evaluated with;
  after the schema-3 work below changed that module, the pin was re-established
  by replaying the training (`tools/train_context_model.py`) and checking that
  the artifact came back byte for byte, not by editing the digest.
- Ship the same prefix, boundary and orthotactic weights as before
  (`prefix-v1-2f0c54bf546b`, `boundary-v2-3b2b1af6693e`,
  `ortho-v1-bdb915e4f06f`) and re-verify their frozen evidence against this
  engine instead of retraining them on CI. `tools/verify_lexical_compatibility.py`
  attests that the lexical inputs the prefix and boundary corpora consumed are
  unchanged and that the frozen corpus, candidate, seal and report bytes match
  reviewed digests; `tools/verify_prefix_model.py --verify-frozen` and
  `tools/verify_boundary_v2.py --verify-frozen` check the numeric regressions;
  the engine replays were refreshed on the current runtime and pin the packaged
  intent model (`tools/auxiliary_runtime_evidence.py`): boundary-v2 18 of 18
  sequences exact with no correct word changed, prefix-v1 128 of 128 desired
  restorations with no length mismatch. The rejected context-v2 keeps its
  numeric history behind `tools/verify_context_v2.py --verify-frozen` and
  `tools/verify_context_v2_history.py`; `tools/verify_context_model.py` is the
  single active-model gate and picks the replay by feature schema. The
  strict-report verifier refuses a report that lacks a required gate even when
  every remaining gate passes.
- One automatic word decision for the engine and the training tools.
  `src/keyswitch/word_decision.py` (`automatic_word_decision`) is what the
  engine calls for every completed word, and
  `context_policy.evidence_for_decision` builds the context evidence the
  engine, the trainer and the evaluator all see. Two earlier defects came from
  corpora that mirrored the engine imperfectly; the tools now call the
  engine's own functions.
- Teach the context runtime feature schema 3 without installing it. The loader
  accepts schema-3 artifacts (`context-v3-…`), `ContextModel` carries the
  runtime masks a schema-3 model is calibrated with, the policy passes the
  boundary character and the origin of the following text, and the engine
  re-decides a waiting word against its planned next word for schema-3 models
  only. Every schema-3 branch is inert for the installed schema-2 weights,
  which the frozen engine replays above confirm. `tests/disclosed_regressions.py`
  marks six authored regressions of the installed pair as expected failures;
  they become hard assertions the moment a schema-3 model is installed.
- Package two lexica for the research features: `resources/identifiers.json`
  (14,160 command names from Debian trixie main, read through
  `identifier_lexicon.py`; evidence for schema-3 features only) and
  `resources/lexicon-supplement-ru_RU.json` (27,209 Russian forms from the
  OpenSubtitles 2018 frequency list that the onboard `ru_RU.lm` lacks; read by
  `tools/reference_lexicon.py` for training and evaluation only). The engine
  and the early switch keep the certified onboard lexicon: the shipped models
  were measured on it, and their frozen evidence pins it.
- Move prefix feature schema two into `src/keyswitch/prefix_schema.py`.
  `prefix_model.py` is pinned byte for byte by the prefix-v1 corpus, seal and
  compatibility gate, so the second schema (characters of the observed prefix)
  and the schema-aware loader live beside it; the engine keeps loading the
  installed artifact through the frozen module, which still refuses a
  schema-two artifact.
- Research that did not ship, recorded so it is not mistaken for shipped work:
  a context action classifier (schema 3, `model/context_v3/`) and a schema-two
  prefix model (`model/prefix_v2/`), trained on physical-key sequences with a
  natural lookahead curriculum, a test-only holdout frozen from UD Russian-GSD,
  UD English-GUM and the Debian sid Contents index
  (`tools/freeze_context_action_holdout.py`), and a joint acceptance protocol
  with one sealed test per pair. The best pair passed every development and
  calibration gate and failed the sealed test twice, by two rows each time
  (corpus v4: net restorations below the baseline; corpus v5: two corrupted
  correct words in the portable early-off mode). Nothing from it is installed.
- Pin line endings for every file whose digest enters a model receipt,
  including the frozen Hunspell dictionaries and the runtime and test harness
  the action replay hashes, so a Windows checkout cannot invalidate a model on
  one platform alone. The Windows packaging validator now reads the active
  context verifier's identity and the v23 pre-seal receipt.

## 0.21.0 — unpublished development, 2026-09-11

These changes were tagged but did not produce a published release. They remain
part of the work following the published 0.20.0 release.

- Seal the layout-intent model against what the toolchain **computes**, not
  what it calls itself. The sealed candidate hash used to cover seven strings
  naming the build machine - `python_build`, `libc`, `machine` and four more -
  so when an `apt upgrade` rebuilt python3.14 without changing the language
  version, the new build date moved the candidate hash and the release replay
  was refused, while every weight, threshold and calibration constant
  reproduced byte for byte. The name had changed and the results had not, and
  the seal could not tell the difference. Those strings now live in
  `model/intent_v1/build-environment.json`, and `manifest.toolchain` carries
  code and config digests only. Nothing is given up: the candidate hash already
  covers the weights, so an interpreter that computes differently is still
  refused - on the weights, which is the honest ground.
- Add `tools/environment_probe.py`, which measures the primitives the trainer
  actually depends on and reduces each to a digest: the FTRL update as the
  trainer writes it, exactly the libm functions it calls, summation order,
  float text round-trips, an exhaustive walk of all 1,114,112 Unicode code
  points, the FNV mixing loop, sort stability, integer arithmetic and the
  Mersenne Twister stream. The probe never votes on identity - its readings are
  provenance and its file is certified, so it cannot be weakened unnoticed -
  but it turns "the replay differs" into "libm moved, here is the cell". A
  one-ULP change to `sqrt` moves exactly `float_arithmetic` and `libm`; a
  changed `NFC` moves exactly `unicode`.
- Consume the sealed test's *answer* once, not just its ticket. Removing the
  environment from the identity gave up a side effect: a second look from
  another machine used to be refused before a sealed row was read.
  `model/intent_v1/seal-outcome-v23.json` now records the digest of the sealed
  sections unconditionally - before the gates are computed, before publication,
  before a metric reaches any file - so a rerun that computes a different
  answer stops without printing one. A failing run consumes the answer too,
  which is the point: it cannot be repeated with a tweak until the numbers
  look nicer.
- Freeze the Hunspell dictionaries into `model/intent_v1/sources/hunspell/`.
  The evaluation derives its lexical populations from them and pins the
  resulting corpus digests, so `apt upgrade hunspell-ru` failed the release
  without changing a single weight - training never consults a dictionary at
  all. A dictionary is an input to the evaluation, and this repository already
  freezes its other lexical inputs beside it. The frozen bytes are identical to
  the system ones today, so no corpus digest moved.
- Rotate the sealed split namespace to `keyswitch:intent-v23:physical-signature`
  and accept the candidate `intent-v1-b2a2ec8caa8d` with artifact SHA-256
  `47f86818c4c1243daeabfafd50d03dd9884aa3973bb546191afe02c4092a3f4d`. The
  certified toolchain changed, so the splits are re-cut and the sealed test is
  one this candidate has never been evaluated against.
- Stop the release checklist from claiming a byte-identical replay it never
  ran. `--replays 0` left the phase status at "running", which the runner
  records as "passed", and the checklist printed "Official and replay outputs
  are byte-identical: passed" after running no replay at all. A skipped phase
  now reads `NOT PROVEN`.

- Never consult the orthotactic model for a token the dictionary of the current
  layout already knows. `руку` is an ordinary Russian word whose keys spell the
  ordinary English word `here`; no character model can separate them, and asking
  it to try cost a threshold high enough to silence real commands. This is
  structural rather than statistical: no dictionary word is converted by this
  route at all.
- Never consult it for punctuation caught between letters either. `и"ю` is a
  quotation mark between two letters, not Russian written badly, and such
  fragments were setting thresholds. A hyphen or an apostrophe inside a token is
  a different matter — the engine joins those to the word before it tests for a
  boundary — so `Я-то`, `из-за` and `don't` keep their conversions.
- Add a measurement pipeline for the orthotactic model under `model/ortho_v2/`
  and `tools/ortho_v2_*`, with the evidence it produced. The weights ship
  unchanged: no candidate built on it beat `ortho-v1-bdb915e4f06f` under honest
  calibration, and `tools/verify_ortho_v2.py` fails closed if one is ever
  installed without doing so.
- Record what that pipeline found, because each item was a defect in how the
  model was measured rather than in the model. The corpus disagreed with the
  engine about which token reaches a model — a key that writes punctuation in the
  layout being typed ends the word, a hyphen does not, nothing is trimmed from
  the front — and nine percent of the population went unmeasured. The Russian
  layout puts the full stop and the comma on the key the US layout writes as `/`,
  which the layout table does not model, so 16.9% of Russian tokens were read
  back as words nobody typed: `Пф,` as `пфб`. The dictionary refusal was measured
  against the frequency list while the runtime also asks Hunspell, so ordinary
  Russian words became evidence against conversion.
- Freeze, for replay, the dictionary verdict for every token the corpus can
  present and the sentence-level check on whether a token's language label can be
  trusted at all. Both need a speller, and a replay cannot call one and get the
  same answer on two machines.
- Record the seven separations that were tried and rejected in
  `model/ortho_v2/development-history/rejected-separations.json`, with the
  numbers. The last of them is the important one: discounting labels the corpus
  cannot vouch for looked like two free points of accuracy and turned `гифка`
  into `ubarf` and `Ютуб` into `Юne,`. Excluding a label does not make the token
  safe to convert; it only hides it from the metric.
- Document a defect of the shipped model found while measuring it: `флуд`, `лут`
  and `дюп` are converted although they are ordinary Russian. They are in neither
  the frequency list nor Hunspell, so the dictionary refusal does not reach them,
  and raising the threshold to cover them costs more conversions than it saves.
- Bind the orthotactic artifact into the context engine replay evidence, which
  compared two models without recording which orthotactic weights were loaded and
  so could drift silently.

## 0.20.0 — 2026-09-09

- Add a counted orthotactic model over the physical key sequence. Because the
  us/ru map is a bijection, one key sequence has exactly two readings, so the
  likelihood ratio of two character models answers "was the layout wrong?"
  without asking whether the token is a word. A counted case channel closes the
  abbreviation reading that the ratio alone leaves open. It only licenses a
  conversion where the detector abstained, never vetoes one, and never overrides
  an explicit rule. Original sentence-group test: 0 false conversions on 23,137
  negatives, 96.7% recall. Token-family overlap with training means this is not
  an independent physical-family test; see the corrected
  [scope](model/ortho_v1/README.md). The lowercased lexicon stress track reports
  its residue openly.

## 0.19.1 — 2026-09-08

- Force LF for the short-word policy source, whose digest joined the context
  model provenance in 0.19.0. A Windows checkout converted it to CRLF, so the
  packaging gate rejected an unchanged file and no Windows artifact was ever
  published for that version. Every other provenance path was already pinned.
- Fail on any platform when a file hashed into model provenance is not pinned
  against end-of-line conversion. The check discovers hashed files from the
  recorded evidence as well as from code, so a future provenance entry cannot
  reintroduce a mismatch that only a Windows checkout can see.

## 0.19.0 — 2026-09-08

- Retrain the completed-word context policy on a corpus that covers what the
  engine actually sends it: out-of-lexicon technical terms, dotfiles, Russian
  and English jargon and misspellings, the word after a literal `/`, every
  application with the `unknown` field role reported when the field is not
  read, and the curated short-word override the engine applies before the
  model. Select the epoch by the development action metric under a
  zero-false-conversion budget instead of weighted log-loss, which stopped
  before the model reached the unchanged 0.985 serving threshold. Score a new
  independent holdout; keep the rejected candidates as development evidence.
- Analyse the word after a literal `/` head (`bild/c,jhrb`, chat `/c,jhrb`) in
  the contextual assist mode instead of treating the whole token as code; the
  head is never replaced, a head that may itself be a wrong-layout word abstains,
  and manual Pause still converts the whole token. Record `literal_head` in
  `word_evaluation`.
- Keep a curated short-word exception from being cancelled by a probabilistic
  `keep` or an under-confident `convert`; `wait` still delays it. Record the
  decision source separately.

## 0.18.0 — 2026-09-07

- Train and activate a separately sealed boundary-v2 ranker for completed-token
  spans. Preserve internal EN/RU punctuation keys until continuation, a hard
  boundary or idle; keep literal suffixes and abstain on uncertain segmentation.
  Retain the existing intent/context/prefix weights and rejected boundary-v1 evidence.
- Cancel stale contextual lookahead when accepting manual Pause, including
  commands still waiting for physical key releases.
- Retry transient field-reader failures at bounded 5/15/60-second intervals;
  record recovery state without exception contents and do not retry missing
  dependencies, explicit access denial or failed provider cleanup.
- Add learned-model, exact-text, undo, Enter/Tab and native X11 regressions;
  bind package contents to the accepted weights and reproducible evidence.
- Share a portal-free isolated GUI test runner across CI and release checks.
  Disable GTK4 portals only in test processes, without sudo or system changes,
  and preserve real accessibility tests.

## 0.17.2 — 2026-09-07

- Record only known non-default settings in versioned diagnostic snapshots;
  log changes and resets explicitly, summarize private collections without
  their contents, and refresh the snapshot when diagnostics are re-enabled.
- Correlate short-word context waits, cancellations, manual conversions and
  observed edits without adding printable keystrokes or adjacent field text
  to the new events. Distinguish baseline decisions from contextual decisions.
- Report safe accessibility-provider failure categories and initialization/read
  stages without exception contents; retain the existing reader retry behavior.
- Clarify diagnostics and their limits, add privacy and input-sequence regressions,
  and archive previous engine evidence while retaining model weights and results.
- Name GitHub Actions runs by KeySwitch tests/release or LogCourier purpose,
  independently of the triggering commit title. This is not a LogCourier release.

## 0.17.1 — 2026-09-07

- Fix Windows package verification failing while printing Cyrillic prefix-model
  report examples to a legacy-encoded stdout. Emit lossless ASCII-safe JSON;
  preserve validation gates, model weights and application behavior.
- Add real CLI regression checks for cp1252, ASCII and UTF-8 output, including
  exact report round-tripping. Include the mid-word model and settings changes
  from 0.17.0, whose cross-platform release publication was blocked by this error.

## 0.17.0 — 2026-09-07

- Train and independently evaluate a separate EN/RU prefix action model for
  mid-word switching with contextual assist enabled; preserve completed-word
  weights, personal intent and execution guards. Keep uncertain prefixes intact.
- Recheck prefix decisions, field identity, exclusions and settings after key
  rollover; preserve exact continuation, spaces, undo and correction confidence.
- Add sealed prefix data, reproducible training, explicit false-intervention
  metrics and exact-text engine replay with package promotion checks.
- Clarify Windows/Linux settings for the separate word and prefix models,
  context-disabled fallback and the trained 4–12 character prefix range;
  preserve saved choices and default values.
- Update LogCourier sources to detect KeySwitch version boundaries, publish
  version markers and select current-version logs by default when retrieving
  them. Keep older logs explicitly accessible; its installer release is separate.

## 0.16.2 — 2026-09-06

- Preserve phrase context, current-word continuation and manual conversion
  eligibility when input replay contains only confirmed key releases.
- Bound pending manual corrections and undo operations by a release timeout;
  report a safe cancellation instead of hanging or changing layout on a
  repeated Pause without new text. Do not infer a physical key-up from time alone.
- Attribute contextual decisions separately from baseline decisions in
  diagnostics, including unsupported/shadow fallbacks and field-reader status.
- Add reproducible engine-only regression refreshes with archived prior
  reports; retain the shipping model and the rejected candidate unchanged.
- Preserve literal punctuation in its original layout during correction undo
  on both backends, and support a bounded punctuation tail before a boundary.
- Add experimental delayed word segmentation and a separately trained public
  corpus boundary ranker. Reject the first candidate: one wrong segmentation
  and excessive abstention on the sealed test. Do not install its weights or
  activate delayed segmentation by default. Add exact text/action regressions,
  reproducible training and guards against accidental rejected-model packaging.

## 0.16.1 — 2026-09-05

- Synchronize Russian/English user guides, architecture, model instructions,
  Linux manual and settings descriptions with current context-policy behavior.
  Clarify log retention, password/input-verification limits, KSLM versus context
  evidence, and the separate CI/release checks. Add documentation navigation,
  troubleshooting and release-recovery guides; leave model weights and defaults unchanged.

## 0.16.0 — 2026-09-05

- Add a frozen CC0 English/Russian phrase corpus with source/near-duplicate
  grouping, separate development/calibration/test partitions, a focus-lexical
  holdout and an unused reserve. Include spelling-error keep interventions
  and separately authored technical safety cases without using private logs.
- Train and seal a larger context candidate with a parity-tested training-only
  native optimizer and fixed portable/Hunspell lexical evidence. Replay its
  weights and independent reports byte-for-byte in CI.
- Compare the candidate with the shipping model through contextual decisions
  and visible-editor engine replay. Reject it for runtime promotion because
  fewer false conversions came with substantially more missed corrections.
  Keep shipping model weights, correction behavior and user defaults unchanged.
- Gate both native packages against accidental activation of the rejected
  candidate; publish the corpus, failed experiment and limitations for review.

## 0.15.0 — 2026-09-05

- Check native AT-SPI initialization before field access and remember startup
  failure for the process lifetime, avoiding libatspi's fatal missing-bus path.
  Cover unavailable-bus startup and reader recreation with a native subprocess
  regression test, isolate parallel accessibility test sessions, and document
  the initial context corpus's limited diversity.
- Add a local, trained four-action contextual policy (keep/convert/wait/suggest),
  app/field evidence, bounded RAM-only phrase context, assist/shadow/off settings
  and redacted decision diagnostics. The initial quality evidence is synthetic,
  not a claim of general language understanding or real-world error rates.
- Resolve a waiting short word with its immediately following word while
  preserving the space, physical stroke order and joint undo. Explicit user
  rules, exclusions and input-integrity guards remain above model decisions.
- Add opt-in UI Automation/AT-SPI caret context without clipboard access,
  detected-password/selection guards and exact field/suffix revalidation.
- Disable prefix replacement in contextual assist mode; retain it in legacy
  and shadow modes. Add reproducible training, token-family holdout evidence,
  artifact provenance gates and context-specific regression tests.
- Resolve X11 common-key group fallback through XKB instead of treating the
  Russian-layout space as an empty character. Log rejected unrepresentable
  replacement plans even when the current word buffer has already been cleared.

## 0.14.1 — 2026-09-05

- Windows native E2E waits for real keyboard-driven field clearing before
  asserting the result, using layout-independent Home/Shift+End/Backspace.
  Diagnostics use UTF-8 and always close the test window;
  watchdogs turn an unhandled GUI stall into a bounded failure with tracebacks.
- This release includes the input-integrity and correct-before-Enter changes
  from 0.14.0, whose installer publication was stopped by the E2E harness defect.

## 0.14.0 — 2026-09-05

- Input integrity audit and an EN/RU regression matrix covering punctuation,
  whitespace, identifiers, Unicode, focus, submission and manual correction.
- Pending replacements are cancelled after unsafe queued input, backspacing,
  pointer activity or focus changes. Undo cannot rewrite an old word after the
  text or caret changes. Early and late-prefix corrections wait for key-up.
- Windows intercepts Enter, Numpad Enter and Tab before delivery: correct the
  word first, then send the action exactly once. Subsequent input stays ordered,
  including a second queued message. Failures, focus changes and missing key-up
  cancel the action. Shift/Ctrl/Alt shortcuts are not intercepted. X11 leaves
  these already delivered actions alone; its trigger controls remain disabled.
- Windows prompt answers suppress key autorepeat through release. Shift+Enter
  remains an application action. Held input uses a separate replay marker and
  is released on every exit path; partial SendInput errors include event counts.
- Preserve internal hyphens/apostrophes, leading digits and identifier markers;
  support Unicode whitespace boundaries and reject unsafe multi-character input.
  Replay letter case using each stroke's Caps Lock state.
- Explicit learned short-word rules are no longer hidden by minimum length.
- Continue tracking the entire word after a boundary-free correction, so
  subsequent typing, Backspace and Pause do not operate on a detached suffix.
- Diagnostics distinguish key presses from all keyboard events, include a
  correction id, and explicitly state that application text was not read back.
  README now explains that technical logs can contain uncorrected private text.
- Windows punctuation keys are named in the log the way X11 names them
  (`comma`, `period`, `apostrophe`, …) instead of the raw `VK_BC` codes.

## 0.13.0 — 2026-09-04

- A correction no longer collides with what is typed into it. The layout is
  switched first and the deletion and the replacement travel in one
  `SendInput` call, which Windows keeps together; keys that arrived after the
  word — typed before the engine got to it, or while it was replacing the
  text — are deleted with the word and typed again in the new layout, and
  keys pressed during the injection are held by the hook and typed last. On
  X11 the late keys are handled the same way; XRecord cannot withhold input,
  so nothing is held there. `correction_applied` reports `late_keys` and
  `held_keys`.
- `Pause` right after an automatic correction now records a rejection of that
  correction, exactly as the undo hotkey does, instead of teaching a rule for
  the way back: the log showed a false correction that had been fixed by hand
  twice and still fired a third time. Toggling a manual conversion back and
  forth with `Pause` no longer counts as confirmations: three toggles used to
  turn a typo into an active rule while the prompt had been dismissed.
- A short word found only in the other language's frequency list is left
  alone when it reads as ordinary text in the language it was typed in and the
  previous word does not favour the switch: `дев` after `на` was traded for
  `ltd`. The bar is the one the unknown-word branch already applies; it lives
  in the policy layer, the certified detector is unchanged.
- The most frequent Russian function words typed in the Latin layout are
  corrected at last: `yt`, `gj`, `yf` (`не`, `по`, `на`) on their own, `kb` and
  `nj` (`ли`, `то`) after a Russian word, and single letters `а`, `и`, `с`,
  `в`, `к`, `у`, `о`, `я` after a Russian word. A trusted short word never
  turns a key that is punctuation in one layout and a letter in the other into
  a word boundary, so `общих` is not cut into `о` and a comma.
- The maintenance page lists what local learning remembers: the typed word,
  what it becomes, the direction, the confirmations gathered and required, and
  the rejections made by undoing.
- The log names a correction that is superseded by a boundary or replaced by
  an undo, and reports the layout of KeySwitch's own window once per visit
  instead of on every poll — that line was fifteen percent of a session.

## 0.12.0 — 2026-09-04

- The key that answers the learning prompt no longer reaches the text as well.
  Enter used to confirm the rule *and* arrive in the editor underneath, which
  in a chat sends the half-written message; Esc had the same double life. The
  Windows hook now withholds an unmodified Enter, keypad Enter or Esc while the
  prompt is on screen — together with the matching key release, so no window
  sees a key-up without its key-down. The prompt still does not take the focus,
  so typing around it is unchanged, and once it is answered or its eight
  seconds run out the keys belong to the application again. Linux keeps passing
  them through: XRecord only observes and cannot withhold a key.
- The Windows end-to-end scenario proves that injected input actually reaches
  its field before it starts typing into it, and retries the activation a few
  times instead of failing ten seconds later with an empty field.

## 0.11.2 — 2026-09-04

- A correction records how much the user typed into it. `correction_applied`
  and `correction_failed` now carry `keys_during_injection` — the real
  keystrokes that arrived while the text was being replaced — next to
  `queued_events`, `replayed_strokes` and `boundary_replayed`. Nothing reads
  the text back from the application, so this is the signal that explains a
  replacement that came out with extra or missing characters while every step
  reported success.
- A correction that is scheduled but never runs is no longer silent. It waits
  for the release of the key that triggered it, and a shortcut, a caret move, a
  focus change or a second `Pause` in between used to cancel it without a
  trace; the log now records `pending_correction_dropped` with the reason, the
  word and the direction. `Pause` that has neither a word nor a second layout
  records `manual_conversion_impossible`.

## 0.11.1 — 2026-09-04

- The technical log now explains what local learning did with a word. Every
  `word_evaluation` carries a `learning` block — whether learning is on, the
  target and confirmation count of the rule for this word, the confirmations it
  still needs, the target a confirmed rule forces, and the rejections
  remembered after an undo. A rule with one confirmation out of two changes
  nothing yet, and until now such a line was indistinguishable from a word with
  no rule at all.
- The per-setting reset button now updates its control at once instead of
  waiting for the settings queue tick, so the value shown always matches the
  value stored.
- Writing a rule is an event of its own: `learning_rule_recorded` names the
  word, the direction, the confirmation count and whether the rule became
  active, and `learning_rejection_recorded` records an undo that blocks a
  correction. Confirming the prompt reports the resulting count as well.

## 0.11.0 — 2026-09-04

- Every page of the Windows settings window scrolls. A page taller than the
  window carries its own scrollbar and answers the wheel and PageUp/PageDown,
  instead of hiding the settings below the fold; the wheel over a number field
  or a drop-down scrolls the page rather than silently changing the value.
  Descriptions wrap to the width the column really has, so the pages survive a
  narrow window too.
- A setting that differs from its shipped default is marked the way an editor
  marks an edited line: an accent bar beside it, an accented title and a
  "Сброс" button that restores that one value. The gutters holding them are
  reserved, so nothing moves as the markers appear. Setting rows also share
  the page background now instead of the grey band the platform theme drew
  behind them.
- A digit or a symbol typed inside a word no longer throws the word away.
  `зь2` used to leave an empty buffer, so `Pause` had nothing to convert and
  only switched the layout; it now becomes `pm2`. Automatic correction is
  unchanged — the detector still treats a token carrying a digit as code and
  never touches it on its own.
- Whenever the engine does drop an unfinished word — a shortcut, a caret move,
  a layout change mid-word, a focus change — the technical log records
  `word_discarded` with the reason, so a correction that never happened can be
  explained from the log instead of guessed at.
- The log file is installed on the root logger directly instead of through
  `logging.basicConfig`, which does nothing at all once anything else has
  configured logging first — and does it silently, leaving an empty
  `keyswitch.log` for a whole session. The maintenance page now states whether
  the journal is really being written and how large it is, and the same state
  is part of the diagnostics report.
- Every line of the technical log carries the version that wrote it, and each
  session opens with a banner naming the version, the platform and the
  rotation budget. A rotated file that outlives an update can no longer be
  read as if it came from the version installed now.

## 0.10.0 — 2026-09-04

- The log file rotates by the mode it is in: ordinary operation keeps 1 MB in
  3 files, and the diagnostics ("developer") log, which writes a line per
  evaluated word, keeps 5 MB in 6 files. Turning the diagnostics mode on now
  starts a fresh file, so the log attached to a report contains that session
  only; turning it off keeps what was recorded. Both platforms share one
  `keyswitch.logsetup` module instead of duplicating the handler.
- A button on the diagnostics page opens the folder that holds the log, in the
  file manager on Linux and in Explorer on Windows. The page also states the
  rotation budget.
- `tools/release.py` performs a release in one command: it propagates the
  version to every file that spells it, closes the changelog section, checks
  the release notes, runs the verification contour, commits, tags, pushes, and
  waits for the workflow and the published packages. Every step recognises the
  work it has already done, so a re-run after a failure continues where it
  stopped; anything that does not hold stops the run with a message naming the
  file to fix.

## 0.9.1 — 2026-09-03

- Windows keeps a keyboard layout per window, so the layout that arrived
  together with another window (Telegram → TeamViewer, an IDE → the browser)
  was logged as a manual switch and protected the next word from
  autocorrection. The engine now tracks the focused window and treats such a
  change as that window's own layout; a layout the engine selects itself also
  drops an older manual pick instead of reviving it minutes later.
- Windows of KeySwitch itself (settings, the learning prompt) never count as
  a layout change. The Windows learning prompt is shown as a non-activating
  popup: it no longer takes the caret away from the editor or swallows the
  next typed key. Enter and Esc still answer it through the global hook, and
  because the editor keeps the focus they now also reach the editor itself.
- Moving to another window drops the unfinished word and marks the last
  committed one stale, so a typing pause or `Pause` never rewrites text in
  the wrong window.
- Learning is offered only for something that reads as a word (at least two
  letters): a lone letter converted to punctuation (`б` → `,`) is neither
  offered nor counted towards an automatic rule.
- The undo hotkey pressed while an early-switched word is still being typed
  reverts that prefix instead of the previous correction and leaves the rest
  of the word alone (also with manual layout respect off). An early switch
  abandoned without a boundary (navigation key, another window) is still
  recorded in the history and can be undone.
- Early switching accepts prefixes typed on punctuation keys that are letters
  in the other layout (`nt,z` → `тебя`).
- Technical log: layout observations carry `focus_changed`; new events
  `focus_changed`, `layout_change_ignored` and `early_switch_undo_scheduled`;
  `manual_conversion_scheduled` reports `learnable`.

## 0.9.0 — 2026-09-03

- `Pause` converts only what was typed after the last word boundary: the
  unfinished word, layout-dependent symbols such as the Russian quote meant as
  `@`, or the last completed word while nothing else was typed after it;
  otherwise the key only switches the layout and protects the next word.
- Early layout switching: once a prefix (4 letters by default, configurable
  3–8) has no continuation in the current language and clearly continues in
  the other one, the layout is switched and the prefix rewritten as soon as
  the last letter's key is released (letters pressed before that release are
  absorbed). A letter that arrives in the old layout within 0.5 s is converted
  on its own;
  the finished word is recorded as one correction at the boundary.
  `detection.early_switch` and `detection.early_switch_min_length` settings,
  rows in both settings UIs and a slowly typed X11 E2E case.
- Configurable typing pause (`detection.pause_delay_seconds`, 0.3–5 s, default
  1.5). The engine wakes exactly when the delay elapses instead of on a 0.5 s
  grid, and key presses without a release older than 3 s no longer block pause
  correction forever.
- A change to the layout the engine itself just selected, observed within
  1.5 s, is no longer treated as a manual switch, so a correction, `Pause` or
  the menu action does not protect the following word by mistake; switching
  away from it stays manual. Only key presses (not releases) update the
  observed layout.
- Technical log: `word_evaluation` now carries `skipped_reason`, protection
  details, the context that was used, `idle_ms` and a shadow detector verdict
  for protected or disabled words; `correction_applied` records the mode,
  deleted characters, previous layout and injection time; new events for
  deferred pause corrections, pruned stale presses, early-switch evaluations,
  late strokes, layout switches without a word, learning prompt lifecycle and
  attributed layout changes; `setting_changed` includes scalar values.

## 0.8.0 — 2026-09-02

- Accept the v20 layout-intent candidate `intent-v1-6ece07f881ec`, the first
  certified under the compiled FTRL kernel and the multi-process evaluator.
  The trainer change altered the toolchain identity behind v15, so the split,
  registry, hard-negative source and holdout namespaces were rotated per
  candidate: v16, v17 and v19 failed the pre-sealed gate (zero-false-positive
  recall on their threshold splits 0.9479, 0.9458 and 0.9463 against the 0.956
  minimum; no registry was claimed), v18 passed its internal gates and holdout
  but was rejected by the `fallback_regression` strict gate (one model-introduced
  false positive on the 5,000-row sealed sample; `rejection-v18.json`), and v20
  passed everything. Freeze `unknown-typo-development-v20.json` (SHA-256
  `61e02546fb05c2502b2535c512b0e11fad13042d25b1f4f70cff621a4e35686f`), preseal
  `holdout-v20-preseal.json` with zero overlap against 288,869 sealed and
  10,000 development signatures, select epoch 45 of 49 on development with
  765,166 nonzero weights and 1,029,480 membership fingerprints. The
  independent strict report, SHA-256
  `01cc92bfc293019377019ecdcd965af11a61ac3da8cdf3837915459bf9f1d525`,
  passed all 30 gates: on the 60,000-negative model-blind holdout the ensemble
  produced 6 false positives (1 per trigger slice, Wilson upper endpoint
  0.000566269 against the 0.001 limit, none introduced by the model, 42
  fallback false positives prevented) with recall 0.944483; every ordinary
  sealed trigger shows 1 false positive among 21,338 negatives with recall
  0.954539 and Pause shows none with recall 0.946712. The published artifact
  has SHA-256
  `85deddb83e041f52622b794cf919770994d71a9f1c50af482be4f6574c4163cd`; the
  manifest and test-report SHA-256 values are
  `9c39b615ba90b94107be6bef0140ce9387e493bb6aae195f4a8d116021283da9` and
  `f3c44b42c96ce654042d17c822d92bd3202a9d1b12d6b28e34e394531a10fa94`. Two
  independent sequential retraining runs reproduced byte-identical KSLM,
  manifest and test-report files, and the strict evaluator re-run against the
  first replay passed all 30 gates with report SHA-256
  `5e77f44b857c9096cc306ce4de3232f81037df932d1d3b5c8ca01de8082404fc`.
- Add `tools/write_intent_rejection.py`, which writes `rejection-vN.json`
  from the manifest, registry and strict report so a rejected candidate's
  receipt cannot disagree with its evidence.
- Score rows on worker processes in the strict intent-model evaluator
  (`--workers`, default every available CPU). Model predictions, fallback
  comparisons and production-context profiles are pure per-row functions, so
  results are identical in any worker count; the context-invariant prediction
  cache is replayed in the parent from the recorded calls so its reported
  counters stay exactly those of a sequential run.
- Precompute the US/RU layout translation tables once and translate through
  `str.translate`. The mapping is unchanged character for character, but the
  runtime, trainer audits and evaluator no longer rebuild the dictionary on
  every call (about 20x faster per translation).
- Run the sequential FTRL-Proximal epochs of the layout-intent trainer in a
  compiled kernel. The C source is embedded in `tools/train_intent_model.py`,
  compiled with fused multiply-add disabled and called through `ctypes`; it
  mirrors `FTRLProximal.update` expression by expression, including the
  compensated float `sum()` of CPython 3.12+, and must reproduce the Python
  reference bit for bit on the first 4,096 real rows before the first epoch.
  `--ftrl-kernel auto|native|python` selects the loop; the bytes are identical
  and an epoch takes seconds instead of a minute.
- Let `packaging/build-deb.sh` reuse a strict intent-model report produced
  earlier in the same release contour instead of repeating the half-hour
  evaluation. `tools/verify_intent_strict_report.py` accepts the report only
  when every gate passed and every recorded hash of the artifact, config,
  frozen sources, model toolchain and preseal receipt still equals the current
  file; CI, the release pipeline and packaging share that single check.
- Run the Windows release job concurrently with the Linux job instead of
  after it, and make the release pipeline's `build-deb` phase depend on
  `model-strict` so one strict evaluation serves both.

## 0.7.0 — 2026-09-02

- Accept the v15 layout-intent candidate `intent-v1-bec1f1d3dceb` after the
  multiprocess trainer changed the toolchain identity behind v14. Rotate the
  split, registry, hard-negative source and holdout namespaces to v15, freeze
  the model-blind `unknown-typo-development-v15.json` (SHA-256
  `a0585bdbd21526434fc77effc64200075269d884321a702fa44bd8a9dc7f963c`) and
  preseal `holdout-v15-preseal.json` with zero overlap against 288,869 sealed
  and 10,000 development physical signatures. The trainer ran all 64 permitted
  epochs and selected epoch 64 with 765,205 nonzero weights and 1,031,416
  membership fingerprints. The independent strict report, SHA-256
  `82ff2b6f332369eea2e71eb2df4960a554a1aa9de9c31025c35bf15d4485c303`,
  passed all 30 gates: on the 60,000-negative model-blind holdout the
  production ensemble produced 12 false positives (2 per trigger slice, Wilson
  upper endpoint 0.000728996 against the 0.001 limit; 6 introduced by the model
  while 60 fallback false positives were prevented) with recall 0.96935, and
  every sealed per-trigger slice shows 2 false positives among 21,574
  negatives with recall 0.973 (Pause 0.968). The published artifact has SHA-256
  `7631b821bafc958364353a8a13de3abc23e922e51b589bd181075db55fa9e9dc`; the
  manifest and test-report SHA-256 values are
  `e0070e8e6813da4a8dde1a09eb2c1713f033d002a64216299cba3764032d82f7` and
  `05caf3828ff2724fc5f1d22ff2e28d9b31cd2d1bcfcceb64f934bb8bfe84480d`. Two
  independent sequential retraining runs reproduced byte-identical KSLM,
  manifest and test-report files, and the strict evaluator re-run against the
  first replay passed all 30 gates with report SHA-256
  `88f6704c845efd86b8c3ba924607bcdf972e915d0d7d6c75ff147e1d0099f23e`.
- Add `tools/release_pipeline.py`, an unattended Linux release pipeline that
  runs model provenance and reproducibility evidence, strict typing, coverage,
  detector gates, X11/tray E2E, the native Debian build, its verifier and the
  packaged E2E as one detached process. Phases form a dependency graph and run
  concurrently under a memory-aware scheduler; every run leaves a live
  `state.json` plus final `summary.json`/`SUMMARY.md` with the runbook
  checklist for later review.
- Pin the v15 registry, preseal receipt and frozen development source in
  `.gitattributes`, describe the v15 contract in the man page and pin
  `intent-v1-bec1f1d3dceb` in the Windows packaging contract test.
- Use every CPU available through process affinity by default while training
  the layout-intent model. Parallelize deterministic feature extraction,
  frozen-model epoch evaluation, calibration, threshold and sealed-test
  scoring; preserve canonical row order and provide `--workers N` for resource
  limits or single-process diagnostics.
- Preserve the maximized or snapped state of the active Windows application
  when the learning prompt closes. Return keyboard focus without issuing
  `SW_RESTORE`, and cover the maximized-window confirmation path in Win32 E2E.

## 0.6.1 — 2026-09-02

- Correct the reviewed Russian-layout `ша` to English `if` case independently
  of the configured minimum word length. Require an exact target lexicon hit
  and a 100x frequency advantage; keep other two-letter collisions behind the
  normal minimum-length guard to avoid new false positives.
- Give an explicit manual layout change absolute priority for the next complete
  word, including pause correction and previously learned automatic rules.
- Add opt-in structured technical decision logging to the Linux and Windows
  settings. Include model/session metadata, score and correction events, redact
  words from excluded applications, and rotate the 5 MiB log with three
  backups.

## 0.6.0 — 2026-09-01

- Accept the v14 layout-intent candidate after all 30 strict gates passed.
  Freeze training config schema 13, split/registry/source/holdout namespaces
  v14, a zero false-positive selection budget, and overall selection recall
  0.956. The independent model-blind holdout is disjoint from 288,891 base
  sealed and 10,000 development physical signatures. On 60,000 unknown-typo
  negatives the production ensemble introduced zero false positives; ordinary
  trigger recall is 0.95 and Pause recall is 0.9425. The complete strict report
  has SHA-256 `c4e72a3290801dab22236a2bc381f7ba97b99f0745a078dd410e973d55d8bf52`.
  The published `intent-v1-6bf96537c28f` artifact has SHA-256
  `b22706d95e6ac942e39cd16006f4ce9c4508d98566271f202d5855d528cf1b16`.
  Two independent full retraining runs reproduced byte-identical KSLM,
  manifest and test-report files; the manifest/report SHA-256 values are
  `33b55674f6454c0b45ad55fc704b9113b5e934404ea69577ce8d7949518d2822`
  and `84b01b68e9fa186f1791828520d1b4319b39d9e6ab261a5210844cd68ba4f01e`.
- Preserve two additional non-reusable audit decisions. V12 never evaluated
  external metrics because the evaluator constructed its base exclusion index
  after merging development and could not reproduce frozen development
  provenance; `rejection-v12.json` records the exact hashes. V13 fixed domain
  separation and evaluated the fresh holdout, but ordinary triggers produced
  4 false positives among 10,000 negatives and a 0.001028128 Wilson upper
  endpoint above the 0.001 limit; `rejection-v13.json` records the decision.
- Reject the internally passing v11 artifact after the signed strict evaluator
  failed before independent external-holdout scoring: its sealed-signature
  exclusion index did not support the frozen `hunspell-unknown-*` row family.
  Preserve exact candidate, artifact, registry, manifest, report and evaluator
  hashes in `rejection-v11.json`; validate those rows against their rendered
  physical signature instead of bypassing the proof. Rotate to training config
  schema 11, split/registry/holdout v12, and a new model-blind frozen
  `unknown-typo-development-v12.json`. The v12 preseal receipt fixes 288,875
  sealed plus 10,000 development exclusions with zero holdout overlap before
  any model is loaded or evaluated.
- Enforce `selection_maximum_false_positives_per_trigger` as one aggregate
  budget jointly allocated across EN-to-RU and RU-to-EN operating curves. Add a
  regression where each direction would individually spend one false positive
  and prove that the combined selection still spends at most one.
- Rotate the signed intent candidate to v11 after preserving the exact v10
  rejection. V10 deterministically selected a common calibrated-logit margin
  of 0.9938225471937638 and passed pre-seal, but independent sealed non-pause
  recall was 0.944410276 against the 0.95 minimum; its revealed sealed rows are
  not reused for tuning. Freeze the already model-blind unknown-typo
  development corpus as `unknown-typo-development-v11.json`, bind its bytes,
  expanded-corpus and physical-signature SHA-256 values plus Hunspell
  provenance, and partition each language's 5,000 signatures under a separate
  role namespace as 3,500/500/500/500 words across
  train/development/calibration/threshold with no test role. Re-expand and
  audit all 120,000 symmetric rows, rotate the split namespace and registry,
  and preseal a model-blind v11 holdout with zero overlap against 288,902
  sealed and 10,000 development signatures.
  Serialise
  KSLM decoding behind an exact cyclic-GC guard, restore the caller's GC state
  on every path, and retain the complete strict evaluator JSON beside native
  Debian build artifacts. Remove the post-model membership-coverage and
  target-language-score vetoes that reduced certified raw recall: after hard
  guards, the calibrated trigger/direction threshold is now the only
  statistical decision, while coverage and language scores remain diagnostics.

- Add a dynamic tray-menu language action on Linux and Windows: when `RU` is
  active it offers English, and when `EN` is active it offers Russian.
- Add an optional local linear intent model on top of the precision-first
  detector: context-invariant feature schema v5 with raw-token-only signed
  character 1–5-grams, direction, length and trigger, FTRL-Proximal offline training,
  independent EN→RU/RU→EN Platt calibration on a dedicated split, jointly
  selected exact calibrated-logit thresholds for every trigger and physical
  direction, and a checksum-validated bundled int16
  KSLM schema 4 artifact
  with sorted uint64 feature-membership fingerprints and deterministic
  fallback.
- Make KSLM the sole statistical decision after user, structural and
  source-known hard guards. Use the heuristic ensemble only for short tokens,
  a disabled model or a missing artifact, so a negative model decision cannot
  be bypassed by a second positive rule and a positive calibrated decision
  cannot be weakened by an unsigned secondary veto.
- Freeze the exact Onboard EN/RU training sources and original copyright
  evidence in the repository, verify their SHA-256 in training, CI and native
  packaging, and keep the complete KSLM container bounded to 14 MiB, its
  embedded manifest to 1 MiB, its payload to 12 MiB and exact membership to
  `2^20` fingerprints.
- Gate the bundled model in CI, native Debian and Windows packaging, expose its
  version/checksum or fallback state in diagnostics, and keep ordinary input
  offline and outside model training.
- Bundle the exact frozen EN/RU Onboard language models in the native Debian
  distribution as well as Windows, prefer them over mutable system models at
  runtime, and verify both installed files byte for byte.
- Harden Windows packaging with bounded artifact/config/manifest reads, exact
  config and `SHA256SUMS` binding for frozen or staged sources, a portable
  sealed-provenance replay before Nuitka, and exact model diagnosis both before
  archiving and after silent installation.
- Harden offline training with a 2,097,152-bucket vector, up to 64 deterministic
  FTRL epochs and exact train/serve feature parity. Feature schema v5 ignores
  context, every `WordScore` and every language-model field; short-lived
  context remains only in the deterministic fallback and is not counted twice
  by the classifier. Train-only character scorers are retained only as
  checked provenance and are never classifier inputs. Use the
  `keyswitch:intent-v12:physical-signature` split namespace and quarantine
  physical signatures across splits, languages and protected safety rows.
- Restrict the optional probabilistic layer to normalized interpretations of at
  least five characters, keep shorter words on deterministic/user-rule paths,
  and enforce the identical post-augmentation limit in training and external
  unknown-typo evaluation. Record the applicability contract in signed model
  policy metadata.
- Preserve real three- and four-character bilingual Onboard collisions as a
  safety-only corpus, proving valid-source pre-model guards for every trigger
  without letting those ambiguous short tokens train the classifier.
- Select the retained epoch only on a development high-precision operating
  curve, prioritizing the full precision/recall/specificity/family-wise
  Wilson-FPR policy
  and using mean log loss only as the final tie-breaker; keep threshold data
  independent from optimization.
- Allocate the immutable 40-bucket namespace as 65/10/10/7.5/7.5 percent for
  train/development/calibration/threshold/test. A rejected 70% training
  experiment both regressed development recall and exceeded the exact-membership
  payload budget, so it is not part of the release design.
- Exclude every pre-seal candidate-quarantine signature from the separately
  built sealed test and validate candidate/test quarantine ownership in its
  original phase. Retain the consumed v2, v3 and v4 seals as rejected-run audit
  evidence. The v3 candidate passed pre-seal and the ordinary sealed slice, but
  its non-pause typo slice produced 10 false positives in 17,392 negatives
  (95% Wilson upper FPR 0.001058171), so no model was published. Rotate the
  strengthened protocol to the v4 namespace instead of reusing that test. The
  v4 candidate then passed selection with 9/23,067 ordinary and 8/17,220 typo
  false positives, but the independent sealed slice produced 14/23,090 and
  13/17,223 respectively (ordinary 95% Wilson upper FPR 0.001017564 and
  0.001291083), so it was also rejected without publication. The v5 candidate
  passed internal sealed gates, but the serving ensemble suppressed too much
  model recall and the first external protocol incorrectly treated flagged
  Hunspell stems as necessarily source-known. Keep the inspected v5 external
  corpus as development-only evidence, fix the runtime policy and methodology,
  and rotate both the internal split/seal and unseen external holdout to v6.
  Candidate v6 passed the ordinary and Pause sealed slices, but its non-pause
  typo slice produced 9 false positives among 15,812 negatives (upper 95%
  Wilson FPR 0.001081498 versus the 0.001 limit), so it was rejected without
  publication. Preserve `rejection-v6.json`, rotate to v7, and never reuse the
  revealed v6 test.
- Make pre-sealed threshold selection non-vacuous and complete: overall and
  typo precision floors of 0.9995, recall floors of 0.95/0.90, specificity
  0.999, and a 0.001 maximum family-wise 95% Wilson FPR upper bound for both.
  Correct the 12 primary selection comparisons (six triggers times overall and
  typo slices) with Bonferroni: per-comparison confidence
  0.9958333333333333 and pinned z-score 2.8652602385321333. Signed gate evidence
  records the correction, comparison count, confidence, z-score and endpoint;
  the independent sealed test keeps the ordinary 95% Wilson endpoint. Keep the
  independent sealed precision floor at 0.999 to provide explicit transfer
  headroom without weakening the closed gate.
  Training config schema 11 additionally enforces a maximum-one false-positive
  budget on every trigger's overall and typo selection slices before the
  sealed test can be materialized; this is at most 0.0045% of 22,552 negatives
  and remains subject to the strengthened family-wise Wilson bound.
  Require 0.96/0.91 overall/typo selection recall and 0.91/0.86 for Pause,
  preserving a one-percentage-point transfer reserve above the sealed floors.
  Require pause recall/typo recall of 0.90/0.85, the same Wilson bound and a 0.5
  logit margin over the strictest non-pause threshold; stop before sealed-test
  evaluation if selection is infeasible. Gate the guarded production detector
  on safety cases while retaining direct model and membership results only as
  raw diagnostics.
- Extend the pre-sealed gate to safety and selection-veto evidence, reject
  global lexicon truncation, and serialize/reload a trial KSLM before claiming
  the test namespace so numeric, quantization and payload-limit failures cannot
  consume the seal.
- Exercise the real production detector under neutral context and six reachable
  context extrema across sealed, typo, external unknown-typo, safety and
  source-known slices. Require no more total false positives than fallback or
  neutral, absolute precision/specificity/Wilson-FPR bounds, guarded-row
  reachability, and the fixed asymmetric recall policy. Use an exact-zero
  invariant for the finite safety/source-known adversarial sets instead of an
  underpowered Wilson interval.
- Store and compare the exact calibrated-logit threshold at runtime; expose its
  sigmoid-derived confidence only as diagnostics, avoiding saturated-probability
  comparisons. Certify exact classifier context invariance with fixed,
  label-independent context-stress profiles and the same per-trigger quality
  gates on threshold and sealed-test splits.
- Freeze external evaluation schema 2 to exact Hunspell dictionary/affix sizes
  and SHA-256 digests, expected lexical, unknown-typo-development and unseen
  unknown-typo-holdout corpus digests, at least 5,000 words per language, and
  the canonical list of all six runtime triggers. Build the v7 holdout without
  loading a model, under separate rank/choice namespaces, and prove zero
  overlap with 288,862 sealed and 10,000 development physical signatures.
- Avoid resorting and revalidating nearly one million already-validated uint64
  fingerprints inside the immutable KSLM constructor; cold loading of the
  current 10.4 MB artifact drops from roughly 0.7-1.0 s to 0.27-0.33 s while
  retaining payload ordering, uniqueness, checksum and corruption checks.
- Keep training config schema 11, KSLM schema 4 and external publication
  manifest schema 1 explicitly independent.
- Bind build provenance to the complete toolchain and protected-token hashes,
  candidate/full datasets, both quarantines, excluded test signatures,
  train-only scorer and Python/platform identity. Publish
  report, artifact and the final manifest commit marker as a rollback-capable
  bundle, and document a two-run byte-identical retraining procedure.
- Build candidate and sealed-test datasets in separate phases, bind the seal to
  the exact KSLM payload/runtime parameters, publish the registry atomically
  without replacement, and stop the evaluator before test construction or
  metric disclosure when provenance is invalid.

## 0.5.0 — 2026-08-28

- Add stable GitHub Release checks at startup and every six hours on Windows
  and Ubuntu, with a complete updates settings/status page on both platforms.
- On Windows, download the exact versioned Setup EXE, require its GitHub asset
  size and SHA-256 digest to match, install silently, and relaunch KeySwitch.
- Keep Ubuntu package replacement under APT authorization: notify about a new
  release and open its verified repository page instead of silently invoking
  elevated package installation.
- Cover update metadata validation, redirect restrictions, corrupted and
  interrupted downloads, state transitions and installer arguments with the
  mandatory 100% line/branch test gate.

## 0.4.0 — 2026-08-28

- Correct a likely wrong-layout word after 1.5 seconds without input, before a
  separator is typed, with a dedicated option to disable idle correction.
- Render Windows country flags at the maximum notification-area icon size
  without the former purple frame.
- Add an EveryLang-style learning prompt after a `Pause/Break` manual
  conversion. `Enter` immediately confirms the word as a switching rule,
  while Escape, unrelated input or an eight-second timeout dismisses it.
- Position the prompt at the text caret when the platform exposes it, with a
  pointer fallback on X11, and restore the original Windows target after the
  focused prompt closes.
- Install the AT-SPI bus runtime on Linux and fall back safely when the desktop
  accessibility service is disabled or unavailable.
- Keep repeated manual conversions as a configurable fallback and retain the
  strict 100% line/branch coverage gate for the expanded core and GTK UI.

## 0.3.0 — 2026-08-27

- Add a native Windows x64 backend using `WH_KEYBOARD_LL`, `ToUnicodeEx`,
  `WM_INPUTLANGCHANGEREQUEST` and `SendInput` without clipboard replacement.
- Preserve manual EN/RU layout choices and the same correction, manual-convert,
  undo, learning, context and exclusion semantics on both supported platforms.
- Add a Windows settings application with nine sections, live diagnostics,
  history, a real test field and complete controls for the shared settings.
- Add `.exe` file picking, active-window targeting and registered App Paths for
  application exclusions on Windows.
- Add per-user Windows logon autostart and a dynamic notification-area icon in
  either `EN/RU` or country-flag style; either mouse button opens the full menu.
- Prevent duplicate Windows instances and focus the existing KeySwitch window
  when the executable is launched again.
- Compile a source-free Windows standalone distribution with pinned Nuitka and
  publish both an Inno Setup installer and a portable ZIP.
- Pin native builds to stable Nuitka 4.2, which officially supports the Python
  3.14 runtime shipped by the current Ubuntu release.
- Bundle licensed Onboard EN/RU language models and all applicable runtime
  license texts in the Windows distribution.
- Add a real bidirectional Windows hook/`SendInput`/Tk correction E2E and make
  tag releases wait for both Debian and Windows artifacts before publishing
  unified checksums.
- Make silent installer tests verify that uninstall removes the application
  files and its per-user autostart registry value.
- Keep the platform-neutral Python contracts under the enhanced strict mypy
  profile and retain the 100% line/branch unit coverage gate.

## 0.2.1 — 2026-08-27

- Respect a manual XKB layout change by leaving the next completed word
  unchanged, with an opt-out switch in the autocorrection settings.

## 0.2.0 — 2026-08-27

- Enable XDG Autostart by default and keep its desktop entry synchronized.
- Add active-window capture, installed-application search and removable rows
  for application exclusions.
- Add a live tray layout indicator with selectable `EN/RU` and country-flag
  styles.
- Add a native left-click DBusMenu with settings, correction toggles, history,
  exclusions, about and quit actions.
- Replace exact-word-only detection with a precision-first ensemble of
  frequency lexicons, Hunspell morphology and smoothed character n-grams.
- Add per-application short-term language context, technical-token guards and
  conservative handling of ambiguous words.
- Learn explicit rules after repeated manual conversions and remember an
  automatic correction rejected with Undo; ordinary typed text is not stored.
- Correct words whose physical letter keys are punctuation in the other
  layout, including `,fpf` to `база`, while preserving punctuation boundaries.
- Add a reproducible 40,000-word detector benchmark with strict CI quality
  gates.
- Enforce an enhanced `mypy --strict` profile across application code, tests
  and developer tools, including typed GTK, D-Bus and ctypes boundaries.
- Compile the application with pinned Nuitka into an architecture-specific
  ELF Debian package without application Python source or bytecode files.
- Add package-structure, dynamic-link and packaged-native X11/DBusMenu E2E
  gates to both test and release workflows.

## 0.1.0 — 2026-08-26

- First public release for Ubuntu X11.
- Automatic bidirectional correction between English and Russian layouts.
- Manual conversion, correction undo and global pause hotkeys.
- GTK 4 and Libadwaita settings window with eight configuration pages.
- Application and word exclusions, local correction history and diagnostics.
- XDG autostart and StatusNotifierItem desktop integration.
- Reproducible `.deb` package build and tag-driven GitHub Release workflow.

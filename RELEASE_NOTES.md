# KeySwitch 0.13.0

## Русский

Выпуск по итогам разбора реального журнала: замена больше не смешивается с
набором, обучение перестало запоминать колебания, короткие русские слова
наконец исправляются, а выученные правила видны в окне. Сертифицированная
модель намерения `intent-v1-6ece07f881ec` не изменилась.

- Замена больше не сталкивается с тем, что в неё печатают. Раскладка
  переключается первой, а удаление и перенабор уходят одним вызовом
  `SendInput`, который Windows не перемешивает с другим вводом; клавиши,
  пришедшие после слова — до того, как движок до него добрался, или пока он
  переписывал текст, — удаляются вместе со словом и набираются заново уже в
  новой раскладке, а нажатия во время самой вставки хук придерживает и
  допечатывает последними. В журнале у половины замен стояло
  `keys_during_injection > 0` — отсюда и брались лишние и потерянные символы.
  В X11 поздние клавиши обрабатываются так же; удерживать ввод XRecord не
  умеет. `correction_applied` сообщает `late_keys` и `held_keys`.
- `Pause` сразу после автозамены теперь записывает запрет этой замены — как
  горячая клавиша отмены, — а не учит правило «обратно»: в журнале была ложная
  замена, которую уже дважды чинили вручную, и она сработала в третий раз.
  Переключение слова `Pause` туда-обратно больше не считается подтверждениями:
  три переключения делали из опечатки действующее правило, хотя подсказку
  отклонили.
- Короткое слово, найденное только в частотном словаре другого языка,
  остаётся на месте, если в своём языке оно читается как обычный текст, а
  предыдущее слово не говорит за смену: «дев» после «на» превращалось в
  «ltd». Порог — тот же, что в ветке неизвестных слов; живёт в слое политики,
  сертифицированный детектор не тронут.
- Самые частые русские служебные слова, набранные в латинской раскладке,
  наконец исправляются: `yt`, `gj`, `yf` («не», «по», «на») сами по себе, `kb`
  и `nj` («ли», «то») после русского слова, одиночные «а», «и», «с», «в», «к»,
  «у», «о», «я» после русского слова. Короткое слово из безопасного списка
  никогда не превращает клавишу, которая в одной раскладке знак, а в другой
  буква, в границу слова — «общих» не режется на «о» и запятую.
- На странице «Обслуживание» видно, что запомнило локальное обучение:
  набранное слово, во что оно превращается, направление, сколько подтверждений
  набрано и сколько нужно, и запреты после отмены.
- Журнал называет замену, вытесненную границей слова или отменой, а раскладку
  собственного окна KeySwitch отмечает один раз за визит, а не каждым опросом —
  эта строка занимала пятнадцать процентов сеанса.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.13.0-x64.exe` или переносимый
  `KeySwitch-0.13.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.13.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release driven by a real session log: corrections no longer collide with
typing, learning stops remembering hesitation, short Russian words are finally
corrected, and learned rules are visible in the window. The certified intent
model `intent-v1-6ece07f881ec` is unchanged.

- A correction no longer collides with what is typed into it. The layout is
  switched first and the deletion and the replacement travel in one
  `SendInput` call, which Windows keeps together; keys that arrived after the
  word — typed before the engine got to it, or while it was replacing the
  text — are deleted with the word and typed again in the new layout, and keys
  pressed during the injection are held by the hook and typed last. Half of
  the corrections in the log carried `keys_during_injection > 0`; that is
  where the extra and missing characters came from. On X11 late keys are
  handled the same way; XRecord cannot withhold input. `correction_applied`
  reports `late_keys` and `held_keys`.
- `Pause` right after an automatic correction records a rejection of that
  correction, exactly as the undo hotkey does, instead of teaching a rule for
  the way back: the log showed a false correction fixed by hand twice that
  still fired a third time. Toggling a manual conversion back and forth with
  `Pause` no longer counts as confirmations: three toggles used to turn a typo
  into an active rule while the prompt had been dismissed.
- A short word found only in the other language's frequency list is left
  alone when it reads as ordinary text in the language it was typed in and the
  previous word does not favour the switch: `дев` after `на` was traded for
  `ltd`. The bar is the one the unknown-word branch already applies; it lives
  in the policy layer, the certified detector is unchanged.
- The most frequent Russian function words typed in the Latin layout are
  corrected at last: `yt`, `gj`, `yf` (`не`, `по`, `на`) on their own, `kb` and
  `nj` (`ли`, `то`) after a Russian word, and the single letters `а`, `и`,
  `с`, `в`, `к`, `у`, `о`, `я` after a Russian word. A trusted short word never
  turns a key that is punctuation in one layout and a letter in the other into
  a word boundary, so `общих` is not cut into `о` and a comma.
- The maintenance page lists what local learning remembers: the typed word,
  what it becomes, the direction, the confirmations gathered and required, and
  the rejections made by undoing.
- The log names a correction superseded by a boundary or replaced by an undo,
  and reports the layout of KeySwitch's own window once per visit instead of on
  every poll — that line was fifteen percent of a session.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.13.0-x64.exe` or the portable
  `KeySwitch-0.13.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.13.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

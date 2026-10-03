# KeySwitch 0.37.0

## Русский

Ранняя смена раскладки снова включена по умолчанию: раскладка меняется уже по первым буквам слова
(`ghbd` → `прив`), не дожидаясь пробела. Решают новые модели — контекстная context-v3 и модель
префиксов prefix-v2. На запечатанной проверке они в обоих режимах, с ранней сменой и без неё, не
испортили ни одной правильно набранной строки (прежние модели — от одной до трёх) и восстановили
больше строк, набранных в чужой раскладке. Кроме того, буква, стёртая `Backspace`, больше не
исправляется паузой. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.37.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.37.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.37.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.37.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.37.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Ранняя смена раскладки

- Включена по умолчанию и выключается одним переключателем «Ранняя смена раскладки» в
  настройках. Раскладка меняется с четвёртой буквы слова; это число тоже настраивается. Когда
  модель префиксов сомневается, она ждёт конца слова, и слово решается как раньше — целиком.
- Файл настроек прежних выпусков хранит выключенную раннюю смену как их значение по умолчанию, и
  отличить его от выбора человека нельзя. Поэтому при первом запуске 0.37.0 ранняя смена
  включится и там. Если вы выключали её сами, выключите ещё раз: дальше выбор сохраняется.

### Модели

- Контекстная модель `context-v3-7d8e1c9b2d87` и модель префиксов `prefix-v2-bf3dc28f8567`
  заменили context-v1 и prefix-v1. Их обучали на корпусе из публичных источников и проверили на
  отдельной запечатанной выборке, которую при обучении никто не видел.
- Слово из трёх букв, у которого оба прочтения — настоящие слова (`tot` и `еще`, `зум` и `pev`),
  модель откладывает до следующего слова и решает вместе с ним: `tot ghbdtn` становится
  `еще привет`. Слово с одним осмысленным прочтением решается сразу: `rjn` → `кот`.
- Установленные модели теперь держатся порогов качества, которые сдвигаются только вверх: модель
  не попадёт в выпуск, если портит правильный текст или восстанавливает меньше, чем установленная.

### Текст

- Буква, стёртая `Backspace`, больше не исправляется паузой. После `rjn`, `Backspace` и паузы
  оставшиеся `rj` превращались в `кт`, будто слово на этом закончено. Теперь стирание считается
  правкой: пауза не отсчитывается, пока не набрана следующая буква.

### Что стало хуже

Новая пара моделей решает четыре случая иначе, чем прежняя. Текст при этом не портится: слово
остаётся таким, как набрано, и его можно исправить клавишей `Pause`.

- `jr.` перед русским словом в начале строки остаётся `jr.` (раньше становилось `ок.`).
- `ша` (английское `if` в русской раскладке) после русского слова не становится `if` сразу на
  пробеле: модель ждёт следующего слова.
- `b/bkb` остаётся как набрано (раньше становилось `и/или`): слова через косую черту модель
  оставляет, когда не уверена.
- `ghbdtn@` остаётся как набрано (раньше становилось `привет"`): со знаком `@` модель читает его
  как часть адреса.

## English

The early layout switch is on by default again: the layout changes after the first letters of a
word (`ghbd` → `прив`) instead of waiting for the space. The decision comes from new models, the
context-v3 context model and the prefix-v2 prefix model. On a sealed test, in both modes, with
the early switch and without it, they corrupted no correctly typed row (the previous models
corrupted one to three) and restored more rows typed in the wrong layout. Besides, a letter
erased with `Backspace` is no longer corrected by a pause. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.37.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.37.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.37.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.37.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.37.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Early layout switch

- On by default, and one switch, "Early layout switch", turns it off in the settings. The
  layout changes from the fourth letter of a word, and that number is a setting too. When the
  prefix model is unsure, it waits for the end of the word, which is then decided whole, as
  before.
- A settings file of an earlier release stores the early switch as off, their default, and that
  cannot be told apart from a person's choice. So the first start of 0.37.0 turns the early
  switch on there too. If you had turned it off yourself, turn it off once more: the choice holds
  from then on.

### Models

- The context model `context-v3-7d8e1c9b2d87` and the prefix model `prefix-v2-bf3dc28f8567`
  replace context-v1 and prefix-v1. They were trained on a corpus built from public sources and
  checked on a separate sealed sample that no step of the training saw.
- A three-letter word whose two readings are both real words (`tot` and `еще`, `зум` and `pev`)
  is held until the next word and decided together with it: `tot ghbdtn` becomes `еще привет`. A
  word with one meaningful reading is decided at once: `rjn` → `кот`.
- The installed models are now held to quality floors that only move up: a model does not reach
  a release if it corrupts correct text or restores less than the installed one.

### Text

- A letter erased with `Backspace` is no longer corrected by a pause. After `rjn`, `Backspace`
  and a pause, the `rj` left behind became `кт`, as if the word had ended there. Erasing now
  counts as editing: the pause does not run until the next letter is typed.

### What got worse

The new pair decides four cases differently from the previous one. No text is corrupted: the
word stays as typed, and `Pause` corrects it.

- `jr.` before a Russian word at the start of a line stays `jr.` (it used to become `ок.`).
- `ша` (the English `if` typed in the Russian layout) after a Russian word no longer becomes `if`
  at the space: the model waits for the next word.
- `b/bkb` stays as typed (it used to become `и/или`): the model leaves slash-joined words alone
  when it is unsure.
- `ghbdtn@` stays as typed (it used to become `привет"`): with the `@` sign the model reads it as
  part of an address.

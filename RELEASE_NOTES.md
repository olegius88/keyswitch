# KeySwitch 0.36.1

## Русский

Самообучение заменено правилами переключения, которые появляются только по явному подтверждению,
а ввод через удалённый доступ (TeamViewer, AnyDesk) больше не задерживает и не теряет Enter. Полный
перечень — в [CHANGELOG.md](CHANGELOG.md).

0.36.0 не был опубликован: на медленной сборочной машине оба нажатия двойного `Pause` успевали встать в
очередь до первой замены, второе считалось неизвестным вводом после слова, и замена не выполнялась. Теперь
нажатие клавиши преобразования в очереди замену не останавливает, а полсекунды на второе нажатие
отсчитываются от момента, когда слово уже исправлено. 0.36.1 содержит все изменения 0.36.0 и это исправление.

### Файлы выпуска

- `KeySwitch-Setup-0.36.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.36.1-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.36.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.36.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.36.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Правила переключения

Раньше после каждого ручного преобразования по `Pause` появлялась подсказка, и `Enter` сразу добавлял
правило — в том числе `Enter`, которым собирались отправить сообщение, — а отмена исправления молча
записывала запрет. Правила появлялись без ведома пользователя, и понять, откуда они взялись, было нельзя.

Теперь правило появляется только по кнопке OK:

1. Сразу после слова, с которым программа поступила не так, дважды нажмите `Pause`. Слово станет
   противоположным тому, что сделала программа: неверное исправление отменится, пропущенное выполнится.
2. Над местом ввода появится вопрос «Добавить правило переключения?». `Enter` открывает окно правила,
   `Esc` закрывает вопрос.
3. Окно уже заполнено: буквы, как они набраны, условие «совпадать с данным сочетанием» и действие —
   «Не переводить слово в другую раскладку» для отменённого исправления, «Переводить» для пропущенного.
   Можно выбрать, что слово должно содержать сочетание букв или начинаться с него, учитывать ли
   регистр, и поправить сами буквы.
4. OK или `Enter` сохраняет правило, «Отмена» или `Esc` — нет.

Одиночное нажатие `Pause` и отмена по `Ctrl+Alt+Z` по-прежнему исправляют текст, но больше ничему не
учат. Правила важнее решения модели; их список с кнопками «Добавить…», «Изменить…», «Удалить
выбранное» и «Удалить все» — на странице «Обслуживание» (в Linux — в разделе «Автокоррекция»). Прежние правила переносятся при первом запуске:
действовавшее правило становится правилом «переводить», запрет — правилом «не переводить», неподтверждённые
счётчики отбрасываются, а старый файл сохраняется рядом как `learning.json.v2-backup`. Настройка
«Подтверждений для правила» убрана.

### Удалённый доступ

Компьютер, которым управляют через TeamViewer, получает набранный текст как нажатия, присланные другой
программой, и большую часть — символами, без отпускания клавиш. KeySwitch на нём ждал этих отпусканий:
каждое `Enter` уходило примерно через 2 секунды, а щелчок мышью за это время его отменял. Кроме того,
`Pause`, нажатый на управляющем компьютере, переключал раскладку и на управляемом.

Теперь в Windows нажатия, присланные другой программой (удалённый доступ, макросы, экранная клавиатура),
по умолчанию не считаются набором: KeySwitch не исправляет слова из них, не отвечает на их горячие
клавиши и не задерживает их `Enter`. Если на управляющем компьютере тоже стоит KeySwitch, слова исправляет
он. Прежнее поведение возвращает настройка «Ввод от других программ».

### Что стало хуже

Если на управляющем компьютере KeySwitch не установлен, на управляемом он больше не исправляет текст,
набранный через удалённый доступ, пока не включена настройка «Ввод от других программ». Символы, которые
удалённый доступ присылает вместо клавиш, не исправляются и с ней: их нельзя перепечатать в другой
раскладке. Двойное нажатие `Pause` быстрее половины секунды больше не возвращает слово обратно, а открывает
предложение правила; чтобы вернуть слово, нажмите `Pause` ещё раз.

## English

Self-learning is replaced by switching rules that exist only once they are confirmed, and typing
through remote control (TeamViewer, AnyDesk) no longer delays or drops Enter. The full list is in
[CHANGELOG.md](CHANGELOG.md).

0.36.0 was not published: on the slow build machine both presses of a double `Pause` were queued before
the first conversion ran, the second counted as unknown input after the word, and the conversion did not
happen. A queued press of the conversion key no longer stops the conversion, and the half second for the
second press starts once the word is corrected. 0.36.1 contains every change of 0.36.0 and this fix.

### Release files

- `KeySwitch-Setup-0.36.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.36.1-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.36.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.36.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.36.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Switching rules

Before, every manual conversion with `Pause` showed an offer that `Enter` accepted at once - including an
`Enter` meant to send a message - and undoing a correction silently stored a ban. Rules appeared without the
user knowing, with no way to tell where they came from.

Now a rule exists only after OK:

1. Right after a word the program handled wrongly, press `Pause` twice. The word becomes the opposite of
   what the program did: a wrong correction is undone, a missed one is made.
2. A prompt above the caret asks "Добавить правило переключения?" (add a switching rule?). `Enter` opens
   the rule window, `Esc` closes the prompt.
3. The window is already filled in: the letters as typed, the condition "matches the letters" and the
   action - "keep the word" for an undone correction, "convert the word" for a missed one. The word may
   instead have to contain the letters or start with them, case may count, and the letters can be edited.
4. OK or `Enter` stores the rule, Cancel or `Esc` does not.

A single `Pause` and the undo with `Ctrl+Alt+Z` still fix the text but teach nothing. Rules outrank the
model's decision; they are listed with Add, Edit, Remove and Remove all buttons on the Maintenance page (on
Linux, in the automatic correction section).
Earlier rules are carried over on first start: a rule that acted becomes a "convert" rule, a ban a "keep"
rule, unconfirmed counters are dropped, and the old file is kept as `learning.json.v2-backup`. The
"confirmations for a rule" setting is gone.

### Remote control

A computer controlled through TeamViewer receives the typing as keys another program sends, most of them as
characters without key releases. KeySwitch there waited for those releases: every `Enter` went out about two
seconds late, and a click during the wait dropped it. A `Pause` pressed on the controlling computer also
switched the layout of the controlled one.

Now on Windows keys another program sends (remote control, macros, on-screen keyboards) are not treated as
typing by default: KeySwitch neither corrects words made of them, nor answers their hotkeys, nor holds their
`Enter` back. When the controlling computer runs KeySwitch too, that copy corrects the words. The "Ввод от
других программ" (input from other programs) setting restores the previous behaviour.

### What got worse

Without KeySwitch on the controlling computer, the controlled one no longer corrects text typed through
remote control unless the "input from other programs" setting is on. Characters remote control sends instead
of keys are not corrected even then: they cannot be typed again in another layout. Pressing `Pause` twice
within half a second no longer converts the word back but offers a rule; press `Pause` once more to
convert it back.

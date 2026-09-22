# KeySwitch 0.29.0

## Русский

Три исправления, найденные в журналах реального набора: буквы, дописанные к слову,
теперь оцениваются вместе с ним; отменённая замена больше не разрывает следующее
слово; очистка выученных правил больше не стирает запреты. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.29.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.29.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.29.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.29.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.29.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Дописанные буквы относятся к слову

Если стереть пробел после слова и продолжить набор, дописанные буквы раньше
становились отдельным словом. «создаш», Backspace и «ь» оценивались как одинокое
«ь» — его не знает ни один словарь, и модели превращали его в «m». Так же «виде»,
два Backspace и «ть» давали попытку заменить «ть».

Теперь один Backspace по пробелу, завершившему слово, снова делает это слово
текущим: дописанные буквы оцениваются вместе с ним, следующий Backspace стирает
буквы самого слова, а Pause преобразует его целиком. Само по себе удаление пробела
не вызывает повторной оценки слова — её запускает только новая буква. Слово не
открывается заново после перемещения курсора, второго пробела, знака препинания
перед пробелом и после Enter.

### Отменённая замена не разрывает следующее слово

Замена слова выполняется, когда отпускается клавиша пробела, а к этому моменту
следующее слово часто уже начато. Если поле ввода к тому времени менялось, замена
отменялась, уже набранные буквы следующего слова терялись для программы, а остаток
этого слова оценивался отдельно — так «все» превращалось в «вct». Теперь остаток
такого слова до его собственной границы не трогается.

### Очистка правил не стирает запреты

Запрет запоминается, когда вы отменяете ложное исправление, — это ваше «не трогай
это слово». Раньше очистка самообучения удаляла вместе с правилами и запреты, и
отменённые исправления после каждой очистки возвращались. Теперь правила и запреты
удаляются по отдельности: в окне настроек Windows и macOS — кнопками «Очистить
правила» и «Очистить запреты», там же можно удалить одну выбранную строку; в Linux —
вариантами «Только правила», «Только запреты» и «Всё».

## English

Three fixes found in logs of real typing: letters added to a word are now judged with
it, an abandoned correction no longer splits the next word, and clearing learned rules
no longer removes the rejections. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.29.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.29.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.29.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.29.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.29.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Added letters belong to the word

Deleting the space after a word and typing on used to turn the added letters into a
word of their own. "создаш", a Backspace and "ь" were judged as a lone "ь", which no
dictionary knows and which the models turned into "m"; "виде", two Backspaces and "ть"
likewise led to an attempt to replace "ть".

One Backspace over the space that ended a word now makes that word the current one
again: the added letters are judged with it, a further Backspace keeps erasing its
letters, and Pause converts it whole. Deleting the space alone does not get the word
judged a second time; only a new letter does. A word is not reopened after a caret
move, a second space, punctuation before the space, or Enter.

### An abandoned correction leaves the next word alone

A correction runs when the space key comes up, and by then the next word has often
begun. When the input field had changed in the meantime, the correction was abandoned,
the letters of the next word already typed were lost to the program, and the rest of
that word was judged on its own — so "все" could become "вct". The rest of such a word
is now left untouched until its own boundary.

### Clearing rules keeps the rejections

A rejection is recorded when you undo a false correction: it is your "leave this word
alone". Clearing local learning used to remove the rejections together with the rules,
so undone corrections came back after every clearing. Rules and rejections are now
removed separately: in the Windows and macOS settings with "Очистить правила" and
"Очистить запреты", where a single selected entry can also be deleted; on Linux with
"Только правила", "Только запреты" and "Всё".

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

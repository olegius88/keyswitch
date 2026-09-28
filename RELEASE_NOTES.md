# KeySwitch 0.34.0

## Русский

Первые слова строки теперь исправляются вместе со следующим словом: если оно показывает, что всё
набрано в чужой раскладке, одна правка берёт и его, и слова перед ним. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.34.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.34.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.34.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.34.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.34.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Первые слова строки

Первым словам сообщения не на что опереться: `руку` — русское слово, `here` — английское, и
`руку ерун`, набранное в русской раскладке, раньше выходило «руку they». Теперь слово, оставленное на
пробеле как набрано, запоминается. Когда следующее слово, набранное в той же раскладке, исправляется,
программа спрашивает контекстную модель о предыдущем ещё раз — уже с исправленным соседом. Если модель
его исправляет, одна правка берёт оба: «here they». Так же — ещё на одно слово назад:
`рун руку ерун` → «hey here they». Слово, которое кончается буквой на клавише знака, спрашивается
целиком: ``dc` njn`` → «всё тот», ``t` vfnm`` → «её мать». Знак на конце исправленного соседа
остаётся знаком: `tot ghbdtn,` → «еще привет,».

Это касается только первых слов строки: перед ними в строке нет ни одной буквы (пустое поле, новая
строка, `//` или `-` в начале). Слово посреди фразы уже решалось вместе с текстом перед ним, а верно
набранное русское слово перед термином в чужой раскладке выглядит для модели так же: в
`склонируй репо пшерги` переспрос сделал бы из «репо» `htgj`. Переспрашивается только слово, другое
прочтение которого — слово: термин, набранный как задумано, перед русским словом в английской раскладке
(`htop gjrfpsdftn`) остаётся `htop`, хотя с «показывает» после него модель перевела бы и его. Токены без
букв (`.`, `1.2`) не берутся никогда. Контекстная модель та же, что в 0.33.3.

На 5 000 предложениях субтитров десяти раскрытых наборов, набранных через программу в пяти видах
полей правильно и в чужой раскладке, исправленных стало 24 805 вместо 24 258 из 25 000, испорченных
правильных — по-прежнему 55; правка внутри слова не изменилась: 24 920 исправленных, ни одного
испорченного. На 500 новых предложениях, которых не видела ни одна модель: исправленных 2 473 вместо
2 419 из 2 500, испорченных по-прежнему 4; внутри слова 2 490 и 2 490, ни одного испорченного.

## English

The first words of a line are now corrected together with the next word: when it shows that all of
them were typed in the other layout, one correction takes it and the words before it. The full list
is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.34.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.34.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.34.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.34.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.34.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### The first words of a line

The first words of a message have nothing before them to go by: "руку" is a Russian word and
"here" an English one, so `руку ерун` typed in the Russian layout used to end as "руку they". A word
left as typed at a space is now remembered. When the next word, typed in the same layout, is
corrected, the program asks the context model about the word before it once more, with the corrected
word after it. If the model corrects it too, one correction takes both: "here they". The same goes
one more word back: `рун руку ерун` → "hey here they". A word that ends in a letter on a punctuation
key is asked about whole: ``dc` njn`` → "всё тот", ``t` vfnm`` → "её мать". A sign at the end of the
corrected word stays a sign: `tot ghbdtn,` → "еще привет,".

Only the first words of a line are taken along: no letter stands before them on their line (an empty
field, a new line, `//` or `-` at its start). A word in the middle of a phrase was already decided
with the text before it, and a Russian word typed as intended before a term typed in the other layout
looks the same to the model: asked again, "склонируй репо пшерги" would turn "репо" into `htgj`.
Only a word whose other reading is a word is asked again: a term typed as intended before a Russian
word typed in the English layout (`htop gjrfpsdftn`) stays `htop`, though with "показывает" after it
the model would convert it too. Tokens without a letter (`.`, `1.2`) are never taken along. The
context model is the one from 0.33.3.

On 5,000 subtitle sentences of ten disclosed sets typed through the program into five kinds of
field, correctly and in the other layout, restored sentences went from 24,258 to 24,805 of 25,000,
with the same 55 spoiled correct ones; letters put back inside a word are unchanged: 24,920
restored, none spoiled. On 500 new sentences no model had seen: restored 2,473 instead of 2,419 of
2,500, spoiled still 4; inside a word 2,490 and 2,490, none spoiled.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

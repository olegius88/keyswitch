# KeySwitch 0.31.1

## Русский

Сомнительное слово теперь ждёт соседа дольше: «tot», пауза на раздумье и «ghbdtn» через
несколько секунд дают «еще привет», а не «tot привет». Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.31.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.31.1-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.31.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.31.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.31.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Ожидание соседа не обрывается через 10 секунд

С 0.31.0 слово, в котором программа сомневается («tot» может быть и английским словом, и
«еще» в английской раскладке), ждёт следующего слова и решается вместе с ним. Но ожидание
длилось всего 10 секунд. Если между словами была пауза подольше, «tot» так и оставалось, а
следующее «привет» исправлялось само по себе.

Теперь слово ждёт столько же, сколько программа помнит соседний текст, — 45 секунд. Как и
раньше, ожидание сразу заканчивается при переходе в другое окно, перемещении курсора,
Backspace или изменении текста в поле. Модели те же, что в 0.31.0.

### Внутреннее: все числа в коде названы

Каждое число в коде, тестах и инструментах, кроме 0 и 1, теперь именованная константа,
объявленная один раз там, где ей место по смыслу. Поведение программы от этого не меняется;
это защищает от ошибок вроде той, что исправлена выше, когда два связанных срока жили в коде
двумя несвязанными числами. Исключение пока — файлы, чьи байты закреплены в печатях
выпущенных моделей: они будут приведены к тому же правилу, когда эти модели обучат и
проверят заново.

## English

A word in doubt now waits longer for its neighbour: "tot", a pause to think and "ghbdtn" a
few seconds later give "еще привет" rather than "tot привет". The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.31.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.31.1-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.31.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.31.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.31.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Waiting for the neighbour no longer ends after 10 seconds

Since 0.31.0 a word the program is in doubt about ("tot" may be the English word or "еще"
typed in the English layout) waits for the next word and is decided together with it. The
wait lasted only 10 seconds, though. With a longer pause between the words, "tot" stayed
and the following "привет" was converted on its own.

A word now waits as long as the program remembers the text next to it: 45 seconds. As
before, the wait ends at once on another window, a caret move, Backspace or a change in the
field. The models are the same as in 0.31.0.

### Internal: every number in the code has a name

Every number in the code, the tests and the tools other than 0 and 1 is now a named constant,
declared once where it belongs. The program behaves exactly as before; this guards against
mistakes like the one fixed above, where two related limits lived in the code as two unrelated
numbers. The exception for now is the files whose bytes are pinned by the seals of the shipped
models: they follow the same rule once those models are trained and checked again.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

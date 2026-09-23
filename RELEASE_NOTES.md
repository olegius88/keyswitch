# KeySwitch 0.31.0

## Русский

Слово, в котором программа сомневается, теперь решается вместе со следующим словом:
«еще», набранное в английской раскладке в начале сообщения, больше не остаётся «tot», а
«tot d » становится «еще в ». Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.31.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.31.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.31.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.31.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.31.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Сомнительное слово решается вместе со следующим

«еще», набранное в английской раскладке, — это «tot», тоже английское слово. В начале
сообщения контекстная модель просит дождаться следующего слова, но программа ждала только
слова из одной-двух букв, и «tot» оставалось, даже когда следующее «тест» исправлялось.

Теперь:

- слово ждёт соседа при любой длине, если модель об этом просит;
- слово, которое модель лишь предлагает исправить, тоже ждёт соседа и решается заново,
  когда тот набран. На паузе оно не пересматривается: пауза не даёт нового контекста;
- если следующее слово не исправилось, модель спрашивают о нём ещё раз так, будто перед
  ним другое прочтение ожидающего слова: «d» после «tot» — одинокая буква, после «еще» —
  «в». Пара исправляется, только если модель исправляет оба слова: «tot d » становится
  «еще в », а «tot is » остаётся.

Контекстная модель дообучена на таких парах. Каждый пример «слово, за ним исправленное
русское слово» стоит рядом с тем же словом без продолжения, чтобы решение определяло
только следующее слово.

Проверка на 500 новых фразах из субтитров фильмов (300 русских и 200 английских), каждая
набрана через программу в пяти видах полей:

- набранное не в той раскладке новая версия исправила в 2 304 фразах, 0.30.0 — в 2 216;
- правильно набранный текст новая версия испортила в 24 фразах из 2 500, 0.30.0 — в 26;
- ни одной фразы, которую 0.30.0 обрабатывал правильно, новая версия не испортила.

Больше всего выиграли фразы, начинающиеся с короткого слова в чужой раскладке: «ш» вместо
«i», «еру» вместо «the», «ye» вместо «ну».

После кода или почти целиком английского текста «tot» по-прежнему остаётся «tot»: там
модель считает его английским словом. Pause исправляет его.

## English

A word the program is in doubt about is now decided together with the next word: "еще"
typed in the English layout at the start of a message no longer stays "tot", and "tot d "
becomes "еще в ". The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.31.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.31.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.31.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.31.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.31.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### A word in doubt is decided with the next one

"еще" typed in the English layout is "tot", an English word too. At the start of a message
the context model asks to wait for the next word, but the program only waited for words of
one or two letters, so "tot" stayed even when the next word, "тест", was converted.

Now:

- a word waits for its neighbour whatever its length when the model asks for it;
- a word the model only suggests converting waits for its neighbour as well and is decided
  again once the neighbour is typed. It is not decided again at a pause: a pause brings no
  new context;
- when the next word is not converted, the model is asked about it once more as if the
  waiting word's other reading stood in front of it: "d" after "tot" is a lone letter,
  after "еще" it is "в". The pair converts only if the model converts both words: "tot d "
  becomes "еще в ", and "tot is " stays.

The context model is retrained on such pairs. Each example of a word followed by a converted
Russian word sits next to the same word followed by nothing, so only the next word decides.

Checked on 500 new sentences from film subtitles (300 Russian, 200 English), each typed
through the program in five kinds of fields:

- the new version restored text typed in the wrong layout in 2,304 sentences, 0.30.0 in 2,216;
- the new version spoiled correctly typed text in 24 sentences out of 2,500, 0.30.0 in 26;
- no sentence that 0.30.0 handled correctly came out wrong.

The largest gain is in sentences that open with a short word typed in the wrong layout: "ш"
for "i", "еру" for "the", "ye" for "ну".

After code or mostly English text "tot" still stays "tot": there the model takes it for the
English word. Pause converts it.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

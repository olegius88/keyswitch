# KeySwitch 0.31.2

## Русский

«tot привет» теперь становится «еще привет» и в Firefox, Edge, WhatsApp, Discord и других
приложениях, а кавычка в Telegram сразу превращается в «@» для упоминания. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.31.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.31.2-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.31.2_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.31.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.31.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Сомнительное слово решается вместе с соседом в любом приложении

Слово, в котором программа сомневается («tot» — это и английское слово, и «еще» в английской
раскладке), ждёт следующего слова. Когда «ghbdtn» становится «привет», контекстная модель ещё
раз оценивает «tot» уже с «привет» после него и уверенно отвечает «перевести». Но модель знает
по имени лишь несколько приложений, и в остальных её ответ отбрасывался: в Firefox, Edge,
WhatsApp, Discord, Slack «tot привет» так и оставалось. Теперь ответ модели в этом случае
принимается везде. Пара решается вместе и тогда, когда перед пробелом после второго слова была
пауза.

Проверка на 500 новых предложениях из субтитров, которые до этого никто не видел: ни одного
испорченного правильного предложения; в обычных приложениях восстановлено 2372 предложения
вместо 2352, при наборе в Firefox — 2212 вместо 2098.

### Кавычка в Telegram сразу становится «@»

Кавычка, набранная в русской раскладке в начале текста или после пробела, сразу показывается
как «@», чтобы открылся список участников; раскладка не меняется. Если набирать дальше,
«@» снова становится кавычкой, и дальше решает слово: «ощрт» даст «@john», русское слово
останется цитатой. Кавычка вплотную к тексту, например `привет,"`, — закрывающая и не
меняется. Pause сразу после «@» возвращает кавычку. Отключается настройкой «Telegram:
кавычка в начале слова — это @».

### Одинаковые диапазоны настроек в обоих окнах

- Порог уверенности — от 0,5 до 10 с шагом 0,1 (в окне Linux было до 8 с шагом 0,5).
- Подтверждений для правила — до 10 (в окне Linux было до 5).
- Пауза — от 0,3 до 5 секунд, так же и в самом движке.
- Выбор приложения-исключения ждёт 3 секунды в обоих окнах (в окне Linux было 2,5).

Значение, вписанное в файл настроек вручную, приводится к этому же диапазону.

### Внутреннее: каждое значение объявлено один раз

Все числа и общие значения программы собраны в одну папку модулей по назначению, и каждое
объявлено ровно один раз, в том числе числа внутри текстов интерфейса. Проверка в наборе
тестов не даёт завести второе имя для того же значения. Исключение пока — файлы, чьи байты
закреплены в печатях выпущенных моделей.

## English

"tot привет" now becomes "еще привет" in Firefox, Edge, WhatsApp, Discord and other
applications too, and the quote in Telegram turns into "@" at once for a mention. The full
list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.31.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.31.2-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.31.2_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.31.2-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.31.2-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### A word in doubt is decided with its neighbour in every application

A word the program is in doubt about ("tot" is an English word and also "еще" typed in the
English layout) waits for the next word. When "ghbdtn" becomes "привет", the context model is
asked about "tot" once more with "привет" after it, and it answers "convert" with confidence.
But the model knows only a few applications by name, and elsewhere its answer was discarded:
in Firefox, Edge, WhatsApp, Discord and Slack "tot привет" stayed. The model's answer now
stands everywhere in this case. A pair is also decided together when there was a pause before
the space after the second word.

Checked on 500 new subtitle sentences nobody had seen before: no correct sentence spoiled;
2372 sentences restored instead of 2352 in the usual applications, and 2212 instead of 2098
when typed in Firefox.

### The quote in Telegram becomes "@" at once

A quote typed in the Russian layout at the start of the text or after a space is shown as
"@" at once, so the member list opens; the layout does not change. Typing on turns the "@"
back into the quote, and the word then decides: "ощрт" gives "@john", a Russian word keeps the
quotation. A quote typed right against text, such as `привет,"`, closes a quotation and stays
as it is. Pause right after the "@" gives the quote back. The setting "Telegram: кавычка в
начале слова — это @" turns this off.

### The same setting ranges in both windows

- Confidence threshold: 0.5 to 10 in steps of 0.1 (the Linux window stopped at 8, in steps of 0.5).
- Rule confirmations: up to 10 (the Linux window stopped at 5).
- Pause: 0.3 to 5 seconds, in the engine as well.
- Picking an application to exclude waits 3 seconds in both windows (the Linux one waited 2.5).

A value edited into the settings file by hand is held to the same range.

### Internal: every value is declared once

Every number and shared value of the program lives in one folder of modules named by purpose
and is declared exactly once, numbers inside interface texts included. A check in the test
suite prevents a second name for the same value. The exception for now is the files whose
bytes are pinned by the seals of the shipped models.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

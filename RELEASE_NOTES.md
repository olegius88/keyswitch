# KeySwitch 0.45.0

## Русский

Русская аббревиатура после латинского текста остаётся как набрана: `/designv2 ТЗ` больше не
становится `/designv2 NP`, а `ЛС`, `ТГ`, `НГ` после команды или английской фразы — `KC`, `NU`,
`YU`. Латинский термин, набранный в русской раскладке после русского текста, в котором модель
сомневалась, теперь решается вместе со следующим словом: `принимай зк и продолжай` становится
`принимай pr и продолжай`. Дефис между словами больше не мешает исправить первое слово
сообщения: `Ytn - ult` становится `Нет - где`. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.45.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.45.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.45.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.45.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.45.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.45.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.45.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что исправлено

- Русская аббревиатура после латиницы. После текста, где латинских букв больше, чем русских,
  модель переводила заглавную русскую аббревиатуру в латинские буквы её клавиш, как бы редко они
  ни встречались. Теперь такую аббревиатуру решает отдельная часть модели, которая знает, как
  часто встречается каждое прочтение в русском техническом тексте и в английской прозе:
  `ТЗ`, `ЛС`, `НГ` остаются, а `ГЫ`, набранное в русской раскладке после английского текста,
  по-прежнему становится `US`. На подобранных фразах остаются 27 аббревиатур из 30 против 11.
- Сомнение модели. Если модель оставляет слово, но перевод для неё не менее вероятен, чем 0.3,
  слово теперь ждёт следующее и решается вместе с ним: `зк` после `принимай` модель на пробеле
  оставляла с вероятностью 0.51 против 0.49, а рядом со следующим словом переводит уверенно.
- Знак между словами. Одиночный знак, который в обеих раскладках печатается одинаково (`-`,
  цифра), больше не разрывает цепочку слов: первое слово сообщения, оставленное на своём
  пробеле, исправляется вместе со словом после знака, если модель его переводит.

### Модель

- Контекстная модель `context-v3-19c791a547f4` — модель 0.44.0 с отдельной частью для
  аббревиатур после латиницы, обученной на свежем корпусе. Модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 197 строк,
  без ранней смены восстановлено 175 строк из 196 против 162 у эталонной пары, с ранней сменой —
  18, как у эталонной пары.
- По журналам набора: 822 верных исправления, 120 пропусков, 80 ложных (у 0.44.0 — 79). Разница —
  `зк` в оборванной строке `принимай все зк делай` теперь становится `pr`; на следующий день то же
  слово исправлялось вручную.

### Что осталось

- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не
  создал ЗК?`): на знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- Пауза посреди слова может перевести его начало (`иг` становится `bu`); дописанное слово
  возвращается обратно (`игру`).
- Окончание, допечатанное после стирания части слова, решается как отдельное слово: `ый` может
  стать `sq`.
- Счётчик задержек `input_delay` в техническом журнале принимает за опоздавшую клавишу с нулевым
  временем от другой программы (опоздание около времени работы системы). На скорость ввода это
  не влияет; исправим в следующей версии.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.

## English

A Russian abbreviation typed after Latin text stays as typed: `/designv2 ТЗ` no longer becomes
`/designv2 NP`, nor `ЛС`, `ТГ`, `НГ` after a command or English prose `KC`, `NU`, `YU`. A Latin
term typed in the Russian layout after Russian text that the model was unsure of is now decided
together with the next word: `принимай зк и продолжай` becomes `принимай pr и продолжай`. A
hyphen between words no longer keeps the first word of a message from being corrected:
`Ytn - ult` becomes `Нет - где`. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.45.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.45.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.45.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.45.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.45.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.45.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.45.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Fixed

- A Russian abbreviation after Latin text. After text with more Latin letters than Russian ones
  the model turned a Russian abbreviation in capitals into the Latin letters of its keys, however
  rarely they occur. Such an abbreviation is now decided by a part of the model of its own that
  knows how often each reading occurs in Russian technical text and in English prose: `ТЗ`, `ЛС`,
  `НГ` stay, while `ГЫ` typed in the Russian layout after English text still becomes `US`. On
  authored sentences 27 of 30 such abbreviations stay against 11.
- The model's doubt. When the model keeps a word but finds the conversion at least 0.3 likely,
  the word now waits for the next one and is decided with it: at its space the model kept `зк`
  after `принимай` at 0.51 against 0.49, and beside the next word it converts it with certainty.
- A sign between words. A lone sign typed the same in either layout (`-`, a digit) no longer
  breaks the chain of words: the first word of a message, kept at its space, is corrected
  together with the word after the sign when the model converts it.

### Model

- The context model `context-v3-19c791a547f4` is the 0.44.0 model with a part of its own for
  abbreviations after Latin text, trained on a fresh corpus. The prefix model stays
  `prefix-v2-bf3dc28f8567`.
- On the new sealed check it corrupted none of 197 correctly typed rows and, without the early
  switch, restored 175 of 196 rows against 162 for the reference pair; with the early switch 18,
  as the reference pair.
- On the typing logs: 822 right corrections, 120 missed, 80 false (79 for 0.44.0). The difference
  is `зк` in the cut-off line `принимай все зк делай`, which now becomes `pr`; the same word was
  corrected by hand the next day.

### What remains

- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- A pause in the middle of a word may convert its start (`иг` becomes `bu`); the finished word
  converts back (`игру`).
- An ending typed after part of a word was erased is decided as a word of its own: `ый` may
  become `sq`.
- The `input_delay` counter of the technical log takes a key with a zero time from another
  program for a late one (late by about the system's uptime). Typing speed is not affected; the
  next version fixes it.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.

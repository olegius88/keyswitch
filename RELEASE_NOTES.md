# KeySwitch 0.62.0

## Русский

В этом выпуске модель перестала портить правильно набранный русский текст в двух случаях: опечатка после короткого первого слова (`еще тыв` больше не становится `tot nsd`) и короткое русское сокращение (`кз` больше не становится `rp`). Русское слово, набранное посреди русской фразы с соскользнувшей на английскую раскладкой, теперь исправляется (`я cgtie` — `я спешу`), а первое слово сообщения исправляется вместе с последним, если сообщение кончается знаком. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.62.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.62.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.62.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.62.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.62.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.62.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.62.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.62.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.62.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Слово после короткого первого слова, которое само не слово, но отличается одной опечаткой от частого слова своего языка, больше не переводит пару в другую раскладку: `еще тыв` (опечатка в `ты`) становилось `tot nsd`. Движок теперь сообщает модели, когда текст перед словом — его предположение о другом прочтении предыдущего слова, и на этот вопрос отвечает отдельная часть модели: она сравнивает, насколько часто встречаются оба прочтения и ближайшие к ним слова.
- Короткое русское слово или сокращение, которое русский текст пишет часто, а его латинское прочтение — редко, после русского текста остаётся как набрано: `кз` в `говорит он кз домене` становилось `rp`, которое знает английский словарь. Как часто пишутся такие слова, берётся из новой таблицы, посчитанной по десяти русским корпусам Leipzig Corpora Collection (CC BY).
- Русское слово, набранное с английской раскладкой посреди русского сообщения, когда раскладка сменилась сама, без смены языка, исправляется там, где модель раньше оставляла его подсказкой: `я cgtie` становится `я спешу`, `вопрос htity` — `вопрос решен`, `волк pfdsk` — `волк завыл`. Латинское слово, набранное как задумано после русского текста и прерванное паузой посередине, не трогается, как и раньше.
- Если сообщение кончается знаком сразу после последнего слова, первое слово, ждавшее следующего, исправляется вместе с ним, как и на пробеле: `tot rjire/` оставалось `tot кошку.`, теперь получается `еще кошку.`; так же `If not.` и `ой правда.`.

### Модель

- Контекстная модель `context-v3-b876e862bbfe` — модель 0.61.0 и поверх неё три новые части; модель префиксов `prefix-v2-bf3dc28f8567` та же. На журналах реального набора она не портит ни одного слова и исправляет четыре. На запечатанном тесте v75, прочитанном один раз, сохранены все 189 правильно набранных строк, а без ранней смены раскладки восстановлены все 189 строк, набранных не в той раскладке, против 188 у базовой пары; с ранней сменой — 11 и 11. На запечатанном экзамене по переписке разработчиков ничего не выиграно и ничего не проиграно.

## English

In this release the model stops spoiling correctly typed Russian text in two cases: a typo after a short first word (`еще тыв` no longer becomes `tot nsd`) and a short Russian abbreviation (`кз` no longer becomes `rp`). A Russian word typed in the middle of a Russian phrase after the layout slipped to English is now converted (`я cgtie` becomes `я спешу`), and the first word of a message converts with the last one when the message ends with a sign. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.62.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.62.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.62.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.62.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.62.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.62.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.62.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.62.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.62.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A word that follows a short first word and is no word itself, though one typo from a common word of its language, no longer turns the pair into the other layout: `еще тыв` (a typo of `ты`) became `tot nsd`. The engine now tells the model when the text before a word is its guess about the other reading of the word before, and a part of the model of its own answers that question by comparing how common each reading and the nearest words to them are.
- A short Russian word or abbreviation that Russian text writes often, while its Latin reading is rare, stays as typed after Russian text: `кз` in `говорит он кз домене` became `rp`, which the English lexicon knows. How often such words are written comes from a new table counted on ten Russian corpora of the Leipzig Corpora Collection (CC BY).
- A Russian word typed with the English layout on in the middle of a Russian message, after the layout changed by itself with no change of language, is converted where the model used to leave it a suggestion: `я cgtie` becomes `я спешу`, `вопрос htity` `вопрос решен`, `волк pfdsk` `волк завыл`. A Latin word typed as meant after Russian text and interrupted by a pause in its middle is left alone, as before.
- When a message ends with a sign right after its last word, a first word that was waiting for the next one converts with it, as at a space: `tot rjire/` was left `tot кошку.` and now becomes `еще кошку.`; likewise `If not.` and `ой правда.`.

### Model

- The context model `context-v3-b876e862bbfe` is the model of 0.61.0 with three new parts on top of it; the prefix model `prefix-v2-bf3dc28f8567` is the same. On the recorded real typing it breaks no word and fixes four. On sealed test v75, read once, it keeps all 189 correctly typed rows and restores all 189 rows typed in the wrong layout with the early switch off, against 188 for the baseline pair; with it, 11 and 11. On the sealed developer-chat test it gains nothing and loses nothing.

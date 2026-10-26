# KeySwitch 0.63.0

## Русский

В этом выпуске латинская переменная из одной буквы, набранная в русской раскладке после русского слова, исправляется, когда за ней идёт знак операции и модель в этом уверена: `пусть н = ч` становится `пусть y = x`, `берём т + 1` — `берём n + 1`. Русские буквы перед числом или знаком остаются как набраны (`ГОСТ Р 54960`, `витамин Е 60`). Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.63.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.63.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.63.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.63.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.63.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.63.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.63.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.63.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.63.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Латинская переменная из одной буквы, набранная в русской раскладке после русского слова, спрашивается у модели ещё раз, когда за ней идёт знак операции, и исправляется, если модель уверена: набранное в русской раскладке `пусть y = x` оставалось `пусть н = ч`, а `берём n + 1` — `берём т + 1`. Раньше буква решалась только на своём пробеле, где переменная и буква из переписки (`да я про то ч`) выглядят одинаково. Теперь движок спрашивает о ней снова, когда следом идёт знак или число, одинаковые в обеих раскладках (`=`, `+`, `==`, `+=`, `2`), и отвечает отдельная часть модели: по самому знаку, букве и тексту перед ней. Исправленная буква забирает знак с собой, раскладка переключается вслед за ней, и остаток формулы набирается уже в ней. Одно тире таким знаком не считается: после буквы это и минус, и тире русской фразы (`серия М - это`), различает их только то, что идёт дальше. Русские буквы перед числом или знаком остаются как набраны (`ГОСТ Р 54960`, `витамин Е 60`, `50 кв м + балкон`).

### Модель

- Контекстная модель `context-v3-b5ed6848d4ce` — модель 0.62.0 и поверх неё новая часть; модель префиксов `prefix-v2-bf3dc28f8567` та же. На журналах реального набора она не портит ни одного слова. На запечатанном тесте v76, прочитанном один раз, сохранены все 180 правильно набранных строк, а без ранней смены раскладки восстановлены 176 и 177 из 179 строк, набранных не в той раскладке, против 173 и 174 у базовой пары; с ранней сменой — 17 и 17, при этом базовая пара портит одну правильную строку. На запечатанном экзамене по переписке разработчиков ничего не выиграно и ничего не проиграно.

## English

In this release a Latin variable of one letter typed in the Russian layout after a Russian word is converted when an operator sign follows it and the model is sure: `пусть н = ч` becomes `пусть y = x`, `берём т + 1` becomes `берём n + 1`. Russian letters before a number or a sign stay as typed (`ГОСТ Р 54960`, `витамин Е 60`). The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.63.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.63.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.63.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.63.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.63.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.63.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.63.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.63.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.63.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A Latin variable of one letter typed in the Russian layout after a Russian word is put to the model again when an operator sign follows it, and converted when the model is sure: `пусть y = x` typed in the Russian layout was left `пусть н = ч`, and `берём n + 1` was left `берём т + 1`. The letter used to be decided only at its own space, where a variable and a letter of chat (`да я про то ч`) look alike. Now the engine asks about it again when the next token is a sign or a number that both layouts type alike (`=`, `+`, `==`, `+=`, `2`), and a part of the model of its own answers from the sign, the letter and the text before it. A converted letter takes the sign along and the layout follows it, so the rest of the formula is typed in it. A dash alone is no such sign: after a letter it is a minus as often as the dash of a Russian sentence (`серия М - это`), and only what follows tells them apart. Russian letters before a number or a sign stay as typed (`ГОСТ Р 54960`, `витамин Е 60`, `50 кв м + балкон`).

### Model

- The context model `context-v3-b5ed6848d4ce` is the model of 0.62.0 with the new part on top of it; the prefix model `prefix-v2-bf3dc28f8567` is the same. On the recorded real typing it breaks no word. On sealed test v76, read once, it keeps all 180 correctly typed rows and restores 176 and 177 of the 179 rows typed in the wrong layout with the early switch off, against 173 and 174 for the baseline pair; with it, 17 and 17, where the baseline pair spoils one correct row. On the sealed developer-chat test it gains nothing and loses nothing.

# KeySwitch 0.47.1

## Русский

Программа работает так же, как 0.47.0: модели, настройки и решения по каждому слову не изменились. Выпуск
ускоряет проверку, которую проходит каждая сборка: тесты, воспроизведения моделей и сборка пакетов идут
параллельно и не повторяют одну и ту же загрузку моделей, поэтому исправления доходят до выпуска быстрее.
Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.47.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.47.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.47.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.47.1-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.47.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.47.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.47.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- Проверка сборки. Тесты под измерением покрытия идут на двух машинах вместо одной, а 100% покрытия
  проверяется по их объединённым данным. Три воспроизведения моделей (контекстной, орфотактической и
  границ слов) идут параллельно, а не друг за другом. Модули тестов, которые создают движок в каждом
  тесте, и проверки префиксного и граничного движков больше не разбирают заново контекстную модель
  и словари для каждого теста или строки: проверка префиксного движка шла 6–9 минут, теперь — около
  минуты. Проверка изменений в целом идёт около 7,5 минуты вместо 9,5.
- Для обучения моделей: сравнение кандидата с установленной моделью называет кадры, которые кандидат
  решает иначе, — слово, его другое прочтение и соседние слова.

### Модель

- Контекстная модель `context-v3-158f1f70aa6e` и модель префиксов `prefix-v2-bf3dc28f8567` — те же,
  что в 0.47.0.

### Что осталось

- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не создал ЗК?`): на
  знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- `КЗ` в русском тексте (`даже в КЗ ходит`) становится `RP`: этой аббревиатуры нет в таблице терминов, которой
  модель проверяет русские аббревиатуры. Вернуть слово можно клавишей `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.

## English

KeySwitch works as 0.47.0 did: the models, the settings and the decision on every word are unchanged. The
release speeds up the verification every build goes through: the tests, the model replays and the package builds
run side by side and no longer repeat the same model loading, so fixes reach a release sooner. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.47.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.47.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.47.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.47.1-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.47.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.47.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.47.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- Build verification. The tests under coverage run on two machines instead of one, and 100% coverage is required
  of their joined data. The three model replays (context, orthotactic and word boundary) run side by side rather
  than one after another. The test modules that build an engine for every test, and the prefix and boundary engine
  replays, no longer parse the context model and the dictionaries again for each test or row: the prefix engine
  replay took 6 to 9 minutes and now takes about one. Checking a change takes about 7.5 minutes instead of 9.5.
- For model training: comparing a candidate with the installed model names the frames the candidate decides
  otherwise — the word, its other reading and the words around it.

### Model

- The context model `context-v3-158f1f70aa6e` and the prefix model `prefix-v2-bf3dc28f8567` are those of 0.47.0.

### What remains

- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- `КЗ` in Russian text (`даже в КЗ ходит`) becomes `RP`: the abbreviation is missing from the term table the model
  checks Russian abbreviations with. `Pause` brings the word back.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.

# KeySwitch 0.52.0

## Русский

Выпуск переводит сборки для Windows и macOS на Python 3.15. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.52.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.52.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.52.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.52.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.52.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.52.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.52.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- Сборки для Windows (x64 и ARM) и для Mac собраны на Python 3.15.0 вместо 3.14.8, как и LogCourier. Работает
  KeySwitch так же: те же модели принимают те же решения, весь набор тестов и проверки установки, обновления и
  удаления проходят на 3.15. Пакет для Ubuntu по-прежнему работает на системном Python 3.14.
- Модель намерений пересобрана на Python 3.15.0 и совпала с прежней байт в байт.

### Модель

- Модели те же, что в 0.51.0: контекстная `context-v3-6291162f5dbd` (корпус v46) и модель префиксов
  `prefix-v2-bf3dc28f8567`.

### Что осталось

- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по
  времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не создал ЗК?`): на
  знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- `КЗ` в русском тексте (`даже в КЗ ходит`) становится `RP`: этой аббревиатуры нет в таблице терминов, которой
  модель проверяет русские аббревиатуры. Вернуть слово можно клавишей `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с
  такими же правами (ограничение Windows).

## English

This release moves the Windows and macOS builds to Python 3.15. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.52.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.52.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.52.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.52.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.52.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.52.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.52.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- The Windows (x64 and Arm) and Mac builds are built on Python 3.15.0 instead of 3.14.8, as is LogCourier. KeySwitch
  works as before: the same models make the same decisions, and the whole test suite and the install, upgrade and
  uninstall checks pass on 3.15. The Ubuntu package still runs on the system's Python 3.14.
- The intent model was rebuilt on Python 3.15.0 and came out byte for byte the same.

### Model

- The models are those of 0.51.0: the context model `context-v3-6291162f5dbd` (corpus v46) and the prefix model
  `prefix-v2-bf3dc28f8567`.

### What remains

- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the
  press cannot tell an undo of a wrong correction from a press out of habit.
- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- `КЗ` in Russian text (`даже в КЗ ходит`) becomes `RP`: the abbreviation is missing from the term table the model
  checks Russian abbreviations with. `Pause` brings the word back.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights
  (a Windows restriction).

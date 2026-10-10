# KeySwitch 0.51.0

## Русский

Выпуск убирает задержку клавиатуры на первом слове после запуска KeySwitch и обновляет инструменты сборки. Полный
перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.51.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.51.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.51.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.51.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.51.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.51.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.51.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- Первое слово после запуска больше не задерживает клавиатуру. Контекстная модель читала таблицу частот терминов
  (4,5 МБ) и список идентификаторов при первом вопросе, а орфотактическая модель тогда же готовила словари. Пока это
  шло, хук клавиатуры ждал, и во всей системе ни одна клавиша не доходила до окна: в журналах обеих машин после каждого
  запуска с 0.46.0 был ровно один отчёт о медленном ответе хука, от 31 до 249 мс, и всегда на первом слове. Теперь всё
  это загружается при запуске, до установки хука. На тестовой машине первое слово решалось 757–891 мс, теперь за 26 мс,
  как любое следующее.
- Инструменты сборки обновлены до последних версий: Nuitka 4.3, mypy 2.4.0, coverage 7.16.2, comtypes 1.4.17;
  LogCourier — PySide6 6.12.0 и PyInstaller 6.22.3. Сборки для Mac собираются на macOS 26, сборка для Windows на ARM —
  с Visual Studio 2026; сборка для Mac теперь проверяет, что приложение по-прежнему запускается на macOS 13.
- Python 3.15: KeySwitch проверен на нём целиком, но выпуск собран, как и прежде, на Python 3.14 — компилятор
  сборки Nuitka поддерживает 3.15 пока лишь экспериментально.

### Модель

- Контекстная модель `context-v3-6291162f5dbd` (корпус v46) переобучена тем же рецептом, чтобы закрепить изменённый
  движок. Набор владельца, батареи аббревиатур и калибровку она решает так же, как модель 0.50.0. На запечатанном
  тесте v46 она сохранила все 193 правильно набранные строки и без ранней смены восстановила 178 из 192 набранных не в
  той раскладке (базовая пара — 156). Модель префиксов `prefix-v2-bf3dc28f8567` — та же.

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

This release removes the hold-up of the keyboard on the first word after KeySwitch starts and updates the build
tools. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.51.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.51.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.51.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.51.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.51.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.51.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.51.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- The first word after a start no longer holds up the keyboard. The context model read its term frequency table
  (4.5 MB) and the identifier list on its first question, and the orthotactic model prepared its dictionaries at the
  same moment. While that ran the keyboard hook waited, and no key anywhere in the system reached a window: the logs of
  both machines held exactly one slow-hook report after every start since 0.46.0, 31 to 249 ms, always on the first
  word. All of it now loads at start, before the hook is installed. On the test machine the first word took 757–891 ms
  to decide; now it takes 26 ms, as every later one.
- The build tools move to their newest releases: Nuitka 4.3, mypy 2.4.0, coverage 7.16.2, comtypes 1.4.17; LogCourier
  takes PySide6 6.12.0 and PyInstaller 6.22.3. The Mac builds run on macOS 26 and the Windows Arm build with Visual
  Studio 2026; the Mac build now checks that the application still runs on macOS 13.
- Python 3.15: KeySwitch is tested on it in full, but the release is built on Python 3.14 as before — the Nuitka
  compiler supports 3.15 only experimentally so far.

### Model

- The context model `context-v3-6291162f5dbd` (corpus v46) was retrained with the same recipe to seal the changed
  engine. It decides the owner's typing, the abbreviation batteries and calibration as the 0.50.0 model does. On the
  sealed test v46 it kept all 193 correctly typed sequences and, without the early switch, restored 178 of the 192
  typed in the wrong layout (156 for the base pair). The prefix model `prefix-v2-bf3dc28f8567` is unchanged.

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

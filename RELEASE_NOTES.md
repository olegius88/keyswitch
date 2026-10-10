# KeySwitch 0.50.0

## Русский

Выпуск возвращает автоматическое переключение в VS Code там, где оно молчало: на компьютере, где у редактора
выключена поддержка экранного чтения, KeySwitch отказывался исправлять многие слова, и их исправлял только
`Pause`. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.50.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.50.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.50.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.50.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.50.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.50.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.50.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- VS Code без поддержки экранного чтения. Перед исправлением слова KeySwitch сверяет его с текстом поля через
  специальные возможности Windows. VS Code с выключенной поддержкой экранного чтения показывает им не текст
  редактора, а пустое служебное поле ввода — в нём лежит самое большее только что нажатая клавиша. Слова там не
  было, и KeySwitch решал, что текст изменился («текст активного поля изменился») или что слово набрано внутри
  другого слова, и ничего не менял: `ghbdtn` оставалось `ghbdtn`, пока не нажмёшь `Pause`. Теперь такое поле (не
  больше одного символа после более длинного слова) не принимается за текст: KeySwitch опирается на то, что сам видел
  с клавиатуры, как в программах, поле которых не читается вовсе. Если поле показывает больше, но набранного слова в
  нём нет, исправление по-прежнему отменяется: стирать там слово опасно.

### Модель

- Контекстная модель `context-v3-86437b6d1577` (корпус v45) переобучена тем же рецептом, чтобы закрепить изменённый
  движок. На наборе владельца, батареях аббревиатур и калибровке она решает так же, как модель 0.49.0. На
  запечатанном тесте v45 она сохранила все 196 правильно набранных строк и без ранней смены восстановила 172 из 196
  набранных не в той раскладке (базовая пара — 160). Модель префиксов `prefix-v2-bf3dc28f8567` — та же.

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

This release brings automatic switching back to VS Code where it was silent: on a computer where the editor's
screen-reader support is off, KeySwitch refused to convert many words, and only `Pause` converted them. The full
list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.50.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.50.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.50.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.50.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.50.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.50.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.50.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- VS Code without screen-reader support. Before converting a word, KeySwitch checks it against the field's text
  through the Windows accessibility interface. VS Code with its screen-reader support off shows that interface not the
  editor's text but an empty input it takes keys through, holding at most the key just pressed. The word was not
  there, so KeySwitch took the text for changed or the word for one typed inside another and left it alone: `ghbdtn`
  stayed `ghbdtn` until `Pause` was pressed. Such a field (at most one character after a longer word) is no longer
  taken for the text: KeySwitch goes by the keys it saw itself, as in programs whose field cannot be read at all. A
  field that shows more and lacks the typed word still cancels the correction, because erasing the word there is not
  safe.

### Model

- The context model `context-v3-86437b6d1577` (corpus v45) was retrained with the same recipe to seal the changed
  engine. It decides the owner's typing, the abbreviation batteries and calibration as the 0.49.0 model does. On the
  sealed test v45 it kept all 196 correctly typed sequences and, without the early switch, restored 172 of the 196
  typed in the wrong layout (160 for the base pair). The prefix model `prefix-v2-bf3dc28f8567` is unchanged.

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

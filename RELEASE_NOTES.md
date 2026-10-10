# KeySwitch 0.49.0

## Русский

Выпуск убирает частую неудобность с `Pause`. Если вы перестали печатать, чтобы нажать `Pause`, а KeySwitch как раз в
эту паузу сам исправил слово, нажатие больше не возвращает слово обратно. Кроме того, буквы, допечатанные во время
исправления, больше не стирают программе память о фразе. Выпуск включает все изменения 0.48.0 — надёжность
Windows: установщик поверх запущенной программы, Блокнот Windows 11, запоздавшая смена раскладки, залипший
модификатор. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.49.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.49.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.49.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.49.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.49.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.49.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.49.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- `Pause` сразу после исправления на паузе. Вы видите слово не в той раскладке, перестаёте печатать и тянетесь к
  `Pause`, а KeySwitch как раз в эту паузу (1,5 секунды без набора) исправляет слово сам. Нажатие приходило следом и
  возвращало слово обратно: в журналах владельца так было все восемь раз, и `Pause` приходилось нажимать снова.
  Теперь первое нажатие не позже 1,5 секунды после такого исправления, если после него ничего не набрано, ничего не
  меняет: в строке состояния — «Уже преобразовано». Второе нажатие вернёт слово, как раньше. Исправления на пробеле
  не изменились: там быстрое `Pause` так же часто отменяет ошибочное исправление.
- Буквы, допечатанные во время исправления. Пока KeySwitch заменяет слово, набранные клавиши придерживаются и
  печатаются сразу после него. Прежде из-за них программа забывала, что было написано перед словом: в фразе
  `нажимаю на Pause b` после ранней смены `Pause` буква `b` не стала `и` — правило для одиночной буквы после
  английского слова в русской фразе не видело русской фразы. Теперь фраза остаётся в памяти; её стирает только
  щелчок мышью во время замены.

Из 0.48.0, которая не публиковалась отдельно:

- Установщик закрывает запущенный KeySwitch. Установка поверх работающей программы останавливалась с кодом
  ошибки 5. Теперь установщик и программа удаления сами закрывают KeySwitch; автообновление сначала даёт программе
  завершиться самой.
- Раскладка в Блокноте Windows 11 берётся у поля ввода, когда у него свой поток; в остальных программах всё как
  прежде.
- Запоздавшая смена раскладки. KeySwitch ждёт подтверждения до полутора секунд вместо половины и засчитывает
  запоздавшую смену себе, а не вам: следующее слово больше не остаётся без исправления.
- Залипший модификатор. Если Ctrl, Alt или Shift отпустить, пока на экране окно UAC, экран Ctrl+Alt+Del или окно
  программы с правами администратора, KeySwitch дальше считал каждую букву сочетанием клавиш. Теперь после паузы
  или смены окна он сверяется с клавиатурой. Поток перехвата клавиатуры работает с повышенным приоритетом.
- В README: как вынести значок `EN/RU` на панель задач Windows 11 и как назначить ручное преобразование другой
  клавише на ноутбуке без `Pause`.

### Модель

- Контекстная модель `context-v3-e6e581145f16` (корпус v43) переобучена тем же рецептом, чтобы закрепить
  изменённый движок. На наборе владельца, батареях аббревиатур и калибровке она решает так же, как модель 0.48.0. На
  запечатанном тесте v43 она сохранила все 201 правильно набранную строку и без ранней смены восстановила 186 из 199
  набранных не в той раскладке (базовая пара — 167). Модель префиксов `prefix-v2-bf3dc28f8567` — та же.

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

This release removes a frequent annoyance with `Pause`. When you stop typing to press `Pause` and KeySwitch converts
the word itself during that very pause, the press no longer converts the word back. Letters typed while a word is
being corrected no longer make KeySwitch forget the phrase before it. The release includes every change of 0.48.0 —
Windows reliability: the installer over a running application, Windows 11 Notepad, a late layout switch, a stuck
modifier. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.49.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.49.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.49.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.49.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.49.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.49.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.49.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- `Pause` right after a correction made at a pause. You see a word in the wrong layout, stop typing and reach for
  `Pause`, and KeySwitch converts the word itself during that very pause (1.5 seconds without typing). The press came
  next and converted the word back: in the owner's logs it did so all eight times, and `Pause` had to be pressed again.
  Now the first press within 1.5 seconds of such a correction, with nothing typed since, changes nothing; the status
  says the word is already converted. A second press converts the word back as before. Corrections made at a space
  are unchanged: there a quick `Pause` undoes a wrong correction just as often.
- Letters typed during a correction. While KeySwitch replaces a word, the keys you type are held back and typed
  right after it. They used to make KeySwitch forget what was written before the word: in `нажимаю на Pause b`, after
  `Pause` was switched early, `b` did not become `и` — the rule for a single letter after an English word in a Russian
  phrase saw no Russian phrase. The phrase is now kept; only a mouse click during the replacement clears it.

From 0.48.0, which was not published on its own:

- The installer closes a running KeySwitch. Installing over the running application stopped with error code 5. The
  installer and the uninstaller now close KeySwitch themselves; an auto-update first lets the application shut itself
  down.
- The layout in Windows 11 Notepad is read from the text control when it runs in a thread of its own; every other
  program works as before.
- A late layout switch. KeySwitch waits up to a second and a half for the confirmation instead of half a second, and
  books a late switch as its own rather than yours, so the next word is no longer left uncorrected.
- A stuck modifier. When Ctrl, Alt or Shift was released while a UAC prompt, the Ctrl+Alt+Del screen or a window of an
  elevated program was in front, KeySwitch took every later letter for a shortcut. After a pause or a window change it
  now asks the keyboard. The keyboard hook thread runs at a raised priority.
- README: how to bring the `EN/RU` icon onto the Windows 11 taskbar, and how to give the manual conversion another key
  on a laptop without `Pause`.

### Model

- The context model `context-v3-e6e581145f16` (corpus v43) was retrained with the same recipe to seal the changed
  engine. It decides the owner's typing, the abbreviation batteries and calibration as the 0.48.0 model does. On the
  sealed test v43 it kept all 201 correctly typed sequences and, without the early switch, restored 186 of the 199
  typed in the wrong layout (167 for the base pair). The prefix model `prefix-v2-bf3dc28f8567` is unchanged.

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

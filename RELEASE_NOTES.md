# KeySwitch 0.48.0

## Русский

Выпуск делает работу в Windows надёжнее: установщик больше не падает, если KeySwitch запущен; раскладка
правильно читается в Блокноте Windows 11; исправление не теряется, если приложение подтверждает смену
раскладки с задержкой; и автопереключение не замолкает после окна UAC или Ctrl+Alt+Del. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.48.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.48.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.48.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.48.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.48.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.48.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.48.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- Установщик закрывает запущенный KeySwitch. Установка поверх работающей программы останавливалась с кодом
  ошибки 5: Windows не умеет закрыть программу, которая живёт в области уведомлений. Теперь установщик и
  программа удаления сами закрывают KeySwitch; автообновление сначала даёт программе завершиться самой.
- Раскладка в Блокноте Windows 11. У его поля ввода своя раскладка, отдельная от окна, и KeySwitch читал и
  переключал не ту. Теперь раскладка берётся у поля ввода, когда у него свой поток; в остальных программах всё
  как прежде.
- Запоздавшая смена раскладки. Приложение подтверждало смену раскладки до секунды (так было в VS Code
  Insiders), а KeySwitch ждал полсекунды: исправление по `Pause` отменялось, а пришедшая раскладка считалась
  ручной и защищала следующее слово от исправления. Теперь KeySwitch ждёт до полутора секунд и засчитывает
  запоздавшую смену себе.
- Залипший модификатор. Если Ctrl, Alt или Shift отпустить, пока на экране окно UAC, экран Ctrl+Alt+Del или
  окно программы с правами администратора, KeySwitch этого не видел и дальше считал каждую букву сочетанием
  клавиш: автопереключение молчало до следующего нажатия этой клавиши. Теперь после паузы или смены окна он
  сверяется с клавиатурой.
- Поток перехвата клавиатуры работает с повышенным приоритетом: Windows молча снимает перехват, который
  отвечает слишком долго.
- В README: Windows 11 прячет значки новых программ, вынести `EN/RU` на панель можно в параметрах панели
  задач; на ноутбуке без клавиши `Pause` ручное преобразование можно назначить другой клавише.

### Модель

- Контекстная модель `context-v3-309759404d7d` (корпус v42) переобучена тем же рецептом, чтобы закрепить
  изменённые файлы программы. На наборе владельца, батареях аббревиатур и калибровке она решает так же, как
  0.47.1; на запечатанном тесте v42 сохранила все 190 правильно набранных строк и без ранней смены восстановила
  169 из 189 набранных не в той раскладке (базовая пара — 156). Модель префиксов `prefix-v2-bf3dc28f8567` — та же.

### Что осталось

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

This release makes KeySwitch more dependable on Windows: the installer no longer fails while KeySwitch is running,
the layout is read correctly in Windows 11 Notepad, a correction is not lost when an application confirms the layout
switch late, and automatic switching no longer falls silent after a UAC prompt or Ctrl+Alt+Del. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.48.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.48.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.48.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.48.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.48.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.48.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.48.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- The installer closes a running KeySwitch. Installing over the running application stopped with error code 5:
  Windows cannot close a program that lives in the notification area. The installer and the uninstaller now close
  KeySwitch themselves; an auto-update first lets the application shut itself down.
- The layout in Windows 11 Notepad. Its text control keeps a layout of its own, apart from the window, and
  KeySwitch read and switched the wrong one. The layout now comes from the focused control when it runs in a thread
  of its own; every other program works as before.
- A late layout switch. An application took up to a second to confirm a layout switch (VS Code Insiders did),
  while KeySwitch waited half a second: the `Pause` correction was given up, and the layout that arrived counted
  as a manual pick that protected the next word. KeySwitch now waits up to a second and a half and books a late
  switch as its own.
- A stuck modifier. When Ctrl, Alt or Shift was released while a UAC prompt, the Ctrl+Alt+Del screen or a window
  of an elevated program was in front, KeySwitch never saw it and took every later letter for a shortcut: automatic
  switching stayed silent until that key was pressed again. After a pause or a window change it now asks the
  keyboard.
- The keyboard hook thread runs at a raised priority: Windows silently removes a hook that answers too late.
- README: Windows 11 hides the icons of new programs, and `EN/RU` can be brought onto the taskbar in its settings;
  on a laptop without a `Pause` key, the manual conversion can be given another key.

### Model

- The context model `context-v3-309759404d7d` (corpus v42) was retrained with the same recipe to seal the changed
  program files. It decides the owner's typing, the abbreviation batteries and calibration as 0.47.1 does; on the
  sealed test v42 it kept all 190 correctly typed sequences and, without the early switch, restored 169 of the 189
  typed in the wrong layout (156 for the base pair). The prefix model `prefix-v2-bf3dc28f8567` is unchanged.

### What remains

- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- `КЗ` in Russian text (`даже в КЗ ходит`) becomes `RP`: the abbreviation is missing from the term table the model
  checks Russian abbreviations with. `Pause` brings the word back.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights
  (a Windows restriction).

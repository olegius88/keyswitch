# KeySwitch 0.36.3

## Русский

Выпуск исправлений. Главное — macOS: KeySwitch больше не закрывается сам через несколько секунд после
запуска, учитывает Shift, Control, Option, Command и Caps Lock и читает длинные поля. Кроме того,
слово, которое ждёт следующего, теперь решается после паузы, а Windows больше не хранит установщики
прежних обновлений. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.36.3-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.36.3-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.36.3_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.36.3-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.36.3-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### macOS

- KeySwitch закрывался сам через несколько секунд после запуска: раскладку он спрашивал у системы не
  из главного потока, а macOS отвечает на такие вопросы только в главном потоке и останавливает
  программу. Теперь вопросы о раскладке задаются в главном потоке.
- Shift, Control, Option и Command приходят как изменение флагов, а программа принимала каждое
  такое событие за отпускание. Поэтому `Ghbdtn` перепечатывалось как `привет`, `Shift+Return`
  удерживался как обычный `Enter`, а горячие клавиши с модификатором не срабатывали. Теперь нажатие
  читается из флагов, заглавная буква перепечатывается заглавной, а горячие клавиши вроде
  `Control+Option+Z` срабатывают и до окна не доходят: раньше TextEdit вставлял на них невидимый
  символ, и отмена стирала его вместо последней буквы слова.
- Слово, набранное с включённым Caps Lock, не исправлялось: программа читала его строчными буквами,
  а в поле стояли заглавные, и проверка поля отказывала. Включённый Caps Lock нажатой клавишей тоже
  не считается.
- Поле длиннее примерно 255 кириллических символов (заметка, письмо, чат) читалось пустым, а эмодзи
  перед курсором сдвигал курсор на символ вправо.

### Текст

- Первое слово предложения не исправлялось там, где редактор сам делает его заглавным, например в
  TextEdit на macOS. Программа видела в поле `Ghbdtn` вместо набранного `ghbdtn` и отказывалась.
  Теперь слово, у которого изменился только регистр первой буквы, по-прежнему считается набранным.
- Когда модель не уверена в коротком слове (`yt `, `ша `), она ждёт следующее слово. Если следующего
  не было, слово так и оставалось набранным: пауза, которая должна была его решить, отсчитывалась от
  момента, который программа уже сбросила. Теперь после паузы такое слово решается ещё раз — модель
  исправляет его или оставляет.

### Надёжность

- Нечисловое значение лимита истории в файле настроек больше не мешает запуску.
- Linux: полный сброс настроек сразу применяет тему, значок в трее и его переключатели, а не после
  перезапуска.
- Windows: при загрузке нового обновления удаляются установщики прежних; каждый занимал десятки
  мегабайт. Установщик, который ещё открыт, остаётся до следующей загрузки.

### Что стало хуже

- После паузы слово, которое ждало следующего, теперь может измениться само; раньше оно оставалось как
  набрано. Если программа ошиблась, верните слово клавишей `Pause`.
- macOS: слово, после которого сразу нажат `Shift+Return`, больше не исправляется: перенос строки уже
  стоит за ним. Раньше слово исправлялось, но вместо переноса строки уходил обычный `Enter`, и в чате
  сообщение отправлялось.

## English

A fix release, macOS first: KeySwitch no longer closes by itself a few seconds after it starts, it
reads Shift, Control, Option, Command and Caps Lock, and it reads long fields. Besides, a word that
waits for the next one is now decided after a pause, and Windows no longer keeps the installers of
earlier updates. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.36.3-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.36.3-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.36.3_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.36.3-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.36.3-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### macOS

- KeySwitch closed by itself a few seconds after it started: it asked the system for the layout
  from a thread other than the main one, and macOS answers such questions on the main thread only
  and stops the program. The layout is now asked for on the main thread.
- Shift, Control, Option and Command arrive as flag changes, and the program took every such event
  for a release. So `Ghbdtn` was retyped as `привет`, `Shift+Return` was held as a plain `Enter`,
  and hotkeys with a modifier never matched. The key state is now read from the flags, a capital is
  retyped as a capital, and hotkeys such as `Control+Option+Z` match and no longer reach the window:
  TextEdit used to insert an invisible character for them, which the undo then erased in place of
  the word's last letter.
- A word typed with Caps Lock on was not corrected: the program read it in lower case while the
  field showed capitals, and the field check refused. An engaged Caps Lock does not count as a held
  key either.
- A field over about 255 Cyrillic characters (a note, a mail, a chat) read as empty, and an emoji
  before the caret moved the caret one character to the right.

### Text

- The first word of a sentence was not corrected where the editor capitalises it, for example in
  TextEdit on macOS. The program saw `Ghbdtn` in the field where `ghbdtn` was typed and refused. A
  word whose first letter changed case only still counts as the typed one.
- When the model is unsure about a short word (`yt `, `ша `), it waits for the next word. With no next
  word the word stayed as typed: the pause that should decide it was measured from a moment the
  program had already cleared. Now the word is decided once more after a pause, and the model
  either corrects it or keeps it.

### Reliability

- A history limit in the settings file that is not a number no longer stops the program at start.
- Linux: a full reset of the settings applies the theme, the tray icon and its toggles at once,
  not after a restart.
- Windows: downloading a new update removes the installers of earlier ones, each tens of
  megabytes. An installer that is still open stays until the next download.

### What got worse

- After a pause, a word that waited for the next one may now change by itself; before, it stayed as
  typed. If the program got it wrong, press `Pause` to turn the word back.
- macOS: a word followed at once by `Shift+Return` is no longer corrected, because the line break
  already stands after it. It used to be corrected, but a plain `Enter` went out instead of the line
  break, and a chat sent the message.

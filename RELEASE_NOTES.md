# KeySwitch 0.36.2

## Русский

Выпуск исправлений: KeySwitch больше не портит слово при отмене раннего
переключения, не останавливается из-за ошибки записи истории, не теряет настройки, сохранённые
Блокнотом, и не превращает `plan b` в `plan и`. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.36.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.36.2-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.36.2_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.36.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.36.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Текст

- После раннего переключения запятая или точка сразу за словом не запоминалась вместе с ним, и `Pause`
  или отмена исправления стирали на один символ меньше: `hello, ` превращалось в `hруддщ `. Теперь
  получается `руддщ, `.
- При выключенной настройке «Учитывать контекст» и в программах без имени одиночная буква после слова
  исправлялась как начало сообщения: `plan b` становилось `plan и`, `vitamin c` — `vitamin с`. Теперь
  правило начала сообщения действует, только когда программа знает, что слова перед буквой не было: без
  контекста буква остаётся как есть, а в программе без имени её решает контекстная модель.

### Надёжность

- Ошибка в исправлении по паузе или при отложенном `Enter` (например, запись истории на заполненный
  диск) останавливала обработку ввода; в Windows удержанный `Enter` после этого не отпускался. Теперь
  ошибка показывается в состоянии программы, а ввод продолжает обрабатываться.
- Файл настроек или правил, сохранённый Блокнотом (UTF-8 с меткой BOM), не читался, и при первом же
  изменении настройки его заменяли значения по умолчанию — вместе с исключёнными программами. Теперь
  такой файл читается, а файл, который прочитать нельзя, сохраняется рядом как
  `config.json.unreadable-<дата-время>` до того, как программа начнёт работать с настройками по
  умолчанию.
- История исправлений перезаписывается через временный файл: сбой во время записи больше не стирает её.
- В Windows фокус на кнопке или поле без доступного текста отключал чтение контекста поля во всех
  программах до минуты. Теперь такое поле просто не читается.

### Что стало хуже

При выключенной настройке «Учитывать контекст» одиночная буква в начале сообщения (`b` вместо `и`)
больше не исправляется автоматически: без контекста программа не знает, что слова перед ней нет.
Исправьте её `Pause` или включите настройку.

## English

A fix release: KeySwitch no longer garbles a word when an early switch is
undone, no longer stops over a failed history write, no longer loses settings saved by Notepad, and
no longer turns `plan b` into `plan и`. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.36.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.36.2-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.36.2_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.36.2-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.36.2-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Text

- After an early switch, a comma or full stop right after the word was not recorded with it, and
  `Pause` or undo deleted one character too few: `hello, ` became `hруддщ `. It now becomes
  `руддщ, `.
- With "consider context" switched off, and in programs without a name, a lone letter after a word
  was corrected as if it started a message: `plan b` became `plan и`, `vitamin c` became
  `vitamin с`. The message-start rule now applies only where the program knows that no word came
  before the letter: without context the letter stays, and in a program without a name the context
  model decides it.

### Reliability

- An error during a pause correction or a deferred `Enter` (for example, writing the history to a
  full disk) stopped input processing; on Windows a held `Enter` was then never released. The error
  is now shown in the program's status and input keeps being processed.
- A settings or rules file saved by Notepad (UTF-8 with a byte order mark) could not be read, and
  the next settings change replaced it with the defaults, excluded programs included. Such a file is
  now read, and a file that cannot be read is kept next to it as
  `config.json.unreadable-<date-time>` before the program works on defaults.
- The correction history is rewritten through a temporary file, so a failure during the write no
  longer erases it.
- On Windows, focusing a button or a field without accessible text switched field context reading
  off in every program for up to a minute. Such a field is now simply not read.

### What got worse

With "consider context" switched off, a lone letter at the start of a message (`b` for `и`) is no
longer corrected automatically: without context the program cannot know that no word stands before
it. Correct it with `Pause`, or switch the setting on.

# KeySwitch 0.26.0

## Русский

KeySwitch теперь работает на macOS. Всё остальное — как в 0.25.3, полный перечень
в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-0.26.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.26.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `keyswitch_0.26.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.26.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.26.0-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот. Если не знаете, какой у вас, откройте меню Apple → «Об этом Mac»: там указан
либо чип Apple, либо процессор Intel.

### Что умеет macOS-версия

Она исправляет раскладку тем же движком и теми же двумя моделями, что Linux и Windows, —
им всё равно, где работать. Напечатанное `ghbdtn ` становится `привет `.

Значок в строке меню показывает текущую раскладку и открывает всё остальное:
автопереключение, звук, уведомления, историю, исключения, настройки. Окно настроек — то
же, что на Windows, со всеми десятью разделами.

Программа читает текст перед курсором через систему универсального доступа. Это помогает
там, где слово дописывают к уже набранному, — раньше такие случаи приходилось угадывать.

Отдельная приятная разница: на macOS придержанные Enter и Tab уходят по назначению без
осторожности, которая нужна на Windows. Там система позволяет придержать клавишу
по-настоящему, поэтому вся сложность, из-за которой вышли 0.25.2 и 0.25.3, попросту не
возникает.

### Установка

1. Распакуйте архив и перенесите `KeySwitch.app` в «Программы».
2. При первом запуске macOS спросит, можно ли программе следить за клавиатурой. Ответьте
   «Открыть настройки» и включите KeySwitch в разделе «Конфиденциальность и безопасность»
   → «Универсальный доступ». Разрешение обязательно: без него клавиатуры не видно, и
   KeySwitch честно ждёт, а не делает вид, что работает.
3. Как только переключатель включён, программа продолжает сама — перезапускать не нужно.

Приложение подписано сертификатом разработчика, но ещё не заверено у Apple. Поэтому при
первом открытии система может не пустить его сразу: откройте «Конфиденциальность и
безопасность» и нажмите «Открыть всё равно». На macOS 13 и 14 достаточно открыть
программу через контекстное меню.

### Чего на macOS пока нет

- Проверка обновлений: на macOS она не работает, обновляться нужно вручную.
- Заверение у Apple: из-за этого разрешение на клавиатуру придётся выдать заново после
  обновления программы. Программа об этом скажет и снова откроет нужный раздел.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

KeySwitch now runs on macOS. Everything else is as in 0.25.3; the full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-0.26.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.26.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `keyswitch_0.26.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-Setup-0.26.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.26.0-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round. The Apple menu, "About This Mac", says which you have.

### What the macOS build does

It corrects the layout with the same engine and the same two models as Linux and Windows,
which never knew which system they were running on. Typing `ghbdtn ` leaves `привет `.

The menu bar shows the current layout and opens everything else: automatic switching,
sound, notifications, history, exclusions, settings. The settings window is the one
Windows already had, with all ten pages.

The text in front of the caret is read through the accessibility interface, which helps
where a word is being added to text already on screen - cases that previously had to be
guessed at.

One pleasant difference: on macOS a withheld Enter or Tab goes through without the caution
Windows needs. There the system allows a key to be genuinely withheld, so the whole
difficulty that produced 0.25.2 and 0.25.3 never arises.

### Installing

1. Unpack the archive and move `KeySwitch.app` into Applications.
2. On the first launch macOS asks whether the program may watch the keyboard. Choose to
   open the settings and enable KeySwitch under Privacy & Security, Accessibility. The
   permission is required: without it there is no keyboard to watch, and KeySwitch waits
   openly rather than pretending to work.
3. The moment the switch is on it carries on by itself; no relaunch is needed.

The application is signed with a developer certificate but is not notarized by Apple yet,
so the system may refuse to open it at first: go to Privacy & Security and choose to open
it anyway. On macOS 13 and 14 opening it from the context menu is enough.

### Not on macOS yet

- Update checking does not work there; updates are installed by hand.
- Without notarization the keyboard permission has to be granted again after the program
  is updated. KeySwitch says so and reopens the right pane.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

# KeySwitch 0.74.4

## Русский

В этом выпуске набор на Windows тратит заметно меньше процессора: в обычных текстовых полях Windows — примерно в два с половиной раза, в браузере и программах на Electron — на четверть.

### Файлы выпуска

- `KeySwitch-Setup-0.74.4-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.74.4-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.74.4-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.4-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.74.4-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.4-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.74.4_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.74.4-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.74.4-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Windows: набор тратит заметно меньше процессора. На тестовой машине на 1000 набранных клавиш ушло 4,6 секунды процессора вместо 11,7 в обычном текстовом поле Windows (RichEdit), 8,4 вместо 11,3 в Edge и 8,5 вместо 11,2 в программе на Electron; текст везде набран так же, как раньше.
- Windows: текст поля, в котором идёт набор, программа читает через специальные возможности Windows (UI Automation). Windows держит для этого служебный поток, пока программа держит хоть что-то от поля, а программа после каждого чтения отпускала всё — и Windows заводила поток заново на каждое чтение: больше тысячи раз за 1500 клавиш. Теперь программа держит последнее прочитанное поле до следующего чтения.
- Windows: в классических программах (Блокнот, поля обычных окон и окон Windows Forms) программа больше не ищет заново при каждом чтении, в каком поле стоит курсор: она спрашивает у поля, которое читала в прошлый раз, по-прежнему ли оно в фокусе, — это примерно в восемь раз дешевле — и ищет заново, только если ответ «нет». В браузерах и программах на Electron поле по-прежнему ищется каждый раз: там прежнее поле отвечает «в фокусе», даже когда курсор уже в адресной строке, на другой вкладке или в инструментах разработчика.
- Windows: имя программы впереди программа узнавала дважды на каждую клавишу, открывая её процесс, а у окна впереди на каждой клавише спрашивала его класс, чтобы понять, не панель ли это задач. Теперь и то и другое запоминается, пока впереди то же окно.
- Windows и macOS: главный поток программы больше не просыпается на каждой клавише. Движок на каждой клавише обновляет своё состояние набираемым словом, которое не показывают ни значок в трее, ни окно настроек, и каждое такое обновление будило их, чтобы нарисовать то же самое. Теперь им передаётся только то, что меняет показанное: работает ли программа и переключает ли раскладку, раскладка, число исправлений, последнее действие или ошибка.

### Если нужно вернуться на 0.74.3

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.74.3-<архитектура>.exe` со [страницы выпуска 0.74.3](https://github.com/olegius88/keyswitch/releases/tag/v0.74.3).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.3`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.74.3.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`.

## English

In this release typing on Windows takes much less processor time: about two and a half times less in ordinary Windows text fields, and a quarter less in a browser and in Electron programs.

### Release files

- `KeySwitch-Setup-0.74.4-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.74.4-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.74.4-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.74.4-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.74.4-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.74.4-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.74.4_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.74.4-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.74.4-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- Windows: typing takes much less processor time. On a test machine 1000 keys typed took 4.6 seconds of processor time instead of 11.7 in an ordinary Windows text field (RichEdit), 8.4 instead of 11.3 in Edge and 8.5 instead of 11.2 in an Electron program; the text came out as before everywhere.
- Windows: KeySwitch reads the text of the field being typed in through the accessibility interface of Windows (UI Automation). Windows keeps a thread for it while the program holds anything of the field, and the program let go of everything after each read, so Windows started the thread again for every read: more than a thousand times in 1500 keys. The program now keeps the last field it read until the next read.
- Windows: in classic programs (Notepad, the fields of ordinary windows and of Windows Forms) the program no longer looks up at each read which field has the caret: it asks the field it read the time before whether it still has the focus, which costs about an eighth of the lookup, and looks the focus up only when the answer is no. In browsers and Electron programs the field is looked up every time as before: there the field left behind says it has the focus while the caret is already in the address bar, another tab or the developer tools.
- Windows: the program learned the name of the program in front twice a key by opening its process, and asked the window in front for its class at each key to know whether it is the taskbar. Both are now kept while the same window is in front.
- Windows and macOS: the program's main thread is no longer woken at every key. The engine updates its state at each key with the word being typed, which neither the tray icon nor the settings window shows, and each such update woke them to draw the same. Now they are handed only what changes what they show: whether the program runs and switches the layout, the layout, the count of corrections, the last action or error.

### Going back to 0.74.3

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.74.3-<architecture>.exe` from the [0.74.3 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.74.3).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.3`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.74.3 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`.

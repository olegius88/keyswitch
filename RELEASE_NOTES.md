# KeySwitch 0.70.0

## Русский

KeySwitch переписан на Rust: в программе больше нет Python. Раскладку он определяет так же, как 0.66.2, модели и словари — те же файлы, но установщик на 40% меньше, а памяти программа занимает в три–четыре с половиной раза меньше. Настройки, правила, история и личные слова сохраняются при обновлении. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.70.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.70.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.70.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.70.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.70.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.70.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.70.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.70.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.70.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Установщик для Windows — 19,5 МБ вместо 31,8 МБ, пакет для Ubuntu — 20,4 МБ вместо 26,6 МБ, архив для Mac — 23 МБ вместо 37 МБ.
- Памяти нужно меньше: в Windows 59 МБ вместо 264 МБ, в Linux 119 МБ вместо 333 МБ, на Mac 61 МБ вместо 216 МБ.
- В Windows программа готова переключать раскладку через 3 секунды после запуска вместо 17.
- Windows: окно настроек, значок в трее и окна правил нарисованы стандартными элементами Windows.
- Windows: буквы на значке в трее крупные и чёткие при любом масштабе экрана: значок рисуется в том размере, в каком его показывает область уведомлений (от 16 точек при 100% до 40 при 250%), и перерисовывается при смене масштаба. Когда автопереключение выключено, значок серый с приглушёнными буквами, без белой черты поперёк них.
- Windows: пункты меню трея «Переключить на русский (RU)» и «Переключить на английский (EN)» переключают раскладку в окне, где набирался текст, и возвращают это окно на передний план. Раньше раскладка менялась у скрытого окна KeySwitch: программа, в которой шёл набор, получала новую раскладку только после щелчка по ней, а при включённом «Позволить выбирать метод ввода для каждого окна приложения» оставалась в прежней, или KeySwitch сообщал «Приложение не подтвердило смену раскладки». Пока впереди панель задач, меню предлагает переключение для окна, где набирался текст, а не по раскладке самой панели.
- macOS: исправление показывается в Центре уведомлений; страница обновлений говорит, что новую версию для Mac ставят вручную со страницы выпуска.
- Linux: строки в окне выбора программы откликаются на щелчок; в 0.66.2 щелчок по ним ничего не делал.
- Linux: окно настроек больше не зависает, если настройку быстро поменять дважды (два быстрых щелчка по «Включено» или смена из меню значка при открытом окне).
- Раскладку можно переключать одиночным нажатием модификатора, как в Punto Switcher и Caramba: настройка «Переключать раскладку одной клавишей» на странице «Горячие клавиши» (левый или правый Shift либо Ctrl, по умолчанию выключено). Shift с буквой, Ctrl+C и системные Alt+Shift, Ctrl+Shift раскладку не меняют; в README описано, какие клавиши её переключают.
- macOS: ручное преобразование и двойное нажатие для правила — на Control+Option+K. Раньше они стояли на Pause, которой на клавиатуре Mac нет, и не работали; настройка с Pause один раз заменяется новым сочетанием, выбранная вручную клавиша остаётся.
- macOS: при первом запуске программа сама замечает, что «Универсальный доступ» включён, и начинает работу без перезапуска. Раньше она ждала разрешения, ничего не показывая, и после включения переключателя так и не начинала исправлять, пока её не запустят заново; так было и в 0.66.2. Пока разрешения нет, KeySwitch уже стоит в строке меню: меню говорит, что программа ждёт доступа к клавиатуре, открывает нужный раздел настроек и позволяет выйти.
- Слово, набранное в третьей раскладке (не английской и не русской), остаётся как набрано. Непредвиденная ошибка больше не останавливает программу: забывается только набираемое слово.

### Если нужно вернуться на 0.66.2

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.66.2-<архитектура>.exe` со [страницы выпуска 0.66.2](https://github.com/olegius88/keyswitch/releases/tag/v0.66.2). Настройки и правила сохранятся.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.66.2`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.66.2.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в 0.66.2.

## English

KeySwitch is rewritten in Rust: no Python is left in the program. It recognises the layout as 0.66.2 does, with the same model and word-list files, while the installer is 40% smaller and the program takes three to four and a half times less memory. Settings, rules, history and personal words are kept on update. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.70.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.70.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.70.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.70.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.70.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.70.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.70.0_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.70.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.70.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- The Windows installer is 19.5 MB instead of 31.8 MB, the Ubuntu package 20.4 MB instead of 26.6 MB, the Mac archive 23 MB instead of 37 MB.
- It needs less memory: on Windows 59 MB instead of 264 MB, on Linux 119 MB instead of 333 MB, on a Mac 61 MB instead of 216 MB.
- On Windows the program is ready to switch 3 seconds after start instead of 17.
- Windows: the settings window, the tray icon and the rule windows are drawn with the standard Windows controls.
- Windows: the letters of the tray icon are large and sharp at every display scale: the icon is drawn at the size the notification area shows it (from 16 pixels at 100% to 40 at 250%) and drawn again when the scale changes. With automatic switching off, the icon is grey with dimmed letters, without a white slash across them.
- Windows: «Переключить на русский (RU)» and «Переключить на английский (EN)» (switch to Russian, switch to English) in the tray menu switch the layout of the window last typed in and bring that window back to the front. They used to switch the layout of KeySwitch's hidden window: the program being typed in took the new layout only once it was clicked again, and with «Let me use a different input method for each app window» it kept the old one, or KeySwitch reported «Приложение не подтвердило смену раскладки» (the application did not confirm the layout switch). While the taskbar is in front, the menu offers the switch for the window last typed in, not one by the taskbar's own layout.
- macOS: a correction is shown in Notification Center; the updates page says that a new Mac version is installed by hand from the release page.
- Linux: the rows of the program picker answer a click; in 0.66.2 a click on them did nothing.
- Linux: the settings window no longer hangs when a setting is changed twice in quick succession (two fast clicks on «Включено», or a change from the tray menu while the window is open).
- The layout can be switched with a lone tap of a modifier, as in Punto Switcher and Caramba: the setting «Переключать раскладку одной клавишей» (switch the layout with a single key) on the Hotkeys page (the left or right Shift or Ctrl, off by default). Shift with a letter, Ctrl+C and the system's Alt+Shift or Ctrl+Shift do not switch it; the README says which keys switch the layout.
- macOS: the manual conversion and the double press that offers a rule are on Control+Option+K. They were on Pause, which a Mac keyboard does not have, and did not work; a setting that still holds Pause takes the new shortcut once, a key chosen by hand stays.
- macOS: on the first start the program notices by itself that Accessibility has been turned on and starts working without a relaunch. It used to wait for the permission with nothing on screen and, once the switch was on, still corrected nothing until it was started again; 0.66.2 did the same. While the permission is missing, KeySwitch is already in the menu bar: its menu says that it waits for access to the keyboard, opens the right settings pane and quits.
- A word typed in a third layout, neither English nor Russian, is left as typed. An error nobody foresaw no longer stops the program: only the word being typed is forgotten.

### Going back to 0.66.2

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.66.2-<architecture>.exe` from the [0.66.2 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.66.2) over it. Settings and rules are kept.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.66.2`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.66.2 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in 0.66.2.

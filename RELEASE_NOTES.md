# KeySwitch 0.71.0

## Русский

В KeySwitch появились разовые подсказки о его возможностях. Слова на «х» и «ю», набранные в неверной раскладке, теперь исправляются и без следующего слова, опечатка исправляется вместе с раскладкой чаще, а команды для выделенного текста объясняют, почему не сработали. Настройки, правила, история и личные слова сохраняются при обновлении. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.71.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.71.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.71.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.71.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.71.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.71.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.71.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Разовые подсказки о возможностях: над курсором сразу после исправления, у значка в трее и на страницах окна настроек. Каждая показывается один раз, по одной за раз, не больше трёх в день и не чаще чем раз в 10 минут, не во время набора и никогда не забирая клавиатуру у поля. У каждой есть «Понятно» и «Больше не показывать подсказки», настройка «Показывать подсказки о возможностях» выключает их совсем. В Linux подсказки у значка приходят системными уведомлениями. На то, как решается слово, подсказки не влияют.
- Слово, набранное в неверной раскладке и оканчивающееся на букву с клавиши знака (`nehybhf[` — `турнирах`, `cntgtym.` — `степенью`), исправляется сразу. Раньше оно ждало следующего слова и, набранное отдельно или последним, оставалось как есть.
- Опечатка в слове, набранном в неверной раскладке, исправляется вместе с раскладкой и тогда, когда раскладка сменилась ещё во время набора слова: так происходит с большинством таких слов (`gcb[jkujbb` — `психологии`).
- Команды для выделенного текста говорят уведомлением, почему не сработали: нет выделения, несколько строк, знаки, которых нет на раскладках, поле нельзя изменить. Раньше причина была видна только в окне настроек.
- Windows: первая команда для выделенного текста в окне Chrome, Edge или программы на Electron срабатывает сразу. Раньше первое нажатие в таком окне ничего не делало.
- GNOME на Wayland: чтобы окно подсказки не забирало фокус на долю секунды, после обновления выйдите из учётной записи и войдите снова: оболочка загружает обновлённое расширение KeySwitch только при входе.

### Если нужно вернуться на 0.70.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.70.0-<архитектура>.exe` со [страницы выпуска 0.70.0](https://github.com/olegius88/keyswitch/releases/tag/v0.70.0). Настройки и правила сохранятся.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.70.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.70.0.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в 0.70.0.

## English

KeySwitch now shows one-time hints about what it can do. Words ending in «х» or «ю» typed in the wrong layout are corrected without waiting for the next word, a slip is corrected together with the layout more often, and the selection commands say why they did not work. Settings, rules, history and personal words are kept on update. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.71.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.71.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.71.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.71.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.71.0_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.71.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.71.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- One-time hints about what the program can do: above the caret right after a correction, near the tray icon, and on the pages of the settings window. Each shows once, one at a time, at most three a day and ten minutes apart, never while a word is being typed and never taking the keyboard from the field. Each has «Понятно» (got it) and «Больше не показывать подсказки» (no more hints), and the setting «Показывать подсказки о возможностях» (show hints about features) turns them off. Near the tray icon on Linux they are system notifications. The hints do not change how a word is decided.
- A word typed in the wrong layout whose last letter sits on a sign key (`nehybhf[` is `турнирах`, `cntgtym.` is `степенью`) is corrected at once. It used to wait for the next word, and typed alone or last it stayed as typed.
- A slip in a word typed in the wrong layout is corrected together with the layout also when the layout changed while the word was being typed, as it does for most such words (`gcb[jkujbb` is `психологии`).
- The selection commands say in a notification why they did not work: nothing selected, several lines, characters no layout has, a field that cannot be changed. The reason used to show only in the settings window.
- Windows: the first selection command in a window of Chrome, Edge or an Electron program works at once. The first press in such a window used to do nothing.
- GNOME on Wayland: so that a hint window does not take the focus for a fraction of a second, log out and back in after updating: the shell loads the updated KeySwitch extension only at login.

### Going back to 0.70.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.70.0-<architecture>.exe` from the [0.70.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.70.0) over it. Settings and rules are kept.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.70.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.70.0 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in 0.70.0.

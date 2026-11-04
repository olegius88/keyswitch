# KeySwitch 0.66.1

## Русский

Этот выпуск — про ресурсы: KeySwitch занимает примерно на 120 МБ меньше памяти, пакеты стали меньше, а в Windows окно настроек и значок в трее появляются сразу после запуска, пока языковые модели загружаются в фоне. Распознавание раскладки и модели — те же, что в 0.65.0. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.66.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.66.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.66.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.66.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.66.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.66.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.66.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.66.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.66.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Памяти нужно примерно на 120 МБ меньше: таблицы буквосочетаний модели правописания, около миллиона записей, хранятся в двух компактных массивах вместо миллиона отдельных объектов, а в Linux память, освобождённая при загрузке моделей, возвращается системе. После запуска движок занимает около 300 МБ вместо 420 МБ (замер в Linux), и все решения те же, что раньше.
- Windows: окно настроек и значок в трее появляются сразу после запуска, а языковые модели загружаются в фоне. Пока они загружаются, окно и диагностика пишут «Загрузка языковых моделей…», а переключение начинается, как только модели готовы.

- Пакеты стали меньше: установщик для Windows x64 — 31,9 МБ вместо 36,8 МБ в 0.65.0, ZIP — 41,9 МБ вместо 52,1 МБ, установленная программа — 90 МБ вместо 115 МБ; для ARM 30,9 МБ и 40,9 МБ вместо 33,8 МБ и 46,0 МБ, для x86 30,2 МБ и 39,6 МБ вместо 34,0 МБ и 48,7 МБ; пакет Debian — 26,5 МБ вместо 29,6 МБ. Код KeySwitch теперь хранится байт-кодом Python, а не компилируется в C: исполняемый файл Windows меньше на 16 МБ, а движок строится не медленнее (замер в Linux). Строк документации в сборках нет, а в сборки Windows входят только те графические форматы, которые нужны значку в трее.
- В Windows личная память программы сразу после запуска — около 230 МБ вместо примерно 325 МБ в 0.65.0, всего выделено около 250 МБ вместо 350 МБ (собранные exe для x64 в одной виртуальной машине).
- Программа, закрытая в первые секунды после запуска, теперь завершается надёжно: проверка установки 0.66.0 на Windows для ARM не заканчивалась, поэтому 0.66.0 не вышел, и этот выпуск включает всё, что в нём было.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — пара 0.65.0 байт в байт, заново запечатанная для изменённого движка. На журналах реального набора новый движок решает каждое слово так же, как 0.65.0. На тесте v85, прочитанном один раз, без ранней смены раскладки восстановлены 746 и 747 из 751 строки, набранной не в той раскладке, против 739 и 743 у базовой пары, и не испорчено ни одной правильной строки (у базовой пары — по одной в каждом профиле).

## English

This release is about resources: KeySwitch takes about 120 MB less memory, the packages are smaller, and on Windows the settings window and the tray icon appear right after start while the language models load in the background. Layout recognition and the models are those of 0.65.0. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.66.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.66.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.66.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.66.1-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.66.1-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.66.1-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.66.1_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.66.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.66.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- KeySwitch takes about 120 MB less memory: the letter-sequence tables of the spelling model, about a million entries, are kept in two compact arrays instead of a million separate objects, and on Linux the memory freed while the models load is handed back to the system. After start the engine holds about 300 MB instead of 420 MB (measured on Linux), and every decision is the same as before.
- Windows: the settings window and the tray icon appear right after start, while the language models load in the background. Until they are loaded the window and its diagnostics say «Загрузка языковых моделей…», and switching starts as soon as they are.

- The packages are smaller: the Windows x64 installer is 31.9 MB instead of 36.8 MB in 0.65.0, its ZIP 41.9 MB instead of 52.1 MB and the installed program 90 MB instead of 115 MB; on Arm 30.9 MB and 40.9 MB instead of 33.8 MB and 46.0 MB, on x86 30.2 MB and 39.6 MB instead of 34.0 MB and 48.7 MB; the Debian package 26.5 MB instead of 29.6 MB. KeySwitch's own code stays Python bytecode instead of being compiled to C: the Windows executable is 16 MB smaller and the engine builds no slower (measured on Linux). The builds carry no docstrings, and the Windows ones only the image formats the tray icon uses.
- On Windows the program's private memory right after start is about 230 MB instead of about 325 MB in 0.65.0, and it commits about 250 MB instead of 350 MB (the built x64 executables in one virtual machine).
- A program closed in its first seconds now ends reliably: the check of the installed 0.66.0 did not finish on Windows on Arm, so 0.66.0 was not published, and this release carries everything it had.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the pair of 0.65.0 byte for byte, sealed again for the changed engine. On the recorded real typing the new engine decides every word as 0.65.0 does. On sealed test v85, read once, it restores 746 and 747 of 751 rows typed in the wrong layout with the early switch off, against 739 and 743 for the baseline pair, and spoils no correct row (the baseline pair one in each profile).

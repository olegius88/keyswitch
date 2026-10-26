# KeySwitch 0.62.1

## Русский

Это исправление безопасности для Windows: библиотека Pillow, которой KeySwitch рисует значки, обновлена с 12.3.0 до 13.0.0, где закрыты опубликованные уязвимости старых версий. Распознавание раскладки и модели — те же, что в 0.62.0. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.62.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.62.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.62.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.62.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.62.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.62.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.62.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.62.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.62.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Pillow, которой сборки для Windows рисуют значок в трее и значок программы, обновлена с 12.3.0 до 13.0.0. В версиях до 13.0.0 опубликованы уязвимости в чтении и записи файлов изображений (JPEG, TIFF, PDF, GIF и другие форматы): ошибки работы с памятью и бесконечные циклы. KeySwitch рисует ими только свои значки и не открывает файлов изображений, но библиотека входит в установщики и архивы для Windows. Значки выглядят точно так же, как раньше.
- Запуск KeySwitch из исходников в Windows (`pip install -e ".[windows]"`) теперь требует Python 3.11 или новее: Pillow 13.0.0 больше не поддерживает Python 3.10.

### Модель

- Модели те же, что в 0.62.0: контекстная модель `context-v3-b876e862bbfe` и модель префиксов `prefix-v2-bf3dc28f8567`.

## English

This is a security fix for Windows: the Pillow library KeySwitch draws its icons with is updated from 12.3.0 to 13.0.0, which fixes the vulnerabilities published against older versions. Layout detection and the models are those of 0.62.0. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.62.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.62.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.62.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.62.1-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.62.1-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.62.1-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.62.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.62.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.62.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- Pillow, which the Windows builds use to draw the tray icon and the application icon, is updated from 12.3.0 to 13.0.0. Vulnerabilities in reading and writing image files (JPEG, TIFF, PDF, GIF and other formats) are published against the versions before 13.0.0: memory errors and endless loops. KeySwitch draws only its own icons with it and opens no image files, but the library is packed into the Windows installers and archives. The icons look exactly as before.
- Running KeySwitch from source on Windows (`pip install -e ".[windows]"`) now needs Python 3.11 or newer: Pillow 13.0.0 no longer supports Python 3.10.

### Model

- The models are those of 0.62.0: the context model `context-v3-b876e862bbfe` and the prefix model `prefix-v2-bf3dc28f8567`.

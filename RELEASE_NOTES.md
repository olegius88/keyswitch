# KeySwitch 0.66.2

## Русский

Этот выпуск исправляет запуск на Mac: опубликованные архивы для macOS падали сразу после открытия, теперь программа запускается. В Windows и Linux ничего не меняется, распознавание раскладки и модели — те же, что в 0.66.1. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.66.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.66.2-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.66.2-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.66.2-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.66.2-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.66.2-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.66.2_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.66.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.66.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- macOS: программа из опубликованного архива запускается. Все прежние архивы падали при старте: в сборку не попадали модуль окна настроек и библиотека Tk, а проверка сборки открывала только диагностику и этого не видела. Теперь сборка на каждой архитектуре открывает окно упакованной программы и не проходит, если оно не открылось.
- macOS: автозагрузка работает. Программа из архива записывала в задачу запуска несуществующий файл `python3`, поэтому при входе в систему не запускалась.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в 0.66.1.

## English

This release fixes the start on a Mac: the published macOS archives crashed right after opening, and now the program starts. Nothing changes on Windows and Linux, and layout recognition and the models are those of 0.66.1. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.66.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.66.2-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.66.2-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.66.2-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.66.2-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.66.2-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.66.2_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.66.2-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.66.2-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- macOS: the program from the published archive starts. Every earlier archive crashed at start: the settings window's module and the Tk library were left out of the build, and the build's check opened only the diagnostics and did not see it. The build now opens the window of the packaged program on each architecture and fails if it does not open.
- macOS: autostart works. The program from the archive wrote a `python3` that does not exist into its launch agent, so it did not start at login.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in 0.66.1.

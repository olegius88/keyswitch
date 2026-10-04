# KeySwitch 0.38.1

## Русский

Выпуск сборок, сама программа не менялась: распознавание и модели те же, что в 0.38.0. Для
Windows на ARM появилась собственная сборка, а приложение для Mac теперь нотаризовано Apple и
открывается обычным двойным щелчком. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.38.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.38.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.38.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.38.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `keyswitch_0.38.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.38.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.38.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Windows на ARM

- Для компьютеров на ARM теперь есть родная сборка вместо x64 через эмуляцию. Её собирают и
  проверяют на Windows 11 на ARM так же, как x64: тесты Win32, исправление раскладки в окне
  через перехват клавиатуры, тихая установка, обновление и удаление.
- x64-установщик на таком компьютере по-прежнему ставится, но программа тогда работает через
  эмуляцию.
- Автоматических обновлений у ARM64-сборки пока нет: проверка обновлений сообщает, что
  архитектура не поддерживается. Новую версию скачивайте со страницы выпусков.

### macOS

- Приложение подписано сертификатом Developer ID и теперь нотаризовано Apple, поэтому macOS
  открывает его сразу. Раньше при первом запуске приходилось подтверждать открытие в «Системных
  настройках» → «Конфиденциальность и безопасность».

## English

A packaging release; the program itself did not change: recognition and models are those of
0.38.0. Windows on Arm gets a build of its own, and the Mac application is now notarized by
Apple and opens with an ordinary double click. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.38.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.38.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.38.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.38.1-windows-arm64.zip` — portable archive for Windows on Arm.
- `keyswitch_0.38.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.38.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.38.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Windows on Arm

- Arm computers now get a native build instead of the x64 one under emulation. It is built and
  checked on Windows 11 on Arm the way the x64 one is: the Win32 tests, a layout correction in a
  window through the keyboard hook, a silent install, update and uninstall.
- The x64 installer still installs on such a computer, but the program then runs under
  emulation.
- The ARM64 build does not update itself yet: its update check reports that the architecture is
  not supported. Download a new version from the releases page.

### macOS

- The application is signed with a Developer ID certificate and now notarized by Apple, so macOS
  opens it at once. Before, the first start had to be confirmed in System Settings → Privacy &
  Security.

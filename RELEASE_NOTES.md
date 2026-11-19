# KeySwitch 0.74.1

## Русский

Этот выпуск исправляет две ошибки при наборе английских терминов внутри русского текста. Термин из букв и цифр, после которого запятая набрана в русской раскладке, теперь исправляется вместе с запятой: `шзм6б то` становится `ipv6, то`. А буква, нажатая, пока программа исправляла слово перед ней, больше не заставляет программу забыть фразу: `и` между двумя терминами снова становится `и`, а не остаётся `b`.

### Файлы выпуска

- `KeySwitch-Setup-0.74.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.74.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.74.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.74.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.74.1_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.74.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.74.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Термин из букв и цифр, набранный в русской раскладке, исправляется вместе с запятой или точкой, набранной сразу после него: `шзм6б` перед продолжением русской фразы становится `ipv6,`, а `ч64б` — `x64,`, так же как `шзм6` без запятой становился `ipv6`. Запятая в русской раскладке — это буква `б`, и с ней модель не видела в слове термина из букв и цифр и оставляла его как набрано. Цифра с буквой в русском тексте (`5б`) остаются как есть.
- Буква, нажатая, пока программа исправляла слово перед ней, и отпущенная уже после исправления, больше не стирает из памяти программы набранную фразу. Программа принимала отпускание такой клавиши за щелчок мышью, забывала слова перед ней и решала следующее слово без них: в `для шзм6 и socks5`, набранном в русской раскладке, `шзм6` становилось `ipv6`, а `и`, нажатое в этот момент, оставалось `b`. Теперь следующее слово решается с текстом перед ним, и получается `для ipv6 и socks5`.

### Если нужно вернуться на 0.74.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.74.0-<архитектура>.exe` со [страницы выпуска 0.74.0](https://github.com/olegius88/keyswitch/releases/tag/v0.74.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.74.0.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`. Изменилось только то, что программа показывает контекстной модели: термин из букв и цифр с запятой или точкой после него, набранной в русской раскладке, модель видит без этого знака.

## English

This release fixes two mistakes in typing English terms inside Russian text. A term of letters and digits with a comma typed after it in the Russian layout is now converted together with the comma: `шзм6б то` becomes `ipv6, то`. And a letter pressed while KeySwitch corrected the word before it no longer makes it forget the sentence: `и` between two terms becomes `и` again instead of staying `b`.

### Release files

- `KeySwitch-Setup-0.74.1-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.74.1-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.74.1-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.74.1-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.74.1-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.74.1-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.74.1_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.74.1-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.74.1-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- A term of letters and digits typed in the Russian layout is converted with the comma or full stop typed right after it: `шзм6б` before the rest of a Russian sentence becomes `ipv6,` and `ч64б` becomes `x64,`, as `шзм6` without the comma became `ipv6`. The comma typed in the Russian layout is the letter `б`, and with it the model saw no term of letters and digits in the word and left it as typed. A digit and a letter of Russian text (`5б`) stay as they are.
- A letter pressed while KeySwitch corrected the word before it, and let go once the correction was typed, no longer wipes the sentence from what KeySwitch remembers. It took the release of such a key for a click, forgot the words before it and decided the next word without them: in `для шзм6 и socks5` typed in the Russian layout, `шзм6` became `ipv6` and the `и` pressed at that moment stayed `b`. The next word is now decided with the text before it, and the result is `для ipv6 и socks5`.

### Going back to 0.74.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.74.0-<architecture>.exe` from the [0.74.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.74.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.74.0 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`. Only what KeySwitch shows the context model changed: a term of letters and digits with a comma or full stop after it typed in the Russian layout is shown to the model without that sign.

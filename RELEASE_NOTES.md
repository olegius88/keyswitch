# KeySwitch 0.75.1

## Русский

В этом выпуске кавычка в Telegram больше не превращается в «@», когда её ставят в уже написанный текст: вокруг слова, к которому вернулись стрелками или мышью, и вокруг вставленного текста.

### Файлы выпуска

- `KeySwitch-Setup-0.75.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.75.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.75.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.75.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.75.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.75.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.75.1_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.75.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.75.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Telegram: кавычка, поставленная в уже написанный текст, остаётся кавычкой. Если KeySwitch может прочитать поле, кавычка сразу после слова, знака или вставленного текста закрывает цитату, а кавычка прямо перед словом, скобкой или «ёлочками» её открывает: `привет"` и `"привет мир` после стрелок, Home, End или щелчка мышью, после Delete или Backspace, и `"текст"` вокруг вставки. Раньше каждая такая кавычка становилась «@», и её приходилось возвращать клавишей Pause по одной. Кавычка в начале строки, после пробела или перевода строки, между пробелами или перед запятой по-прежнему сразу показывается как «@» и открывает список участников, а если стереть эту «@» Backspace и набрать кавычку снова, список откроется снова. Pause на оставленной кавычке превращает её в «@». Где поле прочитать нельзя, кавычка после перемещения курсора клавишами не трогается, как и слово после такого перемещения.

### Если нужно вернуться на 0.75.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.75.0-<архитектура>.exe` со [страницы выпуска 0.75.0](https://github.com/olegius88/keyswitch/releases/tag/v0.75.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.75.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.75.0.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`.

## English

In this release a quote in Telegram no longer turns into «@» when it is put into text already written: around a word the caret was moved back to with the arrows or the mouse, and around pasted text.

### Release files

- `KeySwitch-Setup-0.75.1-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.75.1-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.75.1-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.75.1-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.75.1-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.75.1-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.75.1_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.75.1-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.75.1-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- Telegram: a quote put into text already written stays a quote. Where KeySwitch can read the field, a quote right after a word, a sign or pasted text closes a quotation, and one right before a word, a bracket or guillemets opens one: `привет"` and `"привет мир` after the arrows, Home, End or a click, after Delete or Backspace, and `"текст"` around a paste. Each such quote became «@» and had to be put back with Pause one by one. A quote at the start of a line, after a space or a line break, between two spaces or before a comma still shows as «@» at once and opens the list of people, and erasing that «@» with Backspace and typing the quote again opens the list again. Pause on a quote kept so turns it into «@». Where the field cannot be read, a quote typed after the caret was moved with the keys is left alone, as the word after such a move is.

### Going back to 0.75.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.75.0-<architecture>.exe` from the [0.75.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.75.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.75.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.75.0 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`.

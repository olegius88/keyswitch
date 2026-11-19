# KeySwitch 0.74.3

## Русский

В этом выпуске набор на Windows тратит меньше процессора, а клавиша отмены после слова через дефис возвращает его целиком, как оно было набрано.

### Файлы выпуска

- `KeySwitch-Setup-0.74.3-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.74.3-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.74.3-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.3-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.74.3-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.3-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.74.3_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.74.3-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.74.3-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Windows: набор тратит меньше процессора, и перехватчик клавиш держит каждую клавишу чуть меньше. Раньше на каждую клавишу он до четырёх раз спрашивал систему, какое окно впереди, и дважды переводил клавишу в обеих раскладках; теперь спрашивает один раз и запоминает, какие символы даёт клавиша в каждой раскладке. Текст поля, в котором идёт набор, читается одним запросом сразу за свойствами поля и его текстом, а не отдельным запросом на каждое свойство. На тестовой машине процессора на набор ушло примерно на 9 % меньше, перехватчик держал клавишу примерно на 14 % меньше, а чтение поля стало быстрее на 9–21 %; поле читается так же, как раньше.
- Клавиша отмены возвращает целиком слово через дефис, начало которого программа перевела ещё во время набора. `ьщтпщ-сервиса`, набранное в русской раскладке, как и раньше становится `mongo-сервиса`: начало `mongo` переводится по ходу набора, а слово после дефиса возвращается в русскую раскладку на пробеле. Раньше отмена возвращала только слово после дефиса и оставляла `mongo-cthdbcf` в английской раскладке; теперь она возвращает `ьщтпщ-сервиса`, все клавиши так, как они были набраны, — так же, как после слова без дефиса.

### Если нужно вернуться на 0.74.2

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.74.2-<архитектура>.exe` со [страницы выпуска 0.74.2](https://github.com/olegius88/keyswitch/releases/tag/v0.74.2).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.2`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.74.2.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`.

## English

In this release typing on Windows takes less processor time, and the undo hotkey after a hyphenated word puts it back whole, as it was typed.

### Release files

- `KeySwitch-Setup-0.74.3-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.74.3-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.74.3-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.74.3-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.74.3-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.74.3-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.74.3_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.74.3-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.74.3-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- Windows: typing takes less processor time, and the keyboard hook holds each key a little shorter. For every key the hook asked the system up to four times which window was in front and translated the key twice in both layouts; it now asks once and keeps the characters a key gives in each layout. The text of the field being typed in is read with one request for the field's properties and text together instead of one request for each property. On a test machine typing took about 9 % less processor time, the hook held a key about 14 % shorter, and a read of a field was 9–21 % faster; the field is read as before.
- The undo hotkey puts back the whole of a hyphenated word whose start KeySwitch converted while it was typed. `ьщтпщ-сервиса` typed in the Russian layout becomes `mongo-сервиса` as before: the start `mongo` is converted while it is typed, and the word after the hyphen goes back to the Russian layout at the space. Undo put back only the word after the hyphen and left `mongo-cthdbcf` in the English layout; it now puts back `ьщтпщ-сервиса`, every key as it was typed, as it does after a word without a hyphen.

### Going back to 0.74.2

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.74.2-<architecture>.exe` from the [0.74.2 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.74.2).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.2`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.74.2 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`.

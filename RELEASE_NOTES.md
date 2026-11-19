# KeySwitch 0.74.2

## Русский

Этот выпуск исправляет английский термин, набранный в русской раскладке перед дефисом и русским словом: `зкщ-тарифу` теперь становится `pro-тарифу`, а `ыущ-оптимизация` — `seo-оптимизация`.

### Файлы выпуска

- `KeySwitch-Setup-0.74.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.74.2-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.74.2-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.2-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.74.2-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.2-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.74.2_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.74.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.74.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Английский термин, набранный в русской раскладке и соединённый дефисом с русским словом после него, переводится отдельно: `зкщ-тарифу` становится `pro-тарифу`, `ыущ-оптимизация` — `seo-оптимизация`, а `ьщтпщ-сервиса` — `mongo-сервиса`. Слово после дефиса остаётся как набрано, и раскладка остаётся русской. Раньше программа решала только о токене целиком, а ни одно его прочтение не было словом (`pro-nfhbae` тоже не слово), и токен оставался как набран, хотя `зкщ` отдельно становилось `pro`. Теперь, если часть после дефиса — слово в той раскладке, в которой она набрана, программа отдельно спрашивает модель о части до дефиса. Русские слова через дефис (`что-то`, `экс-чемпион`) остаются как есть, а клавиша отмены возвращает токен таким, каким он был набран.

### Если нужно вернуться на 0.74.1

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.74.1-<архитектура>.exe` со [страницы выпуска 0.74.1](https://github.com/olegius88/keyswitch/releases/tag/v0.74.1).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.1`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.74.1.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`. Изменилось только то, о чём программа спрашивает контекстную модель: о части слова до дефиса она теперь спрашивает отдельно.

## English

This release fixes an English term typed in the Russian layout before a hyphen and a Russian word: `зкщ-тарифу` now becomes `pro-тарифу`, and `ыущ-оптимизация` becomes `seo-оптимизация`.

### Release files

- `KeySwitch-Setup-0.74.2-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.74.2-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.74.2-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.74.2-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.74.2-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.74.2-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.74.2_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.74.2-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.74.2-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- An English term typed in the Russian layout and joined by a hyphen to the Russian word after it is converted alone: `зкщ-тарифу` becomes `pro-тарифу`, `ыущ-оптимизация` becomes `seo-оптимизация` and `ьщтпщ-сервиса` becomes `mongo-сервиса`. The word after the hyphen stays as typed, and the layout stays Russian. KeySwitch decided about the whole token only, and neither of its readings was a word (`pro-nfhbae` is none either), so the token was left as typed, while `зкщ` alone became `pro`. Now, when the part after the hyphen is a word in the layout it was typed in, KeySwitch asks the model about the part before the hyphen alone. Russian words joined by a hyphen (`что-то`, `экс-чемпион`) stay as they are, and the undo hotkey puts the token back as it was typed.

### Going back to 0.74.1

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.74.1-<architecture>.exe` from the [0.74.1 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.74.1).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.1`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.74.1 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`. Only what KeySwitch asks the context model changed: it now asks about the part of a word before a hyphen alone.

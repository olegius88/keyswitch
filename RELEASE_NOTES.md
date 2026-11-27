# KeySwitch 0.76.0

## Русский

В этом выпуске на Windows слово, которое KeySwitch переводит прямо во время набора, показывается в другой раскладке примерно на 40 мс раньше.

### Файлы выпуска

- `KeySwitch-Setup-0.76.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.76.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.76.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.76.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.76.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.76.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.76.0_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.76.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.76.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Windows: слово, которое переводится прямо во время набора, показывается в другой раскладке примерно на 40 мс раньше. Раньше новые буквы ждали 50 мс после того, как старые стёрты, пока окно примет новую раскладку, и всё это время слово стояло наполовину стёртым. Теперь, если KeySwitch только что прочитал поле и увидел в нём слово, короткое слово из одних букв получает новые буквы символами вместе со стиранием, сразу после того как окно подтвердило раскладку. Если в этот момент зажата клавиша, если среди клавиш слова были знаки (`,` для `б`, `;` для `ж`), если замена длиннее или после слова уже набраны клавиши, а также в окнах удалённого доступа буквы набираются клавишами, как раньше. Проверено в Блокноте, Edge, VS Code и Windows Terminal на Windows 11.

### Если нужно вернуться на 0.75.1

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.75.1-<архитектура>.exe` со [страницы выпуска 0.75.1](https://github.com/olegius88/keyswitch/releases/tag/v0.75.1).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.75.1`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.75.1.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`.

## English

In this release on Windows a word KeySwitch converts while it is typed shows in the other layout about 40 ms sooner.

### Release files

- `KeySwitch-Setup-0.76.0-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.76.0-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.76.0-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.76.0-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.76.0-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.76.0-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.76.0_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.76.0-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.76.0-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- Windows: a word converted while it is typed shows in the other layout about 40 ms sooner. The new letters used to wait 50 ms after the old ones were erased, for the window to take the new layout, and the word stood half erased meanwhile. Where KeySwitch has just read the field and seen the word there, a short word of letters only now gets its new letters as characters together with the erasing, right after the window has confirmed the layout. When a key is held down at that moment, when the word's keys typed signs (`,` for `б`, `;` for `ж`), for a longer replacement or one with keys typed after it, and in remote-control windows, the letters are typed as keys, as before. Checked in Notepad, Edge, VS Code and Windows Terminal on Windows 11.

### Going back to 0.75.1

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.75.1-<architecture>.exe` from the [0.75.1 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.75.1).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.75.1`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.75.1 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`.

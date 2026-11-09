# KeySwitch 0.71.1

## Русский

Слово после дефиса снова решается отдельно: `proxy-ghbkj;tybt` становится `proxy-приложение`, а не `зкщчн-приложение`. В Linux Mint раскладка снова переключается. Настройки, правила, история и личные слова сохраняются при обновлении. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.71.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.71.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.71.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.71.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.71.1_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.71.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.71.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Слово после дефиса решается как отдельное слово и там, где программа читает поле ввода, то есть в большинстве окон: `proxy-ghbkj;tybt` становится `proxy-приложение`, `docker-j,hfp` — `docker-образ`. Раньше часть до дефиса проверялась по полю, где уже стояло всё слово, проверка отказывала, и меняли всё слово целиком: `зкщчн-приложение`. Из 500 составленных для проверки терминов с русским словом через дефис, набранных целиком в английской раскладке, неверно выходило 325, теперь 8.
- Раскладка, выбранная вручную, защищает слово только до дефиса: выбрали английскую для `frontend`, а `-магазин` набрали, не переключив обратно, — получится `frontend-магазин`. Раньше всё слово оставалось как набрано. Слова через дефис, набранные правильно в выбранной раскладке (`built-in`, `что-то`), остаются как есть.
- Linux Mint (Cinnamon, сеанс X11): раскладка переключается через сам Cinnamon, и исправление набирается, когда он её сменил. Cinnamon сразу возвращал раскладку, которую ставила программа, поэтому в Linux Mint 22.3 исправленное слово набиралось наполовину в старой раскладке (`ghbdtn vbh` становилось `ghbdtn vbи`), а раскладка оставалась английской.

### Если нужно вернуться на 0.71.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.71.0-<архитектура>.exe` со [страницы выпуска 0.71.0](https://github.com/olegius88/keyswitch/releases/tag/v0.71.0). Настройки и правила сохранятся.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.71.0.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в 0.70.0 и 0.71.0.

## English

A word after a hyphen is decided on its own again: `proxy-ghbkj;tybt` becomes `proxy-приложение`, not `зкщчн-приложение`. On Linux Mint the layout switches again. Settings, rules, history and personal words are kept on update. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.71.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.71.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.71.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.1-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.71.1-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.1-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.71.1_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon) on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.71.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.71.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A word after a hyphen is decided as a word of its own also where the program reads the field, as it does in most windows: `proxy-ghbkj;tybt` becomes `proxy-приложение`, `docker-j,hfp` becomes `docker-образ`. The part before the hyphen used to be checked against a field that already showed the whole token, the check refused, and the whole token converted: `зкщчн-приложение`. Of 500 authored terms joined by a hyphen to a Russian word and typed all in the English layout, 325 came out wrong, and 8 do now.
- A layout picked by hand protects the word only up to a hyphen: picked English for `frontend` and typed `-магазин` without switching back, you get `frontend-магазин`. The whole token used to stay as typed. Words joined by a hyphen and typed right in the picked layout (`built-in`, `что-то`) stay as they are.
- Linux Mint (Cinnamon on X11): the layout is switched through Cinnamon itself, and a correction is typed once Cinnamon has switched it. Cinnamon put back at once a layout the program set, so on Linux Mint 22.3 a corrected word was typed half in the old layout (`ghbdtn vbh` became `ghbdtn vbи`), and the layout stayed English.

### Going back to 0.71.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.71.0-<architecture>.exe` from the [0.71.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.71.0) over it. Settings and rules are kept.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.71.0 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in 0.70.0 and 0.71.0.

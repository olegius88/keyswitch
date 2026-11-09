# KeySwitch 0.71.2

## Русский

Набранное слово больше не теряется в Linux Mint и в программах на GTK 4, когда фокус ввода переходит внутри одного окна, а отложенное короткое слово исправляется и после ручной смены раскладки. Окно настроек сразу говорит, почему горячая клавиша не сработает, флаги в трее стали прямоугольными, а в заголовке окна видна версия. Настройки, правила, история и личные слова сохраняются при обновлении. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.71.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.71.2-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.71.2-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.2-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.71.2-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.71.2-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.71.2_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.71.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.71.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Короткое слово, которое программа отложила до следующего слова, теперь исправляется и тогда, когда раскладку переключили вручную и следующее слово набрали уже в ней: после `olegius888 ? z ` и ручной смены на русскую `ожидал,` получится `я ожидал,`, а не `z ожидал,`. Слово, набранное после ручной смены, и сама выбранная раскладка остаются как были; Pause сразу после исправления или отмена возвращают текст как он был набран.
- Linux (сеанс X11): набранное слово больше не забывается, когда программа переносит фокус ввода между своим окном и его внутренней частью. Так делают окна на GTK 4 в Cinnamon после смены раскладки, и KeySwitch принимал это за переход в другое окно. В Linux Mint 22.3 второе нажатие Pause сразу после преобразования слова переключало раскладку вместо того, чтобы предложить правило: в 4 прогонах проверки из 8, теперь ни в одном из 8.
- Горячая клавиша, которая не может сработать, больше не сохраняется молча. Сочетанию нужна обычная клавиша кроме Ctrl, Alt, Shift и Super, и такая, какая есть на клавиатуре: `Ctrl`, `Ctrl+Alt` или `Левый Ctrl` в поле «Преобразовать последнее слово» сохранялись как есть и ничего не делали. Теперь окно настроек говорит, что не так, и возвращает в поле прежнее сочетание.
- Windows и macOS: функциональные клавиши работают в сочетаниях по своим именам — `F9`, `Ctrl+F12` (F1–F24 в Windows, F1–F20 в macOS). Раньше программа знала их только под именами вида `VK_78`, и сочетание `F9` не срабатывало. В Linux эти имена уже были.
- Флаги в значке трея стали прямоугольными, как настоящие флаги, а не квадратными: 16 на 11 пикселей при обычном масштабе. В Windows полосы и звёзды американского флага ложатся на целые пиксели значка при любом масштабе экрана, а черта на флаге выключенной программы видна и на белой полосе.
- В заголовке окна настроек видна версия: «KeySwitch 0.71.2» вместо «KeySwitch». В Linux заголовок теперь виден и в самом окне, раньше там не было названия.

### Если нужно вернуться на 0.71.1

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.71.1-<архитектура>.exe` со [страницы выпуска 0.71.1](https://github.com/olegius88/keyswitch/releases/tag/v0.71.1). Настройки и правила сохранятся.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.1`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.71.1.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в 0.70.0, 0.71.0 и 0.71.1.

## English

A word being typed is no longer lost on Linux Mint and in GTK 4 programs when the keyboard focus moves inside one window, and a waiting short word is converted also after a layout switched by hand. The settings window says at once why a hotkey will not fire, the flags in the tray are wider than tall, and the window title names the version. Settings, rules, history and personal words are kept through the update. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.71.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.71.2-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.71.2-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.2-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.71.2-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.71.2-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.71.2_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon) on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.71.2-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.71.2-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A short word the program left waiting for the next word is now converted also when the layout is switched by hand and the next word is typed in it: after `olegius888 ? z ` and a switch to Russian by hand, `ожидал,` gives `я ожидал,`, not `z ожидал,`. The word typed after the switch and the layout picked stay as they were; Pause right after the correction or the undo hotkey give the text back as it was typed.
- Linux (X11 session): the word being typed is no longer forgotten when a program moves the keyboard focus between its window and an inner part of it. GTK 4 windows do so in Cinnamon after a layout switch, and KeySwitch took it for a move to another window. On Linux Mint 22.3 a second Pause right after a word was converted switched the layout instead of offering a rule: in 4 runs of the check out of 8, and in none of 8 now.
- A hotkey that can never fire is no longer stored without a word. A hotkey needs a key besides Ctrl, Alt, Shift and Super, and one the keyboard has: `Ctrl`, `Ctrl+Alt` or `Левый Ctrl` typed into «Преобразовать последнее слово» (convert the last word) were stored as they were and did nothing. The settings window now says what is wrong and puts the previous hotkey back into the field.
- Windows and macOS: the function keys work in hotkeys by their names, `F9` or `Ctrl+F12` (F1 to F24 on Windows, F1 to F20 on macOS). The program knew them only by names such as `VK_78`, so a hotkey spelled `F9` never fired. Linux had these names already.
- The flags of the tray icon are wider than tall, as real flags are, and no longer squares: 16 by 11 pixels at the usual scale. On Windows the stripes and the stars of the American flag fall on whole pixels of the icon at every scale of the screen, and the slash across the flag of a paused program shows on the white band too.
- The title of the settings window names the version: «KeySwitch 0.71.2» instead of «KeySwitch». On Linux the window itself now shows the title too; it showed no name before.

### Going back to 0.71.1

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.71.1-<architecture>.exe` from the [0.71.1 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.71.1) over it. Settings and rules are kept.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.1`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.71.1 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in 0.70.0, 0.71.0 and 0.71.1.

# KeySwitch 0.64.1

## Русский

Это исправление для Windows: KeySwitch 0.64.0 на ноутбуке часто забывал набираемое слово, и слова посреди фразы оставались неисправленными, а Pause только переключал раскладку. Распознавание раскладки и модели — те же, что в 0.64.0. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.64.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.64.1-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.64.1-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.64.1-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.64.1-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.64.1-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.64.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (что это даёт — ниже); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.64.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.64.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Windows: KeySwitch больше не забывает набираемое слово, когда система сообщает о вводе, которого не получили его перехватчики клавиатуры и мыши. 0.64.0 каждый раз отпускал слово, текст перед ним и последнее исправление, а на ноутбуке такой ввод приходил часто, прямо во время набора и в любой программе: слова посреди фразы оставались неисправленными, Pause не находил слова и только переключал раскладку, а следующее слово после этого оставалось как набрано. Журнал по-прежнему один раз называет программу, ввод в которую не доходит ни до одного перехватчика, — и только когда время последнего ввода системы сдвинулось несколько раз подряд, а до перехватчиков не дошло ни одного события, как бывает, когда клавиши в такой программе проходят мимо них; одиночный ввод между клавишами, которые перехватчики получили, за такую программу больше не принимается.

### Модель

- Контекстная модель `context-v3-b380007b5e3e` и модель префиксов `prefix-v2-bf3dc28f8567` — те же, что в 0.64.0, байт в байт: пара запечатана заново для изменённого движка. На журналах реального набора новый движок не портит ни одного слова. На запечатанном тесте v80, прочитанном один раз, без ранней смены раскладки восстановлены все 174 строки, набранные не в той раскладке, в обоих профилях, против 173 у базовой пары; правильный текст не испорчен ни разу (у базовой пары — одна строка).

## English

This is a fix for Windows: on a laptop KeySwitch 0.64.0 often forgot the word being typed, so words in the middle of a sentence were left uncorrected and Pause only switched the layout. Layout detection and the models are those of 0.64.0. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.64.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.64.1-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.64.1-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.64.1-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.64.1-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.64.1-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.64.1_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (what it grants is below); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.64.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.64.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- Windows: KeySwitch no longer forgets the word being typed when the system reports input its keyboard and mouse hooks did not get. 0.64.0 let go of the word, the text before it and the last correction each time, and on a laptop such input came often while typing, in every program: words in the middle of a sentence were left uncorrected, Pause found no word to convert and only switched the layout, and the word after it was then left as typed. The log still names, once, a program whose input reaches no hook, and only when the system's last input time moved on several times with no event reaching the hooks, as it does when keys typed into such a program pass them; a single input between keys the hooks got is no longer taken for one.

### Model

- The context model `context-v3-b380007b5e3e` and the prefix model `prefix-v2-bf3dc28f8567` are those of 0.64.0, byte for byte: the pair is sealed again for the changed engine. On the recorded real typing the new engine breaks no word. On sealed test v80, read once, it restores all 174 rows typed in the wrong layout with the early switch off in both profiles, against 173 for the baseline pair, and spoils no correct row (the baseline pair one).

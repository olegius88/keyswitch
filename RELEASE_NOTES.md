# KeySwitch 0.72.0

## Русский

Этот выпуск про целостность набранного текста. Исправление больше не стирает не те символы, когда поле показывает текст с опозданием; Enter и Tab не пропадают и не задерживаются без нужды, а клавиатура не остаётся придержанной. В Windows клавиши, нажатые во время исправления, доходят до окна такими, какими их нажали. Добавлены преобразование последнего слова одним нажатием Shift или Ctrl и самообновление на Mac, а после запуска программа быстрее готова к работе. Настройки, правила, история и личные слова сохраняются при обновлении. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.72.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.72.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.72.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.72.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.72.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.72.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.72.0_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.72.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.72.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Исправление, которое ждёт поле, показывающее текст с опозданием (редакторы на Electron, браузеры), отменяется, если за это время набрано что-то кроме букв следующего слова: скобка, второй пробел, Backspace. Раньше оно всё равно выполнялось и стирало не те символы: из `gthtl (j` получалось `gперед о` вместо `перед (о`.
- Windows, macOS и Wayland: Enter и Tab после слова больше не пропадают. Enter ждёт исправление слова и уходит следом, в том числе когда поле показывает текст с опозданием; если исправить слово нельзя (поле не прочиталось или так и не показало слово), Enter уходит с текстом как есть. Раньше в таком поле первый Enter ничего не делал, а второй отправлял неисправленное.
- Windows, macOS и Wayland: Enter и Tab придерживаются только тогда, когда после предыдущего из них была нажата хоть одна клавиша. Enter после Enter, Tab после Tab и любой из них после щелчка мышью доходят до окна сразу и повторяются при удержании. Раньше каждый Enter и Tab ждал отпускания клавиши, удержанный приходил один раз, а удержанный дольше двух секунд не приходил вовсе. Enter после слова по-прежнему ждёт исправления этого слова и приходит один раз.
- Windows, macOS и Wayland: в программах из списка исключений Enter и Tab больше не придерживаются. Первое нажатие сразу после перехода в такую программу ещё может прийти от KeySwitch, сразу и одним нажатием.
- Windows, macOS и Wayland: клавиатура больше не остаётся придержанной до перезапуска программы. Так бывало, когда окно правила открывалось в момент нажатия Enter, когда горячей клавишей был назначен одиночный Tab или Enter и после сбоя посреди замены.
- Windows, macOS и Wayland: слово, пробел и Enter, набранные быстрее, чем программа успевает их взять, исправляются и отправляются по порядку: `ytn`, пробел и Enter отправляют `нет`. Раньше слово уходило как набрано.
- Windows, macOS и Wayland: первое слово после Enter исправляется как любое другое и тогда, когда перед Enter была клавиша, за которой программа не следит (например, F5), или движение каретки.
- Linux (сеанс X11): Enter при открытой подсказке правила завершает слово, как любой другой Enter. Раньше следующее слово исправлялось вместе с предыдущим: после `Djn`, двух Pause, Enter и слова из шести букв следующий Pause стирал девять символов в строке из шести.
- Windows: Ctrl или Shift, нажатые во время исправления, больше не остаются нажатыми для окна, а клавиша, нажатая в самый момент начала исправления, не портит слово. Pause, Num Lock и правый Shift, попавшие на время исправления, доходят до окна сами собой: раньше Pause в такой момент переключал Num Lock, а правый Shift и Num Lock терялись (проверено на Windows 10). Косая черта цифрового блока, которую исправление набирает заново, остаётся косой чертой: в русской раскладке она могла стать точкой.
- Windows: ничто не держит клавиатуру дольше десяти секунд, а клавиши, которые система отказалась принять, больше не печатаются через минуты в другом окне.
- Windows: исправление набирает Shift своих заглавных так, как это делает клавиатура, и не трогает Shift, который вы держите. Отпускание Ctrl, которое Windows посылает окну при смене раскладки, больше не задерживается: в окне TeamViewer из-за этого заглавные буквы исправления открывали палитру команд редактора на управляемом компьютере.
- Последнее слово можно преобразовать одним нажатием Shift или Ctrl: настройка «Преобразовать последнее слово одной клавишей» на странице «Горячие клавиши», по умолчанию выключена. Нажатие и отпускание выбранной клавиши без других клавиш делает то же, что горячая клавиша преобразования; второе нажатие сразу после первого предлагает правило. Shift с буквой и Ctrl+C ничего не преобразуют.
- macOS: KeySwitch обновляется сам, как в Windows: на странице «Обновления» есть автоматическая проверка, автоматическая установка, «Проверить сейчас» и «Установить сейчас». Архив проверяется по размеру, SHA-256 и подписи того же разработчика; если новая версия не запустилась, возвращается прежняя. С 0.71.2 это обновление нужно один раз поставить вручную: в ней такого кода ещё нет.
- Windows: страница «О программе» показывает модели, с которыми работает программа; раньше там всегда стояло «Загрузка языковых моделей…».
- После запуска программа быстрее готова к работе: модели читаются одновременно, а не одна за другой. На настольном компьютере движок собирался около 1,3 секунды вместо 2,2 без словарей Hunspell и около 2,1 вместо 3,3 со словарями.

### Если нужно вернуться на 0.71.2

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.71.2-<архитектура>.exe` со [страницы выпуска 0.71.2](https://github.com/olegius88/keyswitch/releases/tag/v0.71.2). Настройки и правила сохранятся.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.2`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.71.2.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в выпусках с 0.70.0 по 0.71.2.

## English

This release is about keeping typed text whole. A correction no longer deletes the wrong characters when a field shows its text a moment late; Enter and Tab are neither lost nor kept back without need, and the keyboard is never left held. On Windows the keys pressed while a correction is typed reach the window as they were pressed. New are the conversion of the last word with a single tap of Shift or Ctrl and self-update on the Mac, and the program is ready sooner after it starts. Settings, rules, history and personal words are kept through the update. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.72.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.72.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.72.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.72.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.72.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.72.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.72.0_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon) on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.72.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.72.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A correction that waits for a field read a moment late (Electron editors, browsers) is given up as soon as anything but letters of the next word is typed: a bracket, a second space, a Backspace. It used to be made all the same and deleted the wrong characters: `gthtl (j` became `gперед о` instead of `перед (о`.
- Windows, macOS and Wayland: an Enter or Tab after a word is no longer lost. The Enter waits for the word's correction and follows it, also where the field shows its text late; if the word cannot be corrected (the field gave no answer or never showed the word), the Enter goes on with the text as it stands. In such a field the first Enter used to do nothing and the second one sent the text uncorrected.
- Windows, macOS and Wayland: Enter and Tab are kept back only when a key was pressed since the last one of them. An Enter after an Enter, a Tab after a Tab and either of them after a click reach the window as they are pressed and repeat while held. Every Enter and Tab used to wait for its key to come up, a held one came once, and one held for two seconds did not come at all. An Enter after a word still waits for that word's correction and comes once.
- Windows, macOS and Wayland: a program in the list of exclusions has its Enter and Tab as its own. The first press right after such a program comes to the front may still be typed by KeySwitch, at once and as a single press.
- Windows, macOS and Wayland: the keyboard is no longer left held until the program is restarted. It happened when the rule window opened at the very moment of an Enter, when a hotkey was set to Tab or Enter alone, and after a failure in the middle of a correction.
- Windows, macOS and Wayland: a word, its space and Enter typed faster than the program takes the keys are corrected and sent in that order: `ytn`, a space and Enter send `нет`. The word used to be sent as typed.
- Windows, macOS and Wayland: the first word after an Enter is corrected like any other also when a key the program does not follow (F5, say) or a caret move came before that Enter.
- Linux (X11 session): an Enter pressed while the rule prompt is shown ends the word, as any other Enter does. The next word used to be converted together with the one before: after `Djn`, Pause twice, Enter and a word of six letters the next Pause deleted nine characters on a line that held six.
- Windows: a Ctrl or a Shift pressed while a correction is typed no longer stays down for the window, and a key pressed in the instant a correction begins no longer spoils the word. Pause, Num Lock and the right Shift that fall into a correction reach the window as themselves: a Pause at that moment used to switch Num Lock, and a right Shift or a Num Lock was lost (checked on Windows 10). A slash of the numeric keypad that a correction types again stays a slash: in the Russian layout it could come out as a full stop.
- Windows: nothing keeps the keyboard held for longer than ten seconds, and keys the system refused to take are no longer typed minutes later in another window.
- Windows: a correction types the Shift of its capitals as a keyboard does and leaves alone a Shift you still hold. The key-up of Ctrl that Windows sends to a window whose layout is switched is no longer held back: in a TeamViewer window it made the capitals of a correction open the command palette of the editor on the controlled computer.
- The last word can be converted with a single tap of Shift or Ctrl: the setting «Преобразовать последнее слово одной клавишей» (convert the last word with a single key) on the Hotkeys page, off by default. A press and release of the chosen key with no other key does what the conversion hotkey does; a second tap right after the first offers a rule. Shift with a letter and Ctrl+C convert nothing.
- macOS: KeySwitch updates itself, as on Windows: the Updates page has the automatic check, the automatic install, «Проверить сейчас» (check now) and «Установить сейчас» (install now). The archive is checked by its size, SHA-256 and the signature of the same developer; if the new version does not start, the old one comes back. From 0.71.2 this update has to be installed by hand once: that version has no such code yet.
- Windows: the About page shows the models the program works with; it always read «Загрузка языковых моделей…» (loading the language models) before.
- The program is ready sooner after it starts: the models are read side by side instead of one after another. On a desktop computer the engine was built in about 1.3 seconds instead of 2.2 without the Hunspell dictionaries, and in about 2.1 instead of 3.3 with them.

### Going back to 0.71.2

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.71.2-<architecture>.exe` from the [0.71.2 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.71.2) over it. Settings and rules are kept.
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.71.2`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.71.2 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in every release from 0.70.0 to 0.71.2.

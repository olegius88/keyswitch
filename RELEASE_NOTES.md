# KeySwitch 0.64.0

## Русский

В этом выпуске KeySwitch работает и в сеансе Wayland: в GNOME, KDE Plasma 6 и sway. В Windows перехватчик клавиатуры отвечает быстро и на занятом компьютере, а если клавиши всё же прошли мимо него, KeySwitch оставляет слово как набрано, а не исправляет его вслепую. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.64.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.64.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.64.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.64.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.64.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.64.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.64.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (что это даёт — ниже); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.64.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.64.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Wayland: в сеансе Wayland программа не видит клавиш чужих окон, поэтому KeySwitch перехватывает клавиатуру на уровне ядра: отдельный процесс захватывает клавиатуры и передаёт каждую клавишу дальше через свою виртуальную клавиатуру, через неё же вводится исправление. Раскладку и активное окно сообщает среда: расширение GNOME Shell из пакета, служба раскладок KDE и скрипт KWin или IPC sway. Enter и Tab, как в Windows, ждут, пока слово исправится. Доступ к клавиатурам пакет не даёт: его один раз включает администратор командой `sudo keyswitch --enable-wayland-input`. После этого любая программа сеанса может читать все нажатия, включая пароли, и печатать сама — как любая программа в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу снимает доступ. На экране блокировки KeySwitch отпускает клавиатуры. Если основной процесс завис, клавиши проходят сами через 0,25 секунды и ни одна не теряется. Поле пароля в GNOME KeySwitch не трогает. Проверено в виртуальных машинах с GNOME 50, Plasma 6.6 и sway 1.11, из исходников и из пакета.
- Windows: перехватчик клавиатуры отвечает на каждую клавишу за несколько миллисекунд и на занятом компьютере. Раньше его короткие вызовы системы каждый раз ждали очереди к интерпретатору Python, и пока рядом решалось слово, ответ задерживался до секунды и больше, а с ним вся клавиатура системы. Перехватчик, который держит клавиши так долго, теряет часть из них: Windows отдаёт их окну мимо него. В виртуальной машине с полностью занятым процессором самый долгий ответ упал с 1,8 секунды до 30 миллисекунд.
- Windows: если процессор так занят, что клавиша доходит до перехватчика на 0,28 секунды позже нажатия, KeySwitch оставляет набираемое слово как есть. Windows 10 и 11 отдают окну мимо перехватчика каждую клавишу, которая ждала его около 0,3 секунды, поэтому слово на экране уже не то, что видел KeySwitch, и исправление стёрло бы не те буквы.
- Windows: журнал один раз называет программу, ввод в которую не доходит до KeySwitch, — программу на переднем плане, которая сама читает клавиатуру через Raw Input и ставит свой перехватчик клавиатуры. Windows тогда не вызывает ни одного перехватчика, и KeySwitch там не исправляет и не отвечает на горячие клавиши; текст остаётся как набран.
- Слово, которое решено чуть позже, чем окно получило клавиши после него, больше не отвергается как набранное в изменившееся поле: при чтении поля быстро набранное `e 'njuj ` сохраняло своё `e`.
- Короткое русское слово сразу после латинского текста исправляется вместе с остальным русским текстом, если переключить раскладку обратно на русскую забыли: `seek() , f там` становилось `seek() , а там` только без Hunspell, а `TSet - z использовал` и `(b наверное` с ним оставались как набраны. Hunspell знает `f` и `z` как английские слова, и слово оставалось на своём пробеле, а оставленное посреди строки слово не забиралось вместе с исправленными словами после него. Теперь на этот вопрос отвечает новая часть модели, а движок забирает такое слово и посреди строки, если модель его исправляет; остальные оставленные слова, как и раньше, забираются только в начале строки.
- Подсказка правила по двойному Pause встаёт под курсор, если над ним нет места, и не уходит за край монитора.
- Журнал отличает раннюю смену раскладки, уступившую пробелу сразу за словом, от отменённого исправления и называет вид клавиши, из-за которой клавиши после слова нельзя было набрать заново.
- Первая строка журнала называет сборку системы.

### Модель

- Контекстная модель `context-v3-b380007b5e3e` — пара 0.63.0 с новой частью для короткого слова после латиницы; модель префиксов `prefix-v2-bf3dc28f8567` — та же. На журналах реального набора она не портит ни одного слова и исправляет одно. На запечатанном тесте v79, прочитанном один раз, без ранней смены раскладки восстановлены 176 и 176 из 178 строк, набранных не в той раскладке, против 173 и 174 у базовой пары; правильный текст не испорчен ни разу (у базовой пары — две строки). На экзамене переписки исправлено на 12 сообщений больше, набранных с забытым переключением обратно на русский, потерь нет.

## English

In this release KeySwitch also works in a Wayland session: in GNOME, KDE Plasma 6 and sway. On Windows the keyboard hook answers quickly even on a busy computer, and when keys still pass it, KeySwitch leaves the word as typed instead of correcting it blindly. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.64.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.64.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.64.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.64.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.64.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.64.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.64.0_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (what it grants is below); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.64.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.64.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- Wayland: in a Wayland session a program sees no keys of other windows, so KeySwitch takes the keyboard at the kernel level: a separate process grabs the keyboards and passes every key on through a virtual keyboard of its own, which also types the corrections. The layout and the active window come from the desktop: a GNOME Shell extension installed with the package, the KDE keyboard service and a KWin script, or sway's IPC. Enter and Tab wait for the word to be corrected, as on Windows. The package does not grant access to the keyboards: an administrator enables it once with `sudo keyswitch --enable-wayland-input`. Any program of the session can then read every key, passwords included, and type keys of its own — as any program can on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once. KeySwitch lets the keyboards go on the lock screen. If the main process hangs, keys go through on their own after 0.25 seconds and none is lost. A password field in GNOME is left alone. Verified in virtual machines with GNOME 50, Plasma 6.6 and sway 1.11, from source and from the package.
- Windows: the keyboard hook answers each key in a few milliseconds even on a busy computer. Its short system calls waited for Python's interpreter lock again after each call, and with a word being decided at the same time the answer, and with it every key of the system, was held up to a second and more. A hook that keeps the keys that long loses some of them: Windows passes them to the window without it. In a virtual machine with every core busy the slowest answer fell from 1.8 seconds to 30 milliseconds.
- Windows: when the processor is so busy that a key reaches the hook 0.28 seconds or more after it was pressed, KeySwitch leaves the word being typed as it is. Windows 10 and 11 pass every key that waits about 0.3 seconds for the hook to the window without it, so the word on the screen is no longer the one KeySwitch saw, and a correction would delete the wrong letters.
- Windows: the log names, once, a program whose input does not reach KeySwitch — a program in front that reads the keyboard as raw input itself and has a keyboard hook of its own. Windows then calls no hook at all, and KeySwitch neither corrects nor answers its hotkeys there; the text is left as typed.
- A word decided a moment after the window took the keys that follow it is no longer refused as typed into a changed field: with field reading on, `e 'njuj ` typed fast kept its `e`.
- A short Russian word right after Latin text is corrected with the rest of the Russian text when the change back to Russian was forgotten: `seek() , f там` became `seek() , а там` only without Hunspell, and with it `TSet - z использовал` and `(b наверное` were left as typed. Hunspell knows `f` and `z` as English words, so the word was kept at its space, and a word kept in the middle of a line was not taken along when the words after it were corrected. A new part of the model now answers that question, and the engine takes such a word along in the middle of a line too when the model corrects it; every other kept word is still taken along only at the start of a line.
- The rule prompt of a double Pause goes under the caret when there is no room above it and stays on the monitor.
- The log tells an early switch that gave way to the space right after the word from a correction given up, and names the kind of key that kept the keys typed after a word from being typed again.
- The first line of the log names the system's build.

### Model

- The context model `context-v3-b380007b5e3e` is the pair of 0.63.0 with a new part for a short word after Latin text; the prefix model `prefix-v2-bf3dc28f8567` is the same. On the recorded real typing it breaks no word and fixes one. On sealed test v79, read once, it restores 176 and 176 of the 178 rows typed in the wrong layout with the early switch off, against 173 and 174 for the baseline pair, and spoils no correct row (the baseline pair two). On the developer-chat test it restores 12 more messages typed with the change back to Russian forgotten and loses none.

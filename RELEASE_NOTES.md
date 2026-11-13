# KeySwitch 0.73.0

## Русский

Этот выпуск про слова, которые раньше оставались неисправленными, и про скорость. Программа успевает за быстрым набором: слово, пробел, следующие буквы и Enter подряд теперь исправляются и уходят в том порядке, в каком набраны. Клавиша, набранная перед тем, как вперёд само вышло другое окно, принадлежит своему окну, и исправление больше не стирает лишний символ. Первое слово в окне, куда вы перешли, исправляется, как любое другое. Программа готова к работе вдвое быстрее, а на Mac тратит вдвое меньше времени на каждую нажатую клавишу.

### Файлы выпуска

- `KeySwitch-Setup-0.73.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.73.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.73.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.73.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.73.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.73.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.73.0_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.73.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.73.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Слово, пробел, следующие буквы и Enter, набранные быстрее, чем программа успевает за клавишами, исправляются и уходят в том порядке, в каком набраны: `ytn ghbdtn` и Enter отправляют `нет привет`. Буквы, набранные после слова, программа стирает вместе с ним и набирает заново, и они возвращались ей позади Enter, который уже ждал своей очереди, — хотя в окне стоят перед ним. Такое исправление отменялось, и слова уходили как набраны. Теперь Enter ждёт за последней из этих букв; если буквы не вернулись за четверть секунды, он идёт дальше.
- Буква, набранная перед тем, как вперёд само вышло другое окно (открылся диалог, запустилась программа: без щелчка и без клавиш), принадлежит тому окну, в которое её набрали. Программа берёт клавишу не сразу, а пока она набирает исправление или читает поле — и того позже, и спрашивала об окне только в этот момент: буква считалась первой буквой слова в новом окне, а исправление этого слова стирало на символ больше, чем в окне было, — символ текста, стоявшего перед словом. Теперь перехватчик сообщает окно каждой клавиши.
- Windows и macOS: первое слово в окне, куда вы перешли, исправляется, как любое другое. Эти системы держат раскладку отдельно для каждого окна и сообщают её с первой нажатой там клавишей, а программа принимала это за раскладку, выбранную вами вручную, — а выбранная вручную раскладка защищает слово от любого исправления.
- Программа готова исправлять вдвое быстрее: 0,9 секунды вместо 1,6 на том компьютере, где это мерили. До того как прочитаны модели, она ничего не исправляет, и самая большая из них ждала двух словарей, потому что спрашивает их, есть ли слово в словаре своего языка. При чтении она их не спрашивает, только потом, — поэтому теперь читается рядом с ними, а словари читаются рядом друг с другом.
- macOS: программа спрашивает систему о том, какое окно впереди, один раз на клавишу вместо двух. Mac отвечает на это списком всех окон на экране, и один такой список занимал 1,2 миллисекунды из двух, которые уходили на каждую нажатую клавишу. Отставание от клавиш и есть причина, по которой исправление не делается или делается с опозданием.
- Файл модели намерений втрое меньше: 4,7 МБ вместо 12,9 МБ. Две трети его занимала справочная таблица, которую не читает ни одно решение программы. Ответы те же до последнего бита; установщик для Windows меньше примерно на 4,8 МБ, архив — на 6,5 МБ, памяти программа занимает на 7,9 МБ меньше.

### Если нужно вернуться на 0.72.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.72.0-<архитектура>.exe` со [страницы выпуска 0.72.0](https://github.com/olegius88/keyswitch/releases/tag/v0.72.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.72.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.72.0.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567` — те же файлы, что в выпусках с 0.70.0 по 0.72.0. Модель намерений `intent-v1-8bccdf50b028` решает так же, как в 0.72.0: в пакет идёт та же модель без таблицы отпечатков.

## English

This release is about words that used to be left uncorrected, and about speed. KeySwitch keeps up with fast typing: a word, its space, more letters and Enter in a row are corrected and sent in the order they were typed. A letter typed just before another window comes to the front by itself belongs to its own window, and a correction no longer erases one character too many. The word typed first in a window you move to is corrected like any other. The program is ready to work in half the time, and on a Mac it spends half as long on every key pressed.

### Release files

- `KeySwitch-Setup-0.73.0-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.73.0-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.73.0-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.73.0-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.73.0-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.73.0-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.73.0_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.73.0-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.73.0-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- A word, its space, more letters and Enter typed faster than KeySwitch takes the keys are corrected and sent in the order they were typed: `ytn ghbdtn` and Enter send `нет привет`. The letters typed after the word are erased with it and typed again, and they came back to KeySwitch behind the Enter that was already waiting its turn — though they stand before it in the window. Such a correction was given up and the words went out as typed. The Enter now waits behind the last of those letters; if they do not come back within a quarter of a second, it goes on.
- A letter typed just before another window comes to the front by itself (a dialog that opens, a program that finishes starting: no click and no key) belongs to the window it was typed into. KeySwitch takes a key some time after it is pressed, longer while it types a correction or reads the field, and asked which window was in front only then: the letter became the first letter of the word typed in the new window, and the correction of that word erased one character more than the window had of it — a character of the text standing before the word. The hook now tells the window of every key.
- Windows and macOS: the word typed first in a window you move to is corrected like any other. These systems keep a layout per window and report it with the first key pressed there, and KeySwitch took that for a layout you had picked by hand — and a layout picked by hand protects the word from any correction.
- KeySwitch is ready to correct in half the time: 0.9 seconds instead of 1.6 on the computer this was measured on. It corrects nothing until its models are read, and the largest of them waited for the two dictionaries, because it asks them whether a word is in the dictionary of its script. It does not ask while it is read, only afterwards, so it is read beside them now, and the two dictionaries are read beside each other.
- macOS: KeySwitch asks the system which window is in front once for every key instead of twice. A Mac answers that with the list of every window on screen, and one such list took 1.2 milliseconds of the two that every key pressed cost. Falling behind the keys is what leaves a word uncorrected or corrects it late.
- The file of the layout-intent model is three times smaller: 4.7 MB instead of 12.9 MB. Two thirds of it was a table of fingerprints that no decision of the program reads. Every answer is the same to the bit; the Windows installer is about 4.8 MB smaller, the archive 6.5 MB, and the program uses 7.9 MB less memory.

### Going back to 0.72.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.72.0-<architecture>.exe` from the [0.72.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.72.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.72.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.72.0 archive.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: the same files as in the releases from 0.70.0 to 0.72.0. The layout-intent model `intent-v1-8bccdf50b028` decides as it did in 0.72.0: the same model goes into the package, without its table of fingerprints.

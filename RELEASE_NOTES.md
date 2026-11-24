# KeySwitch 0.75.0

## Русский

В этом выпуске у новых установок клавиши Shift работают как в самых известных переключателях Windows, программа запоминает слово, которое вы стёрли и набрали заново, а слово после стрелок и End больше не остаётся в чужой раскладке. Ещё выпуск исправляет больше слов с зажатым Shift, числа с точками, термины через дефис и делает набор через TeamViewer и другие программы удалённого доступа надёжнее; программа держит меньше памяти и быстрее запускается.

### Файлы выпуска

- `KeySwitch-Setup-0.75.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.75.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.75.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.75.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.75.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.75.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.75.0_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.75.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.75.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Новая установка работает клавишами Shift так же, как самые известные переключатели Windows: одиночное нажатие любого Shift переключает раскладку, двойное переводит последнее слово (или выделенный текст, если слова нет), как Pause и Shift+Pause, а оба Shift вместе выключают и включают автопереключение, как пункт паузы в меню трея, с короткой надписью у курсора. Обновлённая установка сохраняет свои настройки как были: новые клавиши есть на странице горячих клавиш и там выключены. Первая подсказка новой установки называет эти клавиши и страницу, где их меняют.
- Настройка «Переключать раскладку одной клавишей» предлагает «Любой Shift» вместе с прежними четырьмя клавишами, а рядом с ней две новые: «Двойное нажатие переводит слово» и «Оба Shift выключают автопереключение». Если система сама использует оба Shift (параметры XKB `grp:shifts_toggle`, `shift:both_capslock` и подобные), сочетание остаётся системе, а окно настроек объясняет, почему настройка недоступна. На macOS, где нет Pause, двойное нажатие — перевод одной рукой; Control+Option+K остаётся.
- Слово, которое KeySwitch перевёл сам, а вы стёрли Backspace и набрали снова так же, как в первый раз, запоминается и больше не переводится. Набранное заново слово становится правилом «Не переводить» для своей раскладки в общем списке правил, а у курсора появляется надпись «Слово «тыгыдык» запомнено: больше не переводится.» с кнопкой «Отменить», которая убирает это правило. Набор переведённого вида принимает перевод и ничего не запоминает. Настройка «Запоминать слово, набранное заново после исправления» рядом с правилами это выключает.
- Первое слово после стрелки, Home или End больше не остаётся в чужой раскладке. В поле, которое KeySwitch не может прочитать (окно удалённого рабочего стола, терминал), такое слово раньше не решалось вовсе и оставалось как набрано, даже когда следующее слово переводилось: `b ghjdthm` после End давало `b проверь`. Теперь оно ждёт следующего слова и переводится вместе с ним: `b ghjdthm` становится `и проверь`, `d vs` — `в мы`, `nfr/ ghbdtn` — `так. привет`. Без следующего слова, перед словом, которое остаётся (`b nginx`), и среди английского текста (`option b is`) оно остаётся как набрано, а Pause переводит его вручную.
- Windows: исправление в окне программы удалённого доступа или виртуальной машины (TeamViewer, AnyDesk, RustDesk, Parsec, VNC, «Подключение к удалённому рабочему столу», консоли Hyper-V, VirtualBox и VMware) вводится по одной клавише с паузой 15 мс: слово из двенадцати букв с пробелом — около 0,4 секунды. Введённое разом, исправление могло дойти до другого компьютера искажённым: буквы пропадали, а какая-то из следующих повторялась. Остальные окна получают исправление разом, как раньше. Паузу задаёт `applications.remote_key_interval_ms` в файле настроек, от 0 (разом) до 50.
- Зажатый на одну букву дольше Shift исправляется в большем числе слов: в частых словах из трёх букв (`КАк` становится `Как`, `ANd` — `And`), перед знаком (`КАк,` — `Как,`, `HEllo.` в английской раскладке — `Hello.`), в частях через дефис (`КТо-то` — `Кто-то`) и в слове, набранном в другой раскладке, одной правкой с переводом (`VFibyf` — `Машина`, а не `МАшина`). Единицы, сокращения с окончанием и код остаются как набраны (`МВт`, `ОСи`, `IDs`, `GHz`, `VMware`). Имена, которые русский текст пишет двумя заглавными нарочно, остаются: `ВКонтакте` больше не становится `Вконтакте`, а `DRjynfrnt` в английской раскладке становится `ВКонтакте`.
- Версия, адрес или номер пункта, набранные в русском тексте клавишей точки английской раскладки, получают все точки: `версии 1ю5ю3` становится `версии 1.5.3`, `адрес 192ю168ю1ю1` — `адрес 192.168.1.1`, `пункт 1ю2ю` — `пункт 1.2.`. Раньше токен с несколькими знаками оставался как набран.
- Латинское сокращение или название через дефис с русским словом исправляется по частям: набранные целиком в английской раскладке `SQL-pfghjc` становится `SQL-запрос`, `GitHub-htgjpbnjhbq` — `GitHub-репозиторий`, `Type-C-rf,tkm` — `Type-C-кабель`; набранные в русской раскладке `РЕЕЗ-запрос` — `HTTP-запрос`, `Цш-Аш-роутер` — `Wi-Fi-роутер`, `у-ьфшд-рассылка` — `e-mail-рассылка`. Идентификаторы и адреса (`user_name`, `foo=bar`) и токены заглавными остаются целыми, как раньше.
- Пауза после первых букв частого слова больше не переводит их: `ес` в начале `если` становилось `tc`, `оши` в начале `ошибка` — `jib`, и остаток слова набирался в другой раскладке.
- Программа держит меньше памяти, а решения те же. На Windows через минуту после запуска — около 36 МиБ вместо 46 МиБ. На Linux весь процесс занимает около 110 МиБ вместо 230 МиБ: GTK больше не открывает OpenGL вместе с экраном. Движок собирается вдвое быстрее (542 мс вместо 978 мс на компьютере с Linux) и держит 41 МиБ моделей вместо 49 МиБ.
- Поле, в котором идёт набор, читается реже, а решения те же: раннее переключение читало поле почти на каждой букве, теперь — раз на слово. На Windows первая буква слова больше не читает поле только ради того, чтобы узнать, не поле ли это пароля: программа спрашивает об этом одним вызовом. На том же тексте поле читается 0,31 раза на клавишу вместо 0,77 на Windows и 0,48 на других системах.
- Windows: двойное нажатие Shift переводит слово, набранное в любой раскладке, с любым темпом нажатий. Windows посылает окну отпускание Ctrl каждый раз, когда окно получает английскую раскладку, без нажатия перед ним; его принимали за клавишу после слова. Отпускание клавиши, которую KeySwitch не видел нажатой, теперь уходит окну и не считается набором.

### Если нужно вернуться на 0.74.4

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.74.4-<архитектура>.exe` со [страницы выпуска 0.74.4](https://github.com/olegius88/keyswitch/releases/tag/v0.74.4).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.4`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.74.4.

### Модель

- Модели те же, что в 0.74.0: контекстная `context-v3-a87b3715a517`, префиксов `prefix-v2-bf3dc28f8567` и намерений `intent-v1-8bccdf50b028`.

## English

In this release a new installation works the Shift keys as the best-known Windows switchers do, KeySwitch remembers a word you erased and typed again, and a word typed after an arrow key or End no longer stays in the other layout. It also fixes more words with Shift held too long, numbers with dots and hyphenated terms, makes typing through TeamViewer and other remote-control programs more reliable, keeps less in memory and starts faster.

### Release files

- `KeySwitch-Setup-0.75.0-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.75.0-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.75.0-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.75.0-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.75.0-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.75.0-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.75.0_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.75.0-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.75.0-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- A new installation works the Shift keys as the best-known Windows switchers do: a lone tap of either Shift switches the layout, a double tap converts the last word (or the selected text, when no word is there) as Pause and Shift+Pause do, and both Shifts together turn automatic switching off or on, as the pause item of the tray menu does, with a short notice at the caret. An installation updated from an earlier version keeps its settings exactly: the new keys are on the hotkeys page and are off there. The first hint of a new installation names the keys and the page where they are changed.
- The setting «Переключать раскладку одной клавишей» (switch the layout with one key) offers «Любой Shift» (either Shift) besides the four keys, and two settings come next to it: «Двойное нажатие переводит слово» (a double tap converts the word) and «Оба Shift выключают автопереключение» (both Shifts turn automatic switching off). Where the system already uses both Shifts (the XKB options `grp:shifts_toggle`, `shift:both_capslock` and the like), the chord is left to it and the settings window says why the setting is unavailable. On macOS, which has no Pause, the double tap is a conversion with one hand; Control+Option+K stays.
- A word KeySwitch converted by itself, erased with Backspace and typed again as it was first typed, is remembered and is no longer converted. The word typed again becomes a rule «Не переводить» (do not convert) for its layout, listed with the other rules, and a notice at the caret says the word is remembered, with «Отменить» (undo), which removes that rule. Typing the converted form again accepts the conversion and remembers nothing. The setting «Запоминать слово, набранное заново после исправления» (remember a word typed again after a correction), beside the rules, turns it off.
- The first word typed after an arrow key, Home or End no longer stays in the other layout. In a field KeySwitch cannot read (a remote desktop window, a terminal) such a word was never decided and stayed as typed even when the very next word was converted: `b ghjdthm` after End gave `b проверь`. It now waits for the word after it and is converted with it: `b ghjdthm` is `и проверь`, `d vs` is `в мы`, `nfr/ ghbdtn` is `так. привет`. With no word after it, before a word that stays (`b nginx`) and among English typed right (`option b is`) it stays as typed, and Pause converts it by hand.
- Windows: a correction typed into the window of a remote-control or virtual-machine program (TeamViewer, AnyDesk, RustDesk, Parsec, a VNC viewer, Remote Desktop, the console of Hyper-V, VirtualBox or VMware) is typed key by key with a pause of 15 ms: a twelve-letter word with its space takes about 0.4 s. Typed in one burst, a correction could reach the other computer garbled, with letters lost and a later one repeated. Every other window gets the correction in one burst, as before. The pause is `applications.remote_key_interval_ms` in the settings file, from 0 (one burst) to 50.
- Shift held one letter too long is fixed in more words: in frequent three-letter words (`КАк` becomes `Как`, `ANd` becomes `And`), before a sign (`КАк,` becomes `Как,`, `HEllo.` in the English layout becomes `Hello.`), in parts joined by a hyphen (`КТо-то` becomes `Кто-то`) and in a word typed in the other layout, in the same correction as the conversion (`VFibyf` becomes `Машина`, not `МАшина`). Units, abbreviations with an ending and code stay as typed (`МВт`, `ОСи`, `IDs`, `GHz`, `VMware`). Names Russian text writes with two capitals on purpose stay: `ВКонтакте` no longer becomes `Вконтакте`, and `DRjynfrnt` in the English layout becomes `ВКонтакте`.
- A version, an address or a numbered point typed in Russian text with the dot key of the English layout gets every dot back: `версии 1ю5ю3` becomes `версии 1.5.3`, `адрес 192ю168ю1ю1` becomes `адрес 192.168.1.1`, `пункт 1ю2ю` becomes `пункт 1.2.`. A token with more than one sign was left as typed.
- A Latin abbreviation or name joined by a hyphen to a Russian word is corrected part by part: typed all in the English layout, `SQL-pfghjc` becomes `SQL-запрос`, `GitHub-htgjpbnjhbq` becomes `GitHub-репозиторий`, `Type-C-rf,tkm` becomes `Type-C-кабель`; typed in the Russian layout, `РЕЕЗ-запрос` becomes `HTTP-запрос`, `Цш-Аш-роутер` becomes `Wi-Fi-роутер`, `у-ьфшд-рассылка` becomes `e-mail-рассылка`. Identifiers and addresses (`user_name`, `foo=bar`) and tokens in capitals stay whole, as before.
- A pause after the first letters of a frequent word no longer converts them: `ес` at the start of `если` became `tc`, `оши` at the start of `ошибка` became `jib`, and the rest of the word was typed in the other layout.
- KeySwitch keeps less in memory, with every decision as before. On Windows it holds about 36 MiB a minute after its start instead of 46 MiB. On Linux the whole process takes about 110 MiB instead of 230 MiB: GTK no longer opens OpenGL with the display. The engine is built in half the time (542 ms instead of 978 ms on a Linux desktop) and holds 41 MiB of models instead of 49 MiB.
- The field being typed in is read less often, with every decision as before: the early switch read the field at almost every letter, and now reads it once a word. On Windows the first letter of a word no longer reads the field only to learn whether it is a password field: the program asks just that, in one call. Over the same text the field is read 0.31 times a key instead of 0.77 on Windows and 0.48 elsewhere.
- Windows: a double tap of Shift converts the word typed in either layout, with the taps made at any pace. Windows sends a window a release of Ctrl each time the window takes the English layout, with no press before it, and it was taken for a key typed after the word. A key released that KeySwitch never saw pressed now goes on to the window without counting as typed.

### Going back to 0.74.4

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.74.4-<architecture>.exe` from the [0.74.4 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.74.4).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.74.4`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.74.4 archive.

### Model

- The models are the ones of 0.74.0: the context model `context-v3-a87b3715a517`, the prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028`.

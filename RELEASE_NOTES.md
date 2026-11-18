# KeySwitch 0.74.0

## Русский

Этот выпуск про опечатки и короткие слова. Исправление опечатки, которое вы стёрли, больше не повторяется на том же месте, а клавиша отмены возвращает последнее исправление и тогда, когда вы уже набираете следующие слова. Опечатка исправляется только в слово, которое часто встречается в письменном тексте. Короткие русские слова после русского текста (`ру`, `ща`) больше не превращаются в английские, а сокращения заглавными буквами, набранные в другой раскладке, исправляются: `CCCH` становится «СССР».

### Файлы выпуска

- `KeySwitch-Setup-0.74.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.74.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.74.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.74.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.74.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.74.0_amd64.deb` — Ubuntu/Xubuntu и Linux Mint 22 (Cinnamon), сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.74.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.74.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Исправление опечатки, стёртое клавишей Backspace, больше не делается снова на слове, набранном на его месте. Слово, набранное как задумано и всё равно исправленное (`мржно` — имя или словечко — становилось `можно`), стирали и набирали снова, и оно исправлялось снова, сколько бы его ни набирали; выручала только клавиша отмены. Теперь, как только Backspace стирает букву такого исправления, слово, набранное на этом месте, остаётся как набрано, а если это то же слово, что и в первый раз, программа запоминает его, как после отмены, и дальше тоже не исправляет. Щелчок, переход курсора, другое окно или Enter это заканчивают. Backspace, нажатые в первые 0,6 секунды после исправления, раньше, чем его можно прочитать, считаются правкой самой опечатки: дойдя до исправленного слова, они оставляют его исправляться, как любое другое.
- Клавиша отмены (`Ctrl+Alt+Z`) возвращает последнее автоматическое исправление и тогда, когда после него уже набраны следующие слова. Неверное исправление часто замечают только словом позже, а клавиша тогда ничего не делала. Теперь она стирает текст до исправленного слова, набирает его так, как оно было набрано, и снова набирает текст после него в той раскладке, в которой он был набран. Так бывает, пока программа знает этот текст по клавишам: до 24 клавиш в течение 10 секунд после исправления, только буквы, цифры, пробелы и знаки `.,!?:;-`, без Backspace, стрелок, щелчков, сочетаний клавиш, Enter, Tab и другого окна. Иначе клавиша делает то же, что и раньше.
- Опечатка исправляется только в слово письменного текста. Слово, в которое исправлялась опечатка, выбиралось по частотам субтитров фильмов, а в них есть обрубки слов (`кажетс`, `сегодн`) и имена из фильмов, и опечатка исправлялась в то, что словом не является. Кроме того, `сьрока` становилось `сорока`, хотя в письменном тексте `строка` встречается гораздо чаще. Теперь выбор сверяется со счётом слов письменного текста, который программа уже содержит. На проверочных текстах программа делает 97 % прежних верных исправлений, а ошибочных — в два с половиной раза меньше (127 вместо 337 на 55 633 опечатках).
- Если слово осталось как набрано только потому, что раскладку вы выбрали вручную, а модель бы его исправила, у курсора появляется подсказка с объяснением и названием клавиши преобразования. Раньше, если переключиться на английскую раскладку и набрать `gfgefcs`, чтобы проверить программу, слово оставалось как набрано без всяких объяснений, и казалось, что программа не знает слова «папуасы». Подсказка показывается каждый раз, но не чаще других подсказок (одна за десять минут, три в день), пока её не выключат её кнопкой «Больше не показывать»; остальные подсказки при этом остаются.
- Пауза после первых двух-трёх букв слова больше не переводит их. `ин` в начале `инструкция` становилось `by`, `уд` — `el`, `рез` — `htp`, и раскладка менялась под оставшуюся часть слова. Теперь такие буквы, если с них начинается не меньше сотни слов своего языка и до них стоит слово той же раскладки, ждут конца слова.
- Остановка после слова, которое ждёт следующего, больше не оставляет его как набрано. Русская однобуквенная частица, набранная в английской раскладке после английского термина, решается вместе со следующим словом: `redis b nats` без остановки становилось `redis и nats`, а секунда раздумья после `b` оставляла `b`. Теперь так же, как без остановки.
- Слово, которому вернули раскладку клавишей преобразования или отменой, остаётся как есть, если вернуться в него Backspace. `ру` после русского слова становилось `he`, Pause возвращал `ру`, а точка, набранная вместо пробела после него, снова делала из него `he/`.
- Короткие русские слова после русского текста остаются русскими, если их латинское прочтение просто совпадает с частым английским словом: «войти через маил ру» и «сайт на ру» больше не заканчиваются на `he`, а «ок ща приду» не начинается с `of`.
- Сокращения из двух–пяти заглавных букв, набранные в другой раскладке, исправляются, когда другое их прочтение пишется заглавными: `CCCH`, набранное в английской раскладке, становится «СССР». Программа смотрит, как часто каждое прочтение пишется заглавными в текстах, на коды валют и на заглавные формы русского словаря. На наборе таких слов, в той половине, на которой модель не училась, русские сокращения выходят верными в 315 случаях из 336 вместо 261, коды валют — в 148 из 176 вместо 116, латинские сокращения — в 520 из 528 вместо 502.
- Linux: значок в трее появляется, если панель запустилась позже KeySwitch. Запущенная со входом в систему, программа может оказаться раньше панели рабочего стола; тогда её значку негде было зарегистрироваться, и он не появлялся до перезапуска KeySwitch. Теперь значок регистрируется, как только панель появилась, и заново после каждого перезапуска панели.

### Если нужно вернуться на 0.73.0

- Windows: выключите в настройках «Автоматически устанавливать обновления» и поставьте поверх `KeySwitch-Setup-0.73.0-<архитектура>.exe` со [страницы выпуска 0.73.0](https://github.com/olegius88/keyswitch/releases/tag/v0.73.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.73.0`, затем `sudo apt-mark hold keyswitch`, чтобы система не обновила его снова.
- Mac: замените программу архивом 0.73.0.

### Модель

- Контекстная модель `context-v3-a87b3715a517` (SHA-256 артефакта `65395624569812f24d02519a5f060a1e683d5089d6d2072e27a135f14baf6c79`) — модель 0.73.0 с двумя головами, обученными на корпусе v88: для коротких слов после русского текста и для слов заглавными. Остальные решения — те же, что у 0.73.0. Закрытый тест v88 прочитан один раз, до установки, и пройден: из 695 первых слов, набранных в неверной раскладке, восстановлены 693 и 694 (с двумя наборами словарей), из 695 набранных верно испорчены 2 и 1 — так же, как у 0.73.0; на тесте переписки разработчиков (1 125 сообщений) верных сообщений в каждом из четырёх способов набора столько же или больше, чем у 0.73.0. Модель префиксов `prefix-v2-bf3dc28f8567` и модель намерений `intent-v1-8bccdf50b028` — те же, что в 0.73.0.

## English

This release is about typos and short words. A typo correction you erased is no longer made again where it stood, and the undo hotkey takes back the last correction also once you are typing the next words. A typo is corrected only into a word that written text uses often. Short Russian words after Russian text (`ру`, `ща`) no longer turn into English ones, and abbreviations in capitals typed in the other layout are converted: `CCCH` becomes `СССР`.

### Release files

- `KeySwitch-Setup-0.74.0-x64.exe` — the installer for Windows 10/11 x64. It carries no publisher certificate: SmartScreen will warn about it.
- `KeySwitch-0.74.0-windows-x64.zip` — the portable archive for Windows x64.
- `KeySwitch-Setup-0.74.0-arm64.exe` — the installer for Windows 11 on ARM. It carries no publisher certificate either.
- `KeySwitch-0.74.0-windows-arm64.zip` — the portable archive for Windows on ARM.
- `KeySwitch-Setup-0.74.0-x86.exe` — the installer for 32-bit Windows 10. It carries no publisher certificate either.
- `KeySwitch-0.74.0-windows-x86.zip` — the portable archive for 32-bit Windows.
- `keyswitch_0.74.0_amd64.deb` — Ubuntu/Xubuntu and Linux Mint 22 (Cinnamon), an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the usual system update.
- `KeySwitch-0.74.0-macos-arm64.zip` — a Mac on Apple Silicon (M1 and newer), macOS 13 and newer.
- `KeySwitch-0.74.0-macos-x86_64.zip` — a Mac on an Intel processor, macOS 13 and newer.
- `SHA256SUMS` — the checksums of every file; check them before installing.

Exactly one Mac archive is the right one: the Apple Silicon build does not run on Intel and the other way round.

### Changed

- A typo correction erased with Backspace is not made again on the word typed in its place. A word typed as it was meant and corrected anyway (`мржно`, a name or a word of slang, became `можно`) was erased and typed again, and corrected again, as often as it was typed; only the undo hotkey got out. Once Backspace erases a letter of such a correction, the word next typed there is left as typed, and if it is the word as first typed, KeySwitch remembers it, as after undo, and does not correct it later either. A click, a caret move, another window or Enter end this. Backspaces begun within 0.6 seconds of the correction, sooner than it can be read, fix the slip as typed: reaching the corrected word, they leave it to be corrected as any word.
- The undo hotkey (`Ctrl+Alt+Z`) takes back the last automatic correction also once the next words are being typed. A wrong correction is often seen only a word later, and the hotkey then did nothing. It now deletes back to the corrected word, types it as it was typed, and types the text after it again, in the layout it was typed in. It does so while KeySwitch knows that text key by key: up to 24 keys within 10 seconds of the correction, letters, digits, spaces and `.,!?:;-` alone, with no Backspace, arrow key, click, shortcut, Enter, Tab or other window in between. Otherwise the hotkey does what it did.
- A typo is corrected only into a word of written text. The word a slip was undone into came from counts of film subtitles, which hold stumps of words (`кажетс`, `сегодн`) and names films say, so a word was corrected into something that is no word; and `сьрока` became `сорока`, though written text says `строка` far more often. The choice is now checked against the counts of written words the program already ships. On development texts KeySwitch makes 97 % of the right corrections it made and two and a half times fewer wrong ones (127 instead of 337 of 55,633 slips).
- A word left as typed only because its layout was picked by hand is pointed out when the model would have converted it: a hint at the caret says why and names the conversion hotkey. Switching to English and typing `gfgefcs` to see whether the program works left the word as typed with no word of why, and it looked as if KeySwitch did not know `папуасы`. The hint shows each time, no more often than the other hints (one in ten minutes, three a day), until its own «Больше не показывать» (do not show again), which leaves the other hints on.
- A pause after the first two or three letters of a word no longer converts them: `ин` at the start of `инструкция` became `by`, `уд` became `el`, `рез` became `htp`, and the layout changed under the rest of the word. Such letters, when at least a hundred words of their language begin with them and a word of the same layout stands before them, now wait for the end of the word.
- A stop after a word that waits for the next one no longer leaves it as typed. A Russian one-letter word typed in the English layout after an English term is decided together with the next word: `redis b nats` typed without a stop became `redis и nats`, and a second's thought after `b` left `b`. Now it goes as without a stop.
- A word given back its layout with the conversion hotkey or by undo stays as it is when you go back into it with Backspace: `ру` after a Russian word became `he`, Pause brought `ру` back, and a full stop typed in place of the space after it made it `he/` again.
- Short Russian words after Russian text stay Russian when their Latin reading only happens to be a common English word: `войти через маил ру` and `сайт на ру` no longer end in `he`, and `ок ща приду` no longer starts with `of`.
- Abbreviations of two to five capitals typed in the other layout are converted when their other reading is written in capitals: `CCCH` typed in the English layout becomes `СССР`. KeySwitch reads how often each reading is spelled in capitals in written text, the currency codes and the capital forms of the Russian dictionary. On its set of such tokens, in the half the model did not learn from, Russian abbreviations come out right in 315 of 336 cases instead of 261, currency codes in 148 of 176 instead of 116, Latin abbreviations in 520 of 528 instead of 502.
- Linux: the tray icon appears when the panel starts after KeySwitch. Started with the session, KeySwitch can be up before the desktop's panel; its icon then found no panel to register with and stayed away until KeySwitch was started again. It now registers as soon as a panel is there, and again whenever the panel starts again.

### Going back to 0.73.0

- Windows: turn off «Автоматически устанавливать обновления» (install updates automatically) in the settings and install `KeySwitch-Setup-0.73.0-<architecture>.exe` from the [0.73.0 release page](https://github.com/olegius88/keyswitch/releases/tag/v0.73.0).
- Ubuntu: `sudo apt install --allow-downgrades keyswitch=0.73.0`, then `sudo apt-mark hold keyswitch` so that the system does not update it again.
- Mac: replace the program with the 0.73.0 archive.

### Model

- The context model `context-v3-a87b3715a517` (artifact SHA-256 `65395624569812f24d02519a5f060a1e683d5089d6d2072e27a135f14baf6c79`) is the 0.73.0 model with two heads fitted onto it on corpus v88: one for short words after Russian text and one for words in capitals. Every other decision is the 0.73.0 model's. Test v88 was read once, before the install, and passed: of 695 first words typed in the wrong layout it restores 693 and 694 (with the two sets of dictionaries), of 695 typed right it spoils 2 and 1, as 0.73.0 does; on the developer-chat test (1,125 messages) it types as many messages right as 0.73.0 or more in each of the four ways of typing. The prefix model `prefix-v2-bf3dc28f8567` and the layout-intent model `intent-v1-8bccdf50b028` are the ones of 0.73.0.

# KeySwitch 0.65.0

## Русский

KeySwitch теперь исправляет опечатки и две заглавные в начале слова, умеет менять раскладку, регистр и транслитерацию выделенного текста, следует тёмной теме системы и распознаёт ещё пять видов ошибок раскладки: однобуквенные русские слова, флаги командной строки, цифры с точкой, кавычку в начале сообщения и короткие ответы вроде `jr` → `ок`. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.65.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.65.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.65.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.65.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.65.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.65.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.65.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11 или Wayland (GNOME, KDE Plasma 6, sway). Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы. В сеансе Wayland один раз включите доступ к клавиатурам: `sudo keyswitch --enable-wayland-input` (после этого любая программа этого пользователя может читать все клавиши и набирать свои, как в X11 и Windows; `sudo keyswitch --disable-wayland-input` сразу отбирает доступ); в GNOME после первой установки выйдите из учётной записи и войдите снова: оболочка находит расширение KeySwitch только при входе.
- `KeySwitch-0.65.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.65.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Исправление опечаток: русское слово от пяти букв с одной ошибкой (переставлены две буквы, пропущена или лишняя буква, задета соседняя клавиша) исправляется на границе слова, если одно исправление явно лучше остальных: `нромально` становится `нормально`. Слово, которое знает словарь любой из раскладок, не трогается. Включено по умолчанию; английские слова исправляются, только если включить это в настройках. `Ctrl+Alt+Z` возвращает слово, а слово, дважды оставленное как набрано или возвращённое, дальше считается своим; этот личный словарь хранит солёные хеши, а не слова. Слово, набранное сразу после ручной смены раскладки, остаётся в выбранной раскладке, но опечатка в нём исправляется.
- Две заглавные в начале слова исправляются: `ПРивет` становится `Привет`, `HEllo` — `Hello`. Сокращения, слова прописными, имена вида CamelCase и слова, набранные с Caps Lock, не трогаются. Включено по умолчанию.
- Команды для выделенного текста, как в Punto Switcher: `Shift+Pause` перепечатывает выделение в другой раскладке и переключает на неё (в выделении из двух алфавитов — только слова того, которого меньше), `Alt+Pause` меняет регистр по кругу, `Alt+Scroll Lock` транслитерирует кириллицу (ICAO Doc 9303) и обратно; в macOS — `Control+Option+L`, `U` и `T`. Результат снова выделяется, команду можно повторить, буфер обмена возвращается как был.
- Тема окна настроек следует тёмному режиму Windows и macOS.
- `Ctrl+Alt+Z` отменяет последнее исправление и после пробелов, набранных за ним.
- Окно настроек: ручная смена раскладки в поле проверки защищает слово так же, как в других программах; настройка ручной раскладки показана один раз.
- Windows: в Блокнот Windows 11, который теряет символы, набранные поверх выделения, результат команды вставляется из буфера обмена, и буфер возвращается; клавиши исправления ждут 50 мс после смены раскладки, пока служба ввода поля её примет; клавиша, которую KeySwitch не отдаёт окну при зажатом Alt или Win (`Alt+Pause`), сопровождается клавишей без действия, чтобы окно не открывало меню, а Win — меню «Пуск».
- Русское слово из одной буквы, набранное в английской раскладке после русского текста, исправляется: `и d документ` становится `и в документ`. Если модель уверена на пробеле после буквы, буква исправляется сразу; иначе она ждёт следующего слова. Латинская буква, названная после русского текста, остаётся (`язык C`, `витамин D`).
- Флаг командной строки решается как флаг: `git -v` больше не становится `git -м`, а `-цфеср`, набранное в русской раскладке, становится `-watch`.
- Цифры с `ю` в русской раскладке становятся цифрами с точкой: `пункт 2ю` — `пункт 2.`, `версии 5ю2` — `версии 5.2`. Заменяется только знак, русская раскладка остаётся. `5б` не трогается.
- Русское сообщение, начатое кавычкой в английской раскладке, исправляется с первого слова: `@Z сейчас не могу` становится `"Я сейчас не могу`. `@media`, `@property` и упоминания остаются.
- Короткий ответ в английской раскладке, отправленный Enter, исправляется до отправки: `jr` становится `ок` в Telegram, и с пробелом перед Enter тоже. `ok`, `ha`, `no` остаются.
- Pause, когда переводить нечего, переключает раскладку и больше не защищает следующее слово от исправления.
- Короткое слово на паузе в наборе ждёт конца слова там, где с его двух букв начинается много слов своего языка: `ша` в `шаблоне` больше не становится `if`, пока слово ещё набирается.
- Окно настроек: название настройки переносится рядом с переключателем, а не обрезается; в «О программе» упомянута и macOS.
- Страница продукта публикуется на GitHub Pages вместе с репозиторием APT.

### Модель

- Контекстная модель `context-v3-48a863e75bb6` (SHA-256 артефакта `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) и модель префиксов `prefix-v2-bf3dc28f8567`: пять новых частей модели поверх пары 0.64. На журналах реального набора она исправляет 24 слова и не портит ни одного против 0.64.1. Запечатанный тест с этого выпуска крупнее: до 512 документов на язык. На тесте v84, прочитанном один раз, без ранней смены раскладки восстановлены 720 и 721 из 727 строк, набранных не в той раскладке, против 713 и 716 у базовой пары, и не испорчено ни одной правильной строки.

## English

KeySwitch now corrects typos and two capitals at the start of a word, changes the layout, case and transliteration of the selected text, follows the system's dark theme and recognises five more kinds of layout slips: one-letter Russian words, command-line flags, digits with a dot, a quotation mark opening a message and short replies such as `jr` → `ок`. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.65.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.65.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.65.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.65.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.65.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.65.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.65.0_amd64.deb` — Ubuntu/Xubuntu on an X11 or Wayland session (GNOME, KDE Plasma 6, sway). With 0.28.0 or newer installed, this version arrives through the ordinary system update. In a Wayland session, enable the access to the keyboards once: `sudo keyswitch --enable-wayland-input` (any program of that user can then read every key and type keys of its own, as on X11 and Windows; `sudo keyswitch --disable-wayland-input` takes the access away at once); in GNOME, log out and back in after the first installation: the shell finds the KeySwitch extension only at login.
- `KeySwitch-0.65.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.65.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- Typo correction: a Russian word of five letters and more with one slip in it (two letters swapped, a letter left out or typed in excess, a neighbouring key hit) is corrected at its boundary when a single correction clearly wins: `нромально` becomes `нормально`. A word either layout's dictionary knows is left alone. It is on by default; English words are corrected only with the setting turned on. `Ctrl+Alt+Z` takes a correction back, and a word left as typed twice, or taken back, is the user's own from then on; that personal dictionary stores salted hashes, never the words. A word typed right after switching the layout by hand keeps that layout and still gets its slip fixed.
- Two capitals at the start of a word are fixed: `ПРивет` becomes `Привет`, `HEllo` becomes `Hello`. Abbreviations, words in capitals, CamelCase names and words typed under Caps Lock are left as typed. On by default.
- Commands on the selected text, as in Punto Switcher: `Shift+Pause` types the selection in the other layout and switches to it (in a selection in two alphabets only the words of the rarer one), `Alt+Pause` changes its case in a cycle, `Alt+Scroll Lock` transliterates Cyrillic (ICAO Doc 9303) and back; on macOS `Control+Option+L`, `U` and `T`. The result is selected again, so a command can be repeated, and the clipboard is given back as it was.
- The settings window's theme follows the dark mode of Windows and macOS.
- `Ctrl+Alt+Z` takes the last correction back also after spaces typed after it.
- Settings window: a layout switched by hand in the test field protects the word as in other programs, and the manual-layout setting is shown once.
- Windows: in Notepad of Windows 11, which loses characters typed over a selection, the result of a selection command is pasted and the clipboard given back; the keys of a correction wait 50 ms after a layout switch, until the field's text service takes the new layout; a key KeySwitch keeps from the window while Alt or Win is held (`Alt+Pause`) is followed by a key that types nothing, so the window opens no menu bar, and Win no Start menu, on the release.
- A Russian word of one letter typed in the English layout after Russian text is corrected: `и d документ` becomes `и в документ`. When the model is sure at the letter's space the letter is converted there; otherwise it waits for the next word. A Latin letter named after Russian text stays (`язык C`, `витамин D`).
- A command-line flag is decided as a flag: `git -v` no longer becomes `git -м`, and `-цфеср` typed with the Russian layout on becomes `-watch`.
- Digits with `ю` typed in the Russian layout become digits with a dot: `пункт 2ю` becomes `пункт 2.`, `версии 5ю2` becomes `версии 5.2`. Only the sign is replaced and the Russian layout stays on. `5б` is left as written.
- A Russian message begun with a quotation mark in the English layout is corrected from its first word: `@Z сейчас не могу` becomes `"Я сейчас не могу`. `@media`, `@property` and mentions are left as written.
- A short reply typed in the English layout and sent with Enter is corrected before it is sent: `jr` becomes `ок` in Telegram, also with a space before Enter. `ok`, `ha`, `no` stay as written.
- Pause with nothing to convert switches the layout and no longer holds the next word from correction.
- A short word at a pause in typing waits for the word's end where its two letters begin many words of their own language: `ша` of `шаблоне` no longer becomes `if` while the word is still being typed.
- Settings window: a setting's title wraps beside its control instead of being cut; About names macOS as well.
- The product page is published on GitHub Pages together with the APT repository.

### Model

- The context model `context-v3-48a863e75bb6` (artifact SHA-256 `9a3486e87873a3e74fc6d27f80111deff945d0986ba6eb8dd55c632cef7b5ace`) and the prefix model `prefix-v2-bf3dc28f8567`: five new parts of the model on the pair of 0.64. On the recorded real typing it fixes 24 words and breaks none against 0.64.1. The sealed test is larger from this release: up to 512 documents per language. On sealed test v84, read once, it restores 720 and 721 of 727 rows typed in the wrong layout with the early switch off, against 713 and 716 for the baseline pair, and spoils no correct row.

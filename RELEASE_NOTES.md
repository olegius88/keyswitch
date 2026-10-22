# KeySwitch 0.61.0

## Русский

В этом выпуске модель научилась отдельно решать токены из букв и цифр, набранные в русской раскладке: `шзм6` становится `ipv6`, `зь2` — `pm2`, `ч64` — `x64`, а `1с`, `3д` и `100мс` остаются. Первое слово сообщения, набранного целиком в неверной раскладке, теперь исправляется вместе со следующими словами ещё в четырёх случаях (`Её`, `всю`, `If`, `ой`). Движок стал быстрее примерно на пятую часть. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.61.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.61.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.61.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.61.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.61.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.61.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.61.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.61.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.61.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Токен из русских букв и цифр, клавиши которого дают латинские буквы и цифры, решает отдельная часть модели, если известно, как такой токен пишут в русском тексте: `шзм6` становится `ipv6`, `зь2` — `pm2`, `ч64` — `x64`, `2аф` — `2fa`, `ншш2` — `yii2`, а `1с`, `3д`, `100мс` и `2фа` остаются. Раньше модель читала любую цифру одинаково: такие токены оставались подсказкой, токен из одной буквы с цифрами не исправлялся никогда, а русский токен с цифрами мог стать латиницей. Как часто пишется каждый вариант, берётся из новой таблицы, посчитанной по архиву ru.stackoverflow 2023 года. Токен, которого в таблице нет (`м67` вместо `v67`), набранный после английского текста или набранный латинскими буквами, решается как раньше.
- Первое слово сообщения, набранного целиком в неверной раскладке, исправляется вместе со следующими словами ещё в четырёх случаях: когда последняя клавиша слова — буква другой раскладки (`` T` `` — это `Её`, `dc.` — `всю`), когда следующее слово исправлено со знаком на конце (`Ша тщеб` — это `If not,`) и когда слова нет в словаре программы, хотя в русской прозе оно встречается часто (`jq` — это `ой`). На фразах Tatoeba, набранных с неверной раскладкой в начале, пара теперь уступает старой базовой паре 3 первыми словами из 6000 вместо 8.
- Движок читает настройки без копирования на каждом нажатии и разбирает горячие клавиши один раз: обработка набранного слова стала быстрее примерно на пятую часть.
- Файл контекстной модели записывается по признаку на строку, чтобы изменения модели были видны при сравнении версий; такой файл может занимать до 16 МиБ.

### Модель

- Контекстная модель `context-v3-e02b44de73db` — модель 0.60.0 с обученной поверх неё частью для токенов из букв и цифр; модель префиксов `prefix-v2-bf3dc28f8567` та же. На журналах реального набора она не портит ни одного слова и исправляет два. На запечатанном тесте v73, прочитанном один раз, сохранены все 191 правильно набранная строка, а без ранней смены раскладки восстановлены все 190 строк, набранных не в той раскладке, против 189 у базовой пары; с ранней сменой — 20 и 20. На запечатанном экзамене по переписке разработчиков — три сообщения выиграно, ни одного не проиграно.

### Что осталось

- Токен из букв и цифр, которого нет в таблице частот (`м67` вместо `v67`, `ц4и` вместо `w4b`), набранный в русской раскладке, остаётся как набран: по форме токена его не отличить от русского (`2х`, `ГОСТ2012`). Его исправляет `Pause`.
- Первое слово с открывающей кавычкой, набранное в английской раскладке (`@Z` вместо `"Я`), остаётся: `@` в начале слова бывает и упоминанием. Его исправляет `Pause`.
- Сборку x86 проверяли тестами и проверкой нажатий на 64-битной Windows, где 32-битные программы работают через WOW64: машин с 32-битной Windows в CI нет.
- Где KeySwitch не может прочитать поле, термин из двух букв один в начале сообщения по-прежнему ждёт следующего слова или паузы, если его не перевела ранняя смена раскладки, и остаётся кириллицей, если следом идёт русское слово (`йк истек`). Его исправляет `Pause`.
- Техническое слово, которого нет в таблицах терминов (`цуиоы` вместо `webjs`, `реьч` вместо `htmx`), впервые набранное в русской раскладке посреди русской фразы, часто остаётся кириллицей: без того же слова раньше в тексте модель не отличает его от незнакомого русского слова, сленга или опечатки. Его исправляет `Pause`.
- Короткий английский ответ, набранный целиком в русской раскладке (`щл` вместо `ok`, `тщ` вместо `no`, `рш` вместо `hi`), остаётся: новая часть модели для коротких ответов берёт только латинские буквы, иначе она превращала бы в латиницу русские слова, набранные как задумано. Не исправляются пока и `jr` («ок»), `tot` («еще»), отправленные одним словом. Их исправляет `Pause`.
- Английское слово через дефис, набранное целиком в русской раскладке (`зку-щквук` вместо `pre-order`), исправляется только после дефиса: получается `зку-order`.
- Термин, процитированный в сообщении об ошибке так, как он набрался (`webjs набралось как цуиоы`), переводится в `webjs`, если этот же термин уже набран выше. Вернуть набранное можно клавишей `Pause` сразу после исправления.
- Латинская аббревиатура, набранная в русской раскладке после английского текста, восстанавливается не всегда: `Ask РК about the leave` (`HR`) остаётся `РК`. Русская аббревиатура после латиницы без словарей Hunspell иногда становится латиницей: `deploy staging ЗП` становится `deploy staging PG`. Слово исправляет или возвращает клавиша `Pause`.
- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `кз` в русской фразе (`говорит он кз домене`) может стать `rp`: в таблицах частот, которыми модель проверяет слова, нет разговорного русского, а `rp` там встречается. Вернуть слово можно клавишей `Pause`.
- Русская аббревиатура заглавными, набранная в английской раскладке посреди русской фразы (`DKRCV` вместо «ВЛКСМ»), остаётся латиницей: по имеющимся данным её не отличить от латинской аббревиатуры (`по версии WBC`). Её исправляет клавиша `Pause`.
- Однобуквенная латинская переменная, оставленная на паузе (`ч` и пауза), и русское сокращение из одной части, набранное в английской раскладке (`cv/` вместо `см.`), остаются как набраны: первое может быть началом недописанного слова, второе — частью пути (`dev/`). Их исправляет `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

In this release the model decides tokens of letters and digits typed in the Russian layout with a part of its own: `шзм6` becomes `ipv6`, `зь2` `pm2`, `ч64` `x64`, while `1с`, `3д` and `100мс` stay. The first word of a message typed wholly in the wrong layout now converts with the words after it in four more cases (`Её`, `всю`, `If`, `ой`). The engine is about a fifth faster. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.61.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.61.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.61.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.61.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.61.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.61.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.61.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.61.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.61.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A token of Russian letters and digits whose keys are Latin letters and digits is decided by a part of the model of its own when Russian text is known to write it one way or the other: `шзм6` becomes `ipv6`, `зь2` `pm2`, `ч64` `x64`, `2аф` `2fa`, `ншш2` `yii2`, while `1с`, `3д`, `100мс` and `2фа` stay. The model used to read every digit alike: such tokens stayed a suggestion, a token of one letter and digits was never converted, and a Russian token with digits could turn Latin. How often each spelling is written comes from a new table counted on the 2023 ru.stackoverflow dump. A token the table does not count (`м67` for `v67`), one typed after English text and one typed in Latin letters are decided as before.
- The first word of a message typed wholly in the wrong layout converts with the words after it in four more cases: when its last key is a letter of the other layout (`` T` `` is `Её`, `dc.` is `всю`), when the next word converts with a sign at its end (`Ша тщеб` is `If not,`), and when the program's lexicon lacks the word though Russian prose uses it often (`jq` is `ой`). On Tatoeba sentences typed starting in the wrong layout the pair now loses 3 first words in 6 000 to the old baseline pair, against 8.
- The engine reads its settings without copying them on every key and parses each hotkey once: a typed word is handled about a fifth faster.
- The context model file is written one feature to a line, so that changes of the model show when versions are compared; such a file may take up to 16 MiB.

### Model

- The context model `context-v3-e02b44de73db` is the model of 0.60.0 with the part for tokens of letters and digits fitted onto it; the prefix model `prefix-v2-bf3dc28f8567` is the same. On the recorded real typing it breaks no word and fixes two. On sealed test v73, read once, it keeps all 191 correctly typed rows and restores all 190 rows typed in the wrong layout with the early switch off, against 189 for the baseline pair; with it, 20 and 20. On the sealed developer-chat test it gains three messages and loses none.

### What remains

- A token of letters and digits the frequency table does not count (`м67` for `v67`, `ц4и` for `w4b`), typed in the Russian layout, stays as typed: by its shape it cannot be told from a Russian one (`2х`, `ГОСТ2012`). `Pause` converts it.
- A first word opened by a quotation mark typed in the English layout (`@Z` for `"Я`) stays: `@` at the start of a word is also a mention. `Pause` converts it.
- The x86 build is tested, keystrokes included, on 64-bit Windows, where 32-bit programs run through WOW64: CI has no machine with a 32-bit Windows.
- Where KeySwitch cannot read the field, a term of two letters alone at the start of a message still waits for the next word or a pause unless the early switch converted it, and stays Cyrillic when a Russian word follows (`йк истек`). `Pause` corrects it.
- A technical word the term tables do not count (`цуиоы` for `webjs`, `реьч` for `htmx`), typed in the Russian layout amid a Russian phrase for the first time in a text, often stays Cyrillic: without the same word earlier in the text the model cannot tell it from an unknown Russian word, slang or a typo. `Pause` corrects it.
- A short English reply typed whole in the Russian layout (`щл` for `ok`, `тщ` for `no`, `рш` for `hi`) stays: the new part of the model for short replies takes Latin letters alone, as reading Russian letters it would turn Russian words typed as meant into Latin. `jr` («ок») and `tot` («еще») sent alone do not convert yet either. `Pause` corrects them.
- An English word joined by a hyphen and typed whole in the Russian layout (`зку-щквук` for `pre-order`) is converted only after the hyphen: it becomes `зку-order`.
- A term quoted in a bug report the way it came out (`webjs набралось как цуиоы`) is converted to `webjs` when the same term was typed earlier in the text. `Pause` right after the correction brings back what was typed.
- A Latin abbreviation typed in the Russian layout after English text is not always restored: `Ask РК about the leave` (`HR`) stays `РК`. Without Hunspell dictionaries a Russian abbreviation after Latin-script text sometimes becomes Latin: `deploy staging ЗП` becomes `deploy staging PG`. `Pause` corrects the word or brings it back.
- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the press cannot tell an undo of a wrong correction from a press out of habit.
- `кз` in a Russian phrase (`говорит он кз домене`) may become `rp`: the frequency tables the model checks words with hold no conversational Russian, while `rp` occurs there. `Pause` brings the word back.
- A Russian abbreviation in capitals typed in the English layout amid a Russian phrase (`DKRCV` for `ВЛКСМ`) stays Latin: with the evidence at hand it cannot be told from a Latin abbreviation (`по версии WBC`). `Pause` corrects it.
- A one-letter Latin variable left at a pause (`ч` and a pause) and a Russian abbreviation of one part typed in the English layout (`cv/` for `см.`) stay as typed: the first may open a word not yet finished, the second may be a part of a path (`dev/`). `Pause` corrects them.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

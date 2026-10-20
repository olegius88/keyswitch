# KeySwitch 0.59.0

## Русский

В этом выпуске KeySwitch переводит однобуквенную латинскую переменную, набранную в русской раскладке после русского слова (`при ч равном` → `при x равном`), и русские сокращения из нескольких частей, набранные в английской раскладке (`n/r/` → `т.к.`), а латинское слово в кавычках и код с двоеточиями (`ЭИщще` → `"Boot`) больше не остаются как набраны. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.59.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.59.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.59.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.59.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.59.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.59.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.59.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.59.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.59.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Однобуквенная латинская переменная, набранная в русской раскладке сразу после русского слова, переводится на пробеле или знаке после неё: `при ч равном нулю` становится `при x равном нулю`, `результат т - 1` — `результат n - 1`. Буква, написанная как задумано, остаётся: `да я про то ч`, `и т. д.`, `по оси Х`, `если б`, `в 2 ч дня`. Буква, оставленная на паузе, может начинать недописанное слово и по-прежнему остаётся как набрана.
- Русское сокращение из двух и более частей, набранное в английской раскладке, переводится: `n/r/` становится `т.к.`, `n/t/` — `т.е.`, `l/,/` — `д.б.`. Латинские токены из коротких частей остаются как набраны: `n/a`, `I/O`, `usr/bin`, `tcp/ip`, `src/app/`.
- Латинское слово в кавычках и код с двоеточиями, набранные в русской раскладке, переводятся, когда этого требует модель: `ЭИщще` становится `"Boot`, `ыевЖЖыцфз` — `std::swap`. Раньше правило, по которому замена должна быть словом, отменяло модель, потому что в таком прочтении есть знаки.

### Модель

- Контекстная модель `context-v3-1ba380ab2999` (корпус v69) — модель 0.58.0, поверх которой обучены две новые части: для переменной из одной буквы и для сокращений с точками; все остальные части перенесены без изменений, модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. На журналах реального набора она не портит ни одного слова; на сообщениях разработчиков (часть для разработки) восстанавливает на 34, 23 и 2 сообщения больше в трёх видах ошибок раскладки и ни одного не теряет; на калибровочных данных — на один кадр больше в каждом профиле. На запечатанном тесте v69, прочитанном один раз, она сохранила все 182 правильно набранные строки и без ранней смены раскладки восстановила все 181 строку, набранную не в той раскладке, против 179 у базовой пары, а с ранней сменой — 16, столько же, сколько базовая; на запечатанном тесте сообщений разработчиков (1 125 сообщений новой части) восстановила на 11 сообщений больше, чем 0.58.0, и ни одного не потеряла.

### Что осталось

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

This release makes KeySwitch convert a one-letter Latin variable typed in the Russian layout after a Russian word (`при ч равном` → `при x равном`) and Russian abbreviations of several parts typed in the English layout (`n/r/` → `т.к.`), and a Latin word in quotes or code joined by colons (`ЭИщще` → `"Boot`) no longer stays as typed. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.59.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.59.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.59.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.59.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.59.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.59.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.59.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.59.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.59.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A one-letter Latin variable typed in the Russian layout right after a Russian word converts at the space or sign after it: `при ч равном нулю` becomes `при x равном нулю`, `результат т - 1` becomes `результат n - 1`. A letter meant as written stays: `да я про то ч`, `и т. д.`, `по оси Х`, `если б`, `в 2 ч дня`. A letter left at a pause may open a word not yet finished and still stays as typed.
- A Russian abbreviation of two or more parts typed in the English layout converts: `n/r/` becomes `т.к.`, `n/t/` `т.е.`, `l/,/` `д.б.`. Latin tokens of short parts stay as typed: `n/a`, `I/O`, `usr/bin`, `tcp/ip`, `src/app/`.
- A Latin word in quotes and code joined by colons, typed in the Russian layout, convert when the model asks for it: `ЭИщще` becomes `"Boot`, `ыевЖЖыцфз` becomes `std::swap`. The rule that a replacement must be a word used to overrule the model there, as such a reading holds signs.

### Model

- The context model `context-v3-1ba380ab2999` (corpus v69) is the model of 0.58.0 with two new parts fitted onto it, for a one-letter variable and for abbreviations with dots; every other part is carried over unchanged, and the prefix model stays `prefix-v2-bf3dc28f8567`. On the logs of real typing it breaks no word; of the developer-chat development messages it restores 34, 23 and 2 more in three kinds of layout mistakes and loses none; on the calibration data it nets one frame more in each profile. On the sealed test v69, read once, it kept all 182 correctly typed sequences and restored all 181 typed in the wrong layout without the early switch, against 179 for the baseline pair, and 16 with it, as many as the baseline pair; on the sealed developer-chat test (1 125 messages of a new batch) it restored 11 messages more than 0.58.0 and lost none.

### What remains

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

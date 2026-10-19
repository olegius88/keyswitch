# KeySwitch 0.58.0

## Русский

В этом выпуске KeySwitch сразу переводит и те короткие термины в начале сообщения, которые совпадают с окончанием русских слов (`шь` → `im`, `уче` → `ext`), если поле прочитано и пустое; остальные части модели остались прежними. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.58.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.58.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.58.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.58.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.58.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.58.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.58.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.58.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.58.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Две-три буквы термина, которыми кончается много русских слов (`шь`, `уче`, `ев`), набранные одни в начале сообщения в поле, которое KeySwitch прочитал и нашёл пустым, переводятся на пробеле: `im`, `ext`, `td`. 0.57.0 оставляла их как набраны, принимая за окончание слова, которое допечатывают. Такое окончание одно в пустом поле не стоит: когда вы допечатываете слово после Backspace, KeySwitch решает всё слово целиком (`моло` и `дые` остаются «молодые»). Остановка посреди слова за его конец не считается: `уче`, пауза, `ба` остаётся «учеба». Где поле прочитать нельзя, ничего не изменилось.

### Модель

- Контекстная модель `context-v3-9033af7cb039` (корпус v67) — модель 0.57.0, в которой заново обучена только часть для короткого термина в начале сообщения; все остальные части перенесены из 0.57.0 без изменений, модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. На калибровочных данных она решает так же, как модель 0.57.0, на журналах реального набора не портит и не меняет ни одного слова, а среди сообщений из переписки разработчиков, начатых в другой раскладке, восстанавливает на 15 больше и ни одного не теряет. На запечатанном test v67 правильный текст сохранён во всех 175 строках, а без ранней смены восстановлены все 174 строки, набранные не в той раскладке, против 173 у базовой пары. На запечатанном test из переписки разработчиков она восстановила на одно сообщение, начатое в другой раскладке, больше и ни одного не потеряла.

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
- Латинское слово в кавычках или код с двоеточиями, набранные в русской раскладке (`ЭИщще` вместо `"Boot`, `ыевЖЖмусещк` вместо `std::vector`), остаются как набраны: модель их переводит, но правило, по которому замена должна быть словом, её отменяет. Их исправляет `Pause`.
- Однобуквенная латинская переменная, набранная в русской раскладке посреди русской фразы (`ч` вместо `x`, `т` вместо `n`), остаётся кириллицей: одиночным буквам модель не обучена, а в обычной переписке такая буква чаще русская. Её исправляет `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

This release makes KeySwitch convert at once also the short terms opening a message that look like the end of a Russian word (`шь` → `im`, `уче` → `ext`), when the field was read and found empty; every other part of the model is the same. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.58.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.58.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.58.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.58.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.58.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.58.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.58.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.58.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.58.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- Two or three letters of a term that end many Russian words (`шь`, `уче`, `ев`), typed alone at the start of a message in a field KeySwitch read and found empty, convert at the space: `im`, `ext`, `td`. 0.57.0 kept them as typed, taking them for the end of a word being finished. Such an ending does not stand alone in an empty field: when you finish a word after Backspace, KeySwitch decides the whole word (`моло` and `дые` stay «молодые»). A stop in the middle of a word is not taken for its end: `уче`, a pause, `ба` stays «учеба». Where the field cannot be read nothing changed.

### Model

- The context model `context-v3-9033af7cb039` (corpus v67) is the model of 0.57.0 with only its part for a short term opening a message fitted again; every other part is carried over from 0.57.0 unchanged, and the prefix model stays `prefix-v2-bf3dc28f8567`. On the calibration data it decides as the model of 0.57.0 does; on the logs of real typing it breaks and changes no word; of the developer-chat messages started in the other layout it restores 15 more and loses none. On the sealed test v67 it kept the correct text in all 175 rows and restored all 174 rows typed in the wrong layout without the early switch, against 173 for the baseline pair. On the sealed test of developer chat it restored one more message started in the other layout and lost none.

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
- A Latin word in quotes or code with colons typed in the Russian layout (`ЭИщще` for `"Boot`, `ыевЖЖмусещк` for `std::vector`) stays as typed: the model converts it, but the rule that a replacement must be a word overrules it. `Pause` corrects it.
- A one-letter Latin variable typed in the Russian layout amid a Russian phrase (`ч` for `x`, `т` for `n`) stays Cyrillic: the model has not been trained on single letters, and in ordinary chat such a letter is more often Russian. `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

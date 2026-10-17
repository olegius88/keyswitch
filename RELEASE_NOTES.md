# KeySwitch 0.57.0

## Русский

В этом выпуске KeySwitch с первого раза исправляет технический термин, набранный в русской раскладке (`зк` вместо `pr`, `сш` вместо `ci`), сразу переводит короткий термин в начале сообщения (`йк` → `qr`) и исправляет короткий ответ, набранный целиком в английской раскладке (`ytn` вместо «нет»). Исправление на знаке больше не теряется при быстром наборе, а слова, похожие на пароль, записываются в технический журнал звёздочками. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.57.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.57.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.57.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.57.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.57.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.57.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.57.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.57.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.57.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Технический термин, набранный в русской раскладке, исправляется с первого раза: `зк` → `pr`, `ЗК` → `PR` в конце вопроса, `сш` → `ci` в `npm ci`, `рвьш` → `hdmi`. Раньше такой термин исправлялся, только если тот же термин уже был набран выше в тексте. Окончание русского слова, которое вы допечатываете отдельным токеном (`ев`, `ась`), и слово, которое знает словарь, остаются как есть.
- Короткий термин один в начале сообщения переводится сразу, на пробеле, если KeySwitch прочитал поле и оно пустое: `йк` → `qr`, `зк` → `pr`. Раньше, если не срабатывала ранняя смена раскладки, он ждал паузы в полторы секунды. Следом идущее русское слово его больше не откатывает: `йк истек` становится `qr истек`.
- Термин, исправленный посреди русской фразы, больше не откатывается, когда следующее слово из одной русской буквы набралось латиницей (раскладка уходит за исправлением): `принимай зк и` теперь становится `принимай pr и`, а не возвращается к `зк и`.
- Короткий ответ, набранный целиком в английской раскладке и отправленный Enter или оставленный на паузе, исправляется: `ytn` → «нет», `dct` → «все». Слово, которое бывает и тем и другим (`vs` и «мы»), остаётся как набрано.
- Исправление слова, после которого стоит знак, больше не теряется, если пробел нажат раньше, чем отпущена клавиша знака: при быстром наборе `bcntr?` с пробелом оставалось как было, хотя модель решила исправить слово на `истек,`, а следующее слово исправлялось (`bcntr? но`). Теперь пробел удаляется и набирается заново вместе с исправлением.
- Слова, похожие на пароль, записываются в технический журнал звёздочками: слово сразу после «пароль» или «password» (в любой раскладке) и любой токен от восьми знаков, в котором есть и буквы, и цифры. Пароль короче восьми знаков или из одних букв, набранный без такого слова перед ним, записывается как набран: технический журнал по-прежнему стоит включать только на время диагностики.

### Модель

- Контекстная модель `context-v3-754513f80498` (корпус v65) добавляет к модели 0.56.0 три новые части: для терминов, набранных в русской раскладке, для коротких ответов в английской раскладке и для короткого термина в начале сообщения; модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. На калибровочных данных она делает на 39 и 60 верных исправлений больше модели 0.56.0, на журналах реального набора не портит ни одного слова, которое 0.56.0 исправляла верно, и исправляет ещё 11. На запечатанном test v65 правильный текст сохранён во всех 182 строках, а без ранней смены восстановлены все 182 строки, набранные не в той раскладке, против 180–181 у базовой пары. На запечатанном test из переписки разработчиков она восстановила на 15 сообщений больше и на 2 меньше, когда смена на английский была пропущена, и на 3 больше среди сообщений, начатых в другой раскладке.

### Что осталось

- Сборку x86 проверяли тестами и проверкой нажатий на 64-битной Windows, где 32-битные программы работают через WOW64: машин с 32-битной Windows в CI нет.
- Две-три буквы, которыми кончается много русских слов (`шь`, `уче`, `зь`), набранные одни в начале сообщения в поле, которое KeySwitch прочитал, остаются как набраны, даже если это термин (`im`, `ext`, `pm`): по одному токену не отличить термин от окончания слова, которое допечатывают. 0.56.0 переводила их после паузы. Их исправляет клавиша `Pause`.
- Где KeySwitch не может прочитать поле, термин из двух букв один в начале сообщения по-прежнему ждёт следующего слова или паузы, если его не перевела ранняя смена раскладки, и остаётся кириллицей, если следом идёт русское слово (`йк истек`). Его исправляет `Pause`.
- Техническое слово, которого нет в таблицах терминов (`цуиоы` вместо `webjs`, `реьч` вместо `htmx`), впервые набранное в русской раскладке посреди русской фразы, часто остаётся кириллицей: без того же слова раньше в тексте модель не отличает его от незнакомого русского слова, сленга или опечатки. Его исправляет `Pause`.
- Короткий английский ответ, набранный целиком в русской раскладке (`щл` вместо `ok`, `тщ` вместо `no`, `рш` вместо `hi`), остаётся: новая часть модели для коротких ответов берёт только латинские буквы, иначе она превращала бы в латиницу русские слова, набранные как задумано. Не исправляются пока и `jr` («ок»), `tot` («еще»), отправленные одним словом. Их исправляет `Pause`.
- Английское слово через дефис, набранное целиком в русской раскладке (`зку-щквук` вместо `pre-order`), исправляется только после дефиса: получается `зку-order`.
- Термин, процитированный в сообщении об ошибке так, как он набрался (`webjs набралось как цуиоы`), переводится в `webjs`, если этот же термин уже набран выше. Вернуть набранное можно клавишей `Pause` сразу после исправления.
- Латинская аббревиатура, набранная в русской раскладке после английского текста, восстанавливается не всегда: `Ask РК about the leave` (`HR`) остаётся `РК`. Русская аббревиатура после латиницы без словарей Hunspell иногда становится латиницей: `deploy staging ЗП` становится `deploy staging PG`. Слово исправляет или возвращает клавиша `Pause`.
- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `кз` в русской фразе (`говорит он кз домене`) может стать `rp`: в таблицах частот, которыми модель проверяет слова, нет разговорного русского, а `rp` там встречается. Вернуть слово можно клавишей `Pause`.
- Русская аббревиатура заглавными, набранная в английской раскладке посреди русской фразы (`DKRCV` вместо «ВЛКСМ»), остаётся латиницей: по имеющимся данным её не отличить от латинской аббревиатуры (`по версии WBC`). Её исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

This release makes KeySwitch correct a technical term typed in the Russian layout the first time (`зк` for `pr`, `сш` for `ci`), convert a short term opening a message at once (`йк` → `qr`) and correct a short reply typed whole in the English layout (`ytn` for «нет»). A correction at a sign is no longer lost to fast typing, and words that look like a password go into the technical log as stars. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.57.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.57.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.57.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.57.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.57.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.57.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.57.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.57.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.57.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A technical term typed in the Russian layout converts the first time: `зк` → `pr`, `ЗК` → `PR` at the end of a question, `сш` → `ci` in `npm ci`, `рвьш` → `hdmi`. Such a term used to convert only when the same term had been typed earlier in the text. The end of a Russian word typed as a token of its own to finish it (`ев`, `ась`), and a word the dictionary knows, stay as typed.
- A short term alone at the start of a message converts at once, at the space, when KeySwitch read the field and found it empty: `йк` → `qr`, `зк` → `pr`. Unless the early switch had converted it, it used to wait for a pause of a second and a half. A Russian word after it no longer takes it back: `йк истек` becomes `qr истек`.
- A term converted amid a Russian phrase is no longer taken back when the next word, a Russian word of one letter, comes out in Latin letters with the layout that followed the conversion: `принимай зк и` now becomes `принимай pr и` instead of going back to `зк и`.
- A short reply typed whole in the English layout and sent with Enter or left at a pause converts: `ytn` → «нет», `dct` → «все». A token that can be either (`vs` and «мы») stays as typed.
- A correction of a word followed by a sign is no longer lost when the space is pressed before the sign's key comes up: typed fast, `bcntr?` and a space stayed as typed though the model converted the word to `истек,`, while the next word was corrected (`bcntr? но`). The space is now deleted and typed again with the correction.
- Words that look like a password go into the technical log as stars: the word typed right after «пароль» or «password» (in either layout) and every token of eight characters or more with both letters and digits in it. A password shorter than eight characters, or of letters alone, typed without such a word before it, is logged as typed: technical logging is still for the time of a diagnosis only.

### Model

- The context model `context-v3-754513f80498` (corpus v65) adds three parts to the model of 0.56.0: for terms typed in the Russian layout, for short replies typed in the English layout and for a short term opening a message; the prefix model stays `prefix-v2-bf3dc28f8567`. On the calibration data it makes 39 and 60 more right corrections than the model of 0.56.0; on the logs of real typing it breaks no word 0.56.0 got right and corrects 11 more. On the sealed test v65 it kept the correct text in all 182 rows and restored all 182 rows typed in the wrong layout without the early switch, against 180–181 for the baseline pair. On the sealed test of developer chat it restored 15 messages more and 2 fewer where a change to English was forgotten, and 3 more of those started in the other layout.

### What remains

- The x86 build is tested, keystrokes included, on 64-bit Windows, where 32-bit programs run through WOW64: CI has no machine with a 32-bit Windows.
- Two or three letters that end many Russian words (`шь`, `уче`, `зь`), typed alone at the start of a message in a field KeySwitch read, stay as typed even when meant as a term (`im`, `ext`, `pm`): a single token cannot tell a term from the end of a word being finished. 0.56.0 converted them after a pause. `Pause` corrects them.
- Where KeySwitch cannot read the field, a term of two letters alone at the start of a message still waits for the next word or a pause unless the early switch converted it, and stays Cyrillic when a Russian word follows (`йк истек`). `Pause` corrects it.
- A technical word the term tables do not count (`цуиоы` for `webjs`, `реьч` for `htmx`), typed in the Russian layout amid a Russian phrase for the first time in a text, often stays Cyrillic: without the same word earlier in the text the model cannot tell it from an unknown Russian word, slang or a typo. `Pause` corrects it.
- A short English reply typed whole in the Russian layout (`щл` for `ok`, `тщ` for `no`, `рш` for `hi`) stays: the new part of the model for short replies takes Latin letters alone, as reading Russian letters it would turn Russian words typed as meant into Latin. `jr` («ок») and `tot` («еще») sent alone do not convert yet either. `Pause` corrects them.
- An English word joined by a hyphen and typed whole in the Russian layout (`зку-щквук` for `pre-order`) is converted only after the hyphen: it becomes `зку-order`.
- A term quoted in a bug report the way it came out (`webjs набралось как цуиоы`) is converted to `webjs` when the same term was typed earlier in the text. `Pause` right after the correction brings back what was typed.
- A Latin abbreviation typed in the Russian layout after English text is not always restored: `Ask РК about the leave` (`HR`) stays `РК`. Without Hunspell dictionaries a Russian abbreviation after Latin-script text sometimes becomes Latin: `deploy staging ЗП` becomes `deploy staging PG`. `Pause` corrects the word or brings it back.
- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the press cannot tell an undo of a wrong correction from a press out of habit.
- `кз` in a Russian phrase (`говорит он кз домене`) may become `rp`: the frequency tables the model checks words with hold no conversational Russian, while `rp` occurs there. `Pause` brings the word back.
- A Russian abbreviation in capitals typed in the English layout amid a Russian phrase (`DKRCV` for `ВЛКСМ`) stays Latin: with the evidence at hand it cannot be told from a Latin abbreviation (`по версии WBC`). `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

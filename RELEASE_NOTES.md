# KeySwitch 0.60.0

## Русский

В этом выпуске исправлены три ошибки движка, найденные по журналам набора: «и»/«в» между английскими терминами больше не превращаются обратно в `b`/`d` (`sendapi и botcorp`), знак сразу после русского слова не становится буквой (`приветб`), а в браузерах и редакторах на Electron исправление ждёт, пока поле покажет слово (новая настройка «Ждать отстающее поле»). Модели слов и префиксов те же, что в 0.59.0. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.60.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.60.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.60.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.60.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.60.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.60.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.60.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.60.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.60.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Слово, уже исправленное на пробеле, больше не возвращается обратно, когда следующее слово уходит в другую раскладку, если это английский термин или «и»/«в», поставленные правилом между английскими терминами: `sendapi и botcorp` становился `sendapi b botcorp`, `force push в dev` — `force push d dev`, `qemu чтобы` — `йуьг чтобы`, `gpu но` — `пзг но`. Слово, которое модель перевела по ошибке, по-прежнему возвращается: `кз` перед русским словом остаётся `кз`.
- Знак, набранный сразу после последней буквы слова в раскладке другого языка, остаётся знаком, если это не буква слова. После щелчка или Backspace в конце русского слова при английской раскладке `,` `.` `;` превращались в `б` `ю` `ж` (`приветб`, `касаетсяю`); `.` после `сдела` по-прежнему даёт `сделаю`.
- В браузерах и редакторах на Electron (VS Code, Antigravity, Firefox), которые показывают набранное с задержкой, слово, последние буквы которого поле ещё не показало, решается по нажатым клавишам, а исправление ждёт, пока поле покажет слово целиком: так оставалось `ашкуащч` вместо `firefox`. Сколько ждать, задаёт новая настройка «Ждать отстающее поле»: не ждать, 0,1, 0,3 (по умолчанию), 0,5 или 1 секунда. Исправление делается, только когда поле показывает ровно набранное слово.

### Модель

- Модели те же, что в 0.59.0: контекстная `context-v3-1ba380ab2999` и модель префиксов `prefix-v2-bf3dc28f8567`, заново запечатанные для нового движка без изменения единого веса. На журналах реального набора новый движок не портит ни одного слова и исправляет 16. На запечатанном тесте v71, прочитанном один раз, сохранены все 185 правильно набранных строк, а без ранней смены раскладки восстановлены все 185 строк, набранных не в той раскладке, против 184 у базовой пары; с ранней сменой — 19 против 18.

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

This release fixes three engine faults found in the typing logs: `и`/`в` between English terms no longer turn back into `b`/`d` (`sendapi и botcorp`), a sign typed right after a Russian word no longer becomes a letter (`приветб`), and in browsers and Electron editors a correction waits for the field to show the word (the new setting «Ждать отстающее поле»). The models are those of 0.59.0. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.60.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.60.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.60.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.60.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.60.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.60.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.60.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.60.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.60.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A word converted at the space is no longer taken back when the next word goes the other way, if it is an English term or an `и`/`в` the rule set between English terms: `sendapi и botcorp` became `sendapi b botcorp`, `force push в dev` became `force push d dev`, `qemu чтобы` became `йуьг чтобы`, `gpu но` became `пзг но`. A word the model converted by mistake still goes back: `кз` before a Russian word stays `кз`.
- A sign typed right after a word's last letter in the other language's layout stays a sign unless it is the word's letter. After a click or Backspace at the end of a Russian word in the English layout, `,` `.` `;` came out as `б` `ю` `ж` (`приветб`, `касаетсяю`); `.` after `сдела` still gives `сделаю`.
- In browsers and Electron editors (VS Code, Antigravity, Firefox), which show what was typed a moment late, a word whose last letters the field does not show yet is decided on its keys, and the correction waits for the field to show the whole word: `ашкуащч` stayed for `firefox`. The new setting «Ждать отстающее поле» sets how long: off, 0.1, 0.3 (the default), 0.5 or 1 second. The correction is made only once the field shows the very word typed.

### Model

- The models are those of 0.59.0, the context model `context-v3-1ba380ab2999` and the prefix model `prefix-v2-bf3dc28f8567`, sealed again for the new engine without a single weight changed. On the logs of real typing the new engine breaks no word and fixes 16. On the sealed test v71, read once, it kept all 185 correctly typed sequences and restored all 185 typed in the wrong layout without the early switch, against 184 for the baseline pair; with it, 19 against 18.

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

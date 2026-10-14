# KeySwitch 0.55.0

## Русский

Выпуск делает клавишу `Pause` надёжнее: она переводит последнее слово и после пробелов и знаков за ним, а в слове с дефисом — только неверную часть. Технический термин, набранный в русской раскладке, исправляется, если тот же термин уже набран верно раньше в этом тексте. Русское слово, набранное в английской раскладке, исправляется и посреди фразы, если его последняя буква стоит на клавише знака (`х`, `ю`), а слово из двух через дефис — даже если каждая половина похожа на английское слово. Ещё исправлены три ошибки: защита от автоисправления больше не задерживается на чужих словах, отмена `Ctrl+Alt+Z` больше не портит текст, набранный после исправления, а слово, которое вы исключили, больше не исправляется вместе со следующим. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.55.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.55.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.55.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.55.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.55.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.55.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.55.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- `Pause` переводит последнее слово, даже если после него уже набраны пробелы или знаки, одинаковые в обеих раскладках (`-`, `+`, `!`): `цуиоы - ` становится `webjs - `, а знаки печатаются заново как были. Знак, который в другой раскладке другой, остаётся как набран: `ЗК? ` становится `PR? `. Раньше в таких случаях `Pause` только переключал раскладку, и слово приходилось стирать и набирать заново. Буква, стёртый символ, стрелки, щелчок мышью, Enter и Tab по-прежнему завершают слово.
- В слове из частей, соединённых дефисом или другим знаком, одинаковым в обеих раскладках, `Pause` переводит последнюю часть, если части перед ней верны как набраны: `lid-'ne` становится `lid-эту`, а не `дшв-эту`. Слово, голова которого сама похожа на слово другой раскладки, переводится целиком (`ghbdtn-vbh` — `привет-мир`), а `Pause` после исправления программы возвращает его целиком.
- Защита от автоисправления, которую `Pause` даёт незавершённому слову, теперь заканчивается вместе с ним. Раньше, если после `Pause` слово было брошено (стрелки, щелчок), защита переходила на следующее слово в той же раскладке, даже через минуту: так `ntcn` в начале другого поля не стал `тест`. Выбор раскладки вручную по-прежнему защищает следующее слово.
- Отмена `Ctrl+Alt+Z` больше не переписывает текст, набранный после исправленного слова: `привет мир ` с отменой давал `привghbdtn `. Теперь отмена доступна, только пока за исправленным словом ничего не набрано.
- Технический термин, набранный в русской раскладке посреди русской фразы, исправляется, если этот же термин уже набран верно раньше в том же тексте: `webjs не дает информацию… дело в цуиоы` даёт `webjs`. Редкое русское слово, набранное в английской раскладке после русского текста, решает та же новая часть модели (`htpdt` в русской фразе — `резве`); латинское название, набранное верно, остаётся.
- Русское слово, последняя буква которого стоит на клавише знака (`х` — на `[`, `ю` — на `.`), набранное в английской раскладке посреди фразы, теперь исправляется вместе со следующим словом: `в 2 vbkkbjyf[ cdtnjds[` давало `в 2 vbkkbjyf[ световых`, теперь — `в 2 миллионах световых`. Так же в конце сообщения: `lbre. rjire/` становится `дикую кошку.`. Термин со знаком после него, набранный нарочно (`-PHP,`, `UI,`, `repo,`), остаётся как набран.
- Слово, которое вы исключили, или слово, которое держит ваше правило «оставлять», больше не исправляется вместе со следующим словом: исключённое `tot` перед `ghbdtn` давало `еще привет`.
- Русское слово из двух через дефис, набранное в английской раскладке, исправляется, даже если каждая половина — английское слово: `Re-re!` после русской фразы становится `Ку-ку!`.

### Модель

- Контекстная модель `context-v3-0983883efdb4` (корпус v59) закрепляет новый движок; модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. Поверх весов модели 0.44.0 обучены три головы: для аббревиатур после латиницы, для незнакомых токенов и для слов через дефис. На калибровочных данных она делает на 10 и 6 верных исправлений больше модели 0.54.0 при том же числе ложных (15), а на журналах владельца не ошибается ни в одном слове, которое модель 0.54.0 решала верно. На запечатанном test v59 правильный текст сохранён во всех 195 строках, а без ранней смены восстановлено 193 строки из 195, набранных не в той раскладке, против 192 у базовой пары.

### Что осталось

- Короткое техническое слово, впервые набранное в русской раскладке посреди русской фразы (`цуиоы` вместо `webjs`, `мгуоы` вместо `vuejs`, `реьч` вместо `htmx`), часто остаётся кириллицей: без того же слова раньше в тексте модель не отличает его от незнакомого русского слова, сленга или опечатки. Его исправляет `Pause`, теперь и после пробелов и знаков за словом.
- Английское слово через дефис, набранное целиком в русской раскладке (`зку-щквук` вместо `pre-order`), исправляется только после дефиса: получается `зку-order`.
- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `ЗК` заглавными в конце русской фразы (`почему ты не создал ЗК?`) остаётся кириллицей, а не становится `PR`: следующего слова, с которым модель решила бы его, нет. Его исправляет клавиша `Pause`.
- `кз` в русской фразе (`говорит он кз домене`) может стать `rp`: в таблицах частот, которыми модель проверяет слова, нет разговорного русского, а `rp` там встречается. Вернуть слово можно клавишей `Pause`.
- Русская аббревиатура заглавными, набранная в английской раскладке посреди русской фразы (`DKRCV` вместо «ВЛКСМ»), остаётся латиницей: по имеющимся данным её не отличить от латинской аббревиатуры (`по версии WBC`). Её исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

This release makes the `Pause` key more dependable: it converts the last word past the spaces and signs after it, and only the wrong part of a hyphenated word. A technical term typed in the Russian layout is corrected when the same term was typed right earlier in the text. A Russian word typed in the English layout is now corrected in the middle of a sentence too when its last letter sits on a sign key (`х`, `ю`), and a Russian word of two joined by a hyphen even when each half looks like an English word. It also fixes three bugs: the protection from automatic correction no longer lingers onto unrelated words, undo (`Ctrl+Alt+Z`) no longer damages text typed after the correction, and a word you excluded is no longer converted with the word after it. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.55.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.55.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.55.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.55.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build does not update itself yet: download a new version from the releases page.
- `keyswitch_0.55.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.55.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.55.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- `Pause` converts the last word even when spaces or signs typed the same in both layouts (`-`, `+`, `!`) follow it: `цуиоы - ` becomes `webjs - `, and the signs are typed again as they were. A sign that is another one in the other layout stays as typed: `ЗК? ` becomes `PR? `. Before, `Pause` only switched the layout there, and the word had to be erased and typed again. A letter, an erased character, the arrow keys, a click, Enter and Tab still end the word.
- In a word of parts joined by a hyphen or another sign typed the same in both layouts, `Pause` converts the last part when the parts before it are right as typed: `lid-'ne` becomes `lid-эту`, not `дшв-эту`. A word whose head itself reads as a word in the other layout converts whole (`ghbdtn-vbh` is `привет-мир`), and `Pause` after a correction KeySwitch made takes all of it back.
- The protection from automatic correction that `Pause` gives an unfinished word now ends with that word. When the word was left (the arrow keys, a click), the protection used to pass to the next word in that layout, even a minute later: `ntcn` at the start of another field did not become `тест`. Picking a layout by hand still protects the next word.
- Undo (`Ctrl+Alt+Z`) no longer rewrites text typed after the corrected word: `привет мир ` and undo gave `привghbdtn `. Undo is now available only while nothing follows the corrected word.
- A technical term typed in the Russian layout amid a Russian phrase is corrected when the same term was typed right earlier in that text: `webjs не дает информацию… дело в цуиоы` gives `webjs`. A rare Russian word typed in the English layout after Russian text is decided by the same new part of the model (`htpdt` in a Russian sentence is `резве`); a Latin name typed right stays.
- A Russian word whose last letter sits on a sign key (`х` on `[`, `ю` on `.`), typed in the English layout in the middle of a sentence, is now converted with the next word: `в 2 vbkkbjyf[ cdtnjds[` gave `в 2 vbkkbjyf[ световых` and now gives `в 2 миллионах световых`. The same at the end of a message: `lbre. rjire/` becomes `дикую кошку.`. A term typed as intended with a sign after it (`-PHP,`, `UI,`, `repo,`) stays as typed.
- A word you excluded, or one a rule of yours keeps, is no longer converted with the word after it: an excluded `tot` before `ghbdtn` gave `еще привет`.
- A Russian word of two joined by a hyphen, typed in the English layout, is converted even when each half is an English word: `Re-re!` after a Russian sentence becomes `Ку-ку!`.

### Model

- The context model `context-v3-0983883efdb4` (corpus v59) seals the new engine; the prefix model is still `prefix-v2-bf3dc28f8567`. Three heads are fitted onto the weights of the 0.44.0 model: for abbreviations after Latin-script text, for unknown tokens and for words joined by hyphens. On the calibration data it makes 10 and 6 more right conversions than the 0.54.0 model with as many false ones (15), and on the owner's logs it gets no word wrong that the 0.54.0 model got right. On the sealed test v59 it kept all 195 correctly typed rows and, without the early switch, restored 193 of the 195 typed in the wrong layout against 192 for the baseline pair.

### What remains

- A short technical word typed in the Russian layout amid a Russian phrase for the first time in a text (`цуиоы` for `webjs`, `мгуоы` for `vuejs`, `реьч` for `htmx`) often stays Cyrillic: without the same word earlier in the text the model cannot tell it from an unknown Russian word, slang or a typo. `Pause` corrects it, now also past the spaces and signs after the word.
- An English word joined by a hyphen and typed whole in the Russian layout (`зку-щквук` for `pre-order`) is converted only after the hyphen: it becomes `зку-order`.
- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the press cannot tell an undo of a wrong correction from a press out of habit.
- `ЗК` in capitals at the end of a Russian phrase (`почему ты не создал ЗК?`) stays Cyrillic instead of becoming `PR`: there is no next word for the model to decide it with. `Pause` corrects it.
- `кз` in a Russian phrase (`говорит он кз домене`) may become `rp`: the frequency tables the model checks words with hold no conversational Russian, while `rp` occurs there. `Pause` brings the word back.
- A Russian abbreviation in capitals typed in the English layout amid a Russian phrase (`DKRCV` for `ВЛКСМ`) stays Latin: with the evidence at hand it cannot be told from a Latin abbreviation (`по версии WBC`). `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

# KeySwitch 0.54.0

## Русский

Выпуск исправляет слова, склеенные дефисом из частей в разных раскладках, однобуквенное слово при вставке в середину строки и редкие слова, которые модель только предлагала перевести. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.54.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.54.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.54.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.54.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.54.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.54.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.54.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Слово после дефиса решается отдельно, если весь токен целиком не верен: `mongo-cthdbcf` становится `mongo-сервиса`, `docker-j,hfp` — `docker-образ` (раньше получалось `вщслук-образ`). То же для других знаков, одинаковых в обеих раскладках (`_`, `+`, `=`). Знак в конце допечатывается (`mongo-сервиса.`), а `Pause` после такого исправления возвращает только эту часть. Слова, верные целиком (`какой-то`, `из-за`, `e-mail`, `ИИ-ассистент`), решаются как раньше; идентификаторы, адреса и присваивания (`user_name`, `foo=bar`, `user@host`) и одна буква после знака (`usb-c`) не делятся.
- Однобуквенное русское слово, набранное в английской раскладке после английского термина, исправляется и тогда, когда фраза вставлена в середину строки: `почему я не вижу в vscode d` даёт `в`. Раньше слова перед термином брались только из набранного подряд, а после щелчка в середину строки их не было.
- Слово, которое контекстная модель только предлагает перевести, переводится, если перевод подтверждает и детектор раскладки. Так исправляются редкие слова, которых нет в словаре (`byzpt` — «инязе», `ntby` — «теин»). Одиночная буква по-прежнему переводится только по правилам из безопасного списка.

### Модель

- Контекстная модель `context-v3-8849f92656fd` (корпус v54) закрепляет новый движок; модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. На запечатанном test v54 правильный текст сохранён во всех 187 строках, а без ранней смены восстановлено 186–187 строк из 187, набранных не в той раскладке, против 185 у базовой пары.

### Что осталось

- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `ЗК` заглавными в конце русской фразы (`почему ты не создал ЗК?`) остаётся кириллицей, а не становится `PR`: следующего слова, с которым модель решила бы его, нет. Его исправляет клавиша `Pause`.
- `кз` в русской фразе (`говорит он кз домене`) может стать `rp`: в таблицах частот, которыми модель проверяет слова, нет разговорного русского, а `rp` там встречается. Вернуть слово можно клавишей `Pause`.
- Русская аббревиатура заглавными, набранная в английской раскладке посреди русской фразы (`DKRCV` вместо «ВЛКСМ»), остаётся латиницей: по имеющимся данным её не отличить от латинской аббревиатуры (`по версии WBC`). Её исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

This release fixes words joined by a hyphen from parts typed in different layouts, a one-letter word in a phrase typed into the middle of a line, and rare words the model only suggested converting. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.54.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.54.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.54.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.54.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build does not update itself yet: download a new version from the releases page.
- `keyswitch_0.54.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.54.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.54.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- The part after a hyphen is decided on its own when the whole token is not right: `mongo-cthdbcf` becomes `mongo-сервиса`, and `docker-j,hfp` becomes `docker-образ` (it used to come out as `вщслук-образ`). The same goes for other signs typed the same in both layouts (`_`, `+`, `=`). A sign at the end is replayed (`mongo-сервиса.`), and `Pause` after such a correction takes back only that part. Words right as a whole (`какой-то`, `из-за`, `e-mail`, `ИИ-ассистент`) are decided as before; identifiers, addresses and assignments (`user_name`, `foo=bar`, `user@host`) and one letter after a sign (`usb-c`) are not split.
- A one-letter Russian word typed in the English layout after an English term is corrected when the phrase was typed into the middle of a line too: `почему я не вижу в vscode d` gives `в`. The words before the term used to come only from what was typed in a row, and after a click into the middle of a line there were none.
- A word the context model only suggests converting is converted when the layout detector confirms it. Rare words no lexicon holds are corrected this way (`byzpt` is `инязе`, `ntby` is `теин`). A lone letter still converts only by the rules of the safe list.

### Model

- The context model `context-v3-8849f92656fd` (corpus v54) seals the new engine; the prefix model is still `prefix-v2-bf3dc28f8567`. On the sealed test v54 it kept all 187 correctly typed rows and, without the early switch, restored 186 to 187 of the 187 typed in the wrong layout against 185 for the baseline pair.

### What remains

- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the press cannot tell an undo of a wrong correction from a press out of habit.
- `ЗК` in capitals at the end of a Russian phrase (`почему ты не создал ЗК?`) stays Cyrillic instead of becoming `PR`: there is no next word for the model to decide it with. `Pause` corrects it.
- `кз` in a Russian phrase (`говорит он кз домене`) may become `rp`: the frequency tables the model checks words with hold no conversational Russian, while `rp` occurs there. `Pause` brings the word back.
- A Russian abbreviation in capitals typed in the English layout amid a Russian phrase (`DKRCV` for `ВЛКСМ`) stays Latin: with the evidence at hand it cannot be told from a Latin abbreviation (`по версии WBC`). `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

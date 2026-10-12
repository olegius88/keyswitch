# KeySwitch 0.53.0

## Русский

Выпуск укорачивает паузу, после которой исправляется слово, и чинит несколько случаев на паузе и в VS Code.
Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.53.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.53.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.53.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.53.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.53.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.53.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.53.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что изменилось

- Исправление на паузе срабатывает через 0,7 секунды без набора вместо 1,5. Слово в чужой раскладке, начало которого
  ранняя смена ещё не может решить, раньше переключалось только после полуторасекундной паузы посреди слова. Пауза
  внутри слова, которое ещё может стать словом своей раскладки, по-прежнему ничего не исправляет. Если в настройках
  стояло прежнее значение по умолчанию, оно один раз заменится новым; выбранная вручную пауза сохраняется.
- Знак в конце слова, набранный клавишей знака другой раскладки, отделяется от слова и допечатывается после
  исправления — и на пробеле, и на паузе: `текст.`, набранное в английской раскладке как `ntrcn/`, становится
  `текст.`, а `What.`, набранное в русской раскладке одним сообщением, — `What.`. Раньше слово решалось вместе со
  знаком и оставалось как набрано.
- Слово, решённое на паузе после ожидания соседа, исправляется сразу. Раньше исправление ждало следующей клавиши и
  перепечатывало её в другой раскладке: из `вход` получалось `dход`.
- Слово, переведённое на паузе, пересматривается вместе со следующим словом, как переведённое на пробеле: если
  следующее слово вернулось в исходную раскладку, первое тоже может вернуться.
- В VS Code однобуквенное слово (`b` вместо «и», `d` вместо «в») больше не отклоняется как набранное в изменившееся
  поле.
- Слово, набранное перед текстом, который уже стоит в поле, решается так же, как набранное в конце. Раньше английский
  текст справа от курсора (например, вставленный в чат VS Code) сбивал модель: русское «мне» становилось `vyt`, «нет» —
  `ytn`, а `vyt`, набранное вместо «мне», оставалось как есть.

### Модель

- Контекстная модель `context-v3-d26f82e59d8b` (корпус v52) закрепляет новый движок; модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`. На запечатанном test v52 правильный текст сохранён во всех 193 строках, а без ранней
  смены восстановлено 189 строк из 192, набранных не в той раскладке, против 188 у базовой пары.

### Что осталось

- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по
  времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не создал ЗК?`): на
  знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- `КЗ` в русском тексте (`даже в КЗ ходит`) становится `RP`: этой аббревиатуры нет в таблице терминов, которой
  модель проверяет русские аббревиатуры. Вернуть слово можно клавишей `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с
  такими же правами (ограничение Windows).

## English

This release shortens the pause after which a word is corrected and fixes several cases at a pause and in VS Code.
The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.53.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.53.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.53.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.53.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.53.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.53.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.53.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Changed

- The pause correction comes after 0.7 s of no typing instead of 1.5 s. A word typed in the wrong layout whose start
  the early switch cannot decide yet used to switch only after a pause of a second and a half in the middle of the
  word. A pause inside a word that may still become a word of its layout corrects nothing, as before. A settings file
  with the old default takes the new one once; a pause chosen in the settings is kept.
- A sign at the end of a word typed with the other layout's sign key is split off and replayed after the correction,
  at a space and at a pause: `текст.` typed in the English layout as `ntrcn/` becomes `текст.`, and `What.` typed in
  the Russian layout as a one-word message becomes `What.`. The word used to be judged with its sign and stayed as
  typed.
- A word decided at a pause after waiting for its neighbour is corrected at once. The correction used to wait for the
  next key and type it again in the other layout: `вход` came out as `dход`.
- A word converted at a pause is reconsidered together with the next word, as one converted at a space is: when the
  next word goes back to the first word's layout, the first may go back too.
- In VS Code a one-letter word (`b` for `и`, `d` for `в`) is no longer refused as typed into a changed field.
- A word typed in front of text already in the field is decided as one typed at its end. English text right of the
  caret (pasted into a VS Code chat box, for instance) used to mislead the model: Russian `мне` became `vyt`, `нет`
  `ytn`, and `vyt` typed for `мне` stayed as it was.

### Model

- The context model `context-v3-d26f82e59d8b` (corpus v52) seals the new engine; the prefix model is still
  `prefix-v2-bf3dc28f8567`. On the sealed test v52 it kept all 193 correctly typed rows and, without the early
  switch, restored 189 of the 192 typed in the wrong layout against 188 for the baseline pair.

### What remains

- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the
  press cannot tell an undo of a wrong correction from a press out of habit.
- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- `КЗ` in Russian text (`даже в КЗ ходит`) becomes `RP`: the abbreviation is missing from the term table the model
  checks Russian abbreviations with. `Pause` brings the word back.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights
  (a Windows restriction).

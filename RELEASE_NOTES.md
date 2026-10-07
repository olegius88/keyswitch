# KeySwitch 0.41.0

## Русский

Короткий термин из двух букв, отправленный отдельным сообщением, теперь исправляется: `гш` →
`ui`, `зщ` → `po`, `фш` → `ai`, `еп` → `tg`. Первое слово сообщения модель решает с учётом того,
как часто каждое прочтение встречается в русском и английском тексте. Автоматическое исправление
снова работает в поле, которое сообщает курсор на символ-два раньше набранного слова (чат VS Code
Insiders). Всё остальное модель решает так же, как 0.40.0: её веса не тронуты, поверх них обучены
две отдельные части. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.41.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.41.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.41.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.41.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.41.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.41.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.41.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Модель

- Контекстная модель `context-v3-1a1e595a0dd3`: все веса модели 0.40.0
  (`context-v3-4f77286aafe8`) и две новые головы; модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`.
- По журналам набора она переводит верно 720 слов, пропускает 110 и ошибочно переводит 68;
  у 0.40.0 — 716, 119 и 66. На словах, раскладку которых показала собственная правка
  пользователя, — 115, 35 и 8 против 110, 40 и 8.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 193 строк,
  без ранней смены восстановлено 173 строки против 160 у эталонной пары.

### Что исправлено

- Одиночное слово из двух букв, которое закончено Enter, Tab, знаком или паузой, исправляется,
  если частоты его прочтений однозначны: `гш` → `ui`, `зщ` → `po`, `фш` → `ai`, `еп` → `tg`,
  `щд` → `ol`, `цф` → `wa`. После пробела оно, как и раньше, ждёт следующего слова, а `ye`,
  отправленное отдельно, по-прежнему становится `ну`.
- Первое слово сообщения: редкие русские слова реже уходят в латиницу, а слова, набранные в
  чужой раскладке, чаще исправляются. На словах проверочной выборки, поставленных первыми в
  сообщение, — 16 ошибочных переводов вместо 26 и 7 415 исправленных вместо 7 041.
- В поле, которое сообщает курсор на один-два символа раньше конца набранного слова (так делал
  чат VS Code Insiders 06.10.2026), автоматическое исправление снова работает: раньше каждое
  слово там отклонялось как набранное в изменившееся поле. Поле с другим текстом по-прежнему
  отклоняется, а технический журнал записывает, где стояло слово относительно курсора, — только
  числа.

### Что осталось

- Редкое русское слово, которого нет в словаре, в начале сообщения может уйти в латиницу; его
  возвращает отмена или клавиша `Pause`.
- По журналам появились две новые ошибки и один новый пропуск: `тт` и `ищеа`, отправленные
  отдельно, переводятся (`nn`, `botf`), а обрывок `jxbc` остаётся вместо `очис`.
- Инициал в начале сообщения переводится: `J. J. Thomson` становится `О. Ою Thomson`, как и в
  0.38–0.40.
- Редкое русское сокращение заглавными, набранное в английской раскладке, чаще остаётся
  латиницей: его клавиши выглядят как латинская аббревиатура. Его исправляет клавиша `Pause`.
- Три случая прошлых выпусков решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.

## English

A short term of two letters sent as a message of its own is now corrected: `гш` → `ui`, `зщ` →
`po`, `фш` → `ai`, `еп` → `tg`. The first word of a message is decided with how often each of its
readings occurs in Russian and English text. Automatic correction works again in a field that
reports its caret a character or two short of the typed word (the VS Code Insiders chat).
Everything else is decided as in 0.40.0: its weights are untouched, and two separate parts are
trained on top of them. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.41.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.41.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.41.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.41.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.41.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.41.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.41.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Model

- The context model `context-v3-1a1e595a0dd3`: every weight of the 0.40.0 model
  (`context-v3-4f77286aafe8`) and two new heads; the prefix model stays
  `prefix-v2-bf3dc28f8567`.
- On the typing logs it converts 720 words right, misses 110 and converts 68 falsely; 0.40.0 made
  716, 119 and 66. On the words whose layout the user's own correction shows: 115, 35 and 8
  against 110, 40 and 8.
- On the new sealed check it corrupted none of 193 correctly typed rows and, without the early
  switch, restored 173 rows against 160 for the reference pair.

### Fixed

- A lone word of two letters ended by Enter, Tab, a sign or a pause is corrected when the counts
  of its readings settle it: `гш` → `ui`, `зщ` → `po`, `фш` → `ai`, `еп` → `tg`, `щд` → `ol`,
  `цф` → `wa`. After a space it still waits for the next word, and `ye` sent alone still becomes
  `ну`.
- The first word of a message: rare Russian words turn Latin less often, and words typed in the
  wrong layout are corrected more often. On the words of a held-out set put first in a message:
  16 false conversions instead of 26, and 7 415 corrected instead of 7 041.
- In a field that reports its caret one or two characters short of the end of the typed word (the
  VS Code Insiders chat did so on 06.10.2026) automatic correction works again: every word there
  was refused as typed into a changed field. A field holding other text is still refused, and the
  technical log records where the word stood against the caret, as numbers only.

### What remains

- A rare Russian word the dictionary does not know may still turn Latin at the start of a
  message; undo or `Pause` brings it back.
- The typing logs show two new false conversions and one new miss: `тт` and `ищеа` sent alone are
  converted (`nn`, `botf`), and the fragment `jxbc` stays instead of `очис`.
- An initial at the start of a message is converted: `J. J. Thomson` becomes `О. Ою Thomson`, as
  in 0.38–0.40.
- A rare Russian abbreviation in capitals typed in the English layout stays Latin more often: its
  keys look like a Latin abbreviation. `Pause` corrects it.
- Three cases of earlier releases are decided as before, and no text is corrupted: `jr.` before a
  Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for the next
  word, and `b/bkb` stays as typed.

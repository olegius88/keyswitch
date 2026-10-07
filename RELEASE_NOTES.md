# KeySwitch 0.42.0

## Русский

Русское слово из одной буквы, набранное в английской раскладке сразу после латинского термина,
теперь исправляется: `nats b redis` → `nats и redis`, `docker b kubernetes` → `docker и
kubernetes`, `lid f номер` → `lid а номер`. Английский текст с одиночной буквой (`plan b then`,
`option c is`, `vitamin c and`) остаётся как набран. Всё остальное модель решает так же, как
0.41.0: её веса не тронуты, поверх них обучена одна отдельная часть. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.42.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.42.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.42.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.42.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.42.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.42.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.42.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Модель

- Контекстная модель `context-v3-fb87dd994818`: все веса модели 0.41.0
  (`context-v3-1a1e595a0dd3`) и новая голова для слова из одной буквы после латинского слова;
  модель префиксов прежняя, `prefix-v2-bf3dc28f8567`.
- По журналам набора она переводит верно 726 слов, пропускает 104 и ошибочно переводит 68;
  у 0.41.0 — 720, 110 и 68. Новых ошибок нет. На словах, раскладку которых показала собственная
  правка пользователя, — 115, 35 и 8, как у 0.41.0.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 201 строки,
  без ранней смены восстановлено 180 строк против 159 у эталонной пары.

### Что исправлено

- Русское слово из одной буквы (а, и, в, с, к, у, о, я), набранное в английской раскладке сразу
  после латинского слова, исправляется вместе со следующим словом: если следующее слово
  переведено в русскую раскладку (`lid f номер` → `lid а номер`) или осталось латинским термином
  (`nats b redis` → `nats и redis`). Модель учитывает, насколько слова по обе стороны похожи на
  технические термины русского текста или на английскую прозу, поэтому `plan b then` и
  `option c is` остаются.

### Что осталось

- Одиночная буква в конце сообщения после латинского слова (`todo b`) остаётся как набрана:
  без следующего слова модели не хватает уверенности. Её исправляет клавиша `Pause`.
- Буква после знаков (`-- f разве`) и после русского слова решается прежними правилами.
- Редкое русское слово, которого нет в словаре, в начале сообщения может уйти в латиницу; его
  возвращает отмена или клавиша `Pause`.
- `тт` и `ищеа`, отправленные отдельно, переводятся (`nn`, `botf`), а обрывок `jxbc` остаётся
  вместо `очис`, как в 0.41.0.
- Инициал в начале сообщения переводится: `J. J. Thomson` становится `О. Ою Thomson`, как и в
  0.38–0.41.
- Редкое русское сокращение заглавными, набранное в английской раскладке, чаще остаётся
  латиницей: его клавиши выглядят как латинская аббревиатура. Его исправляет клавиша `Pause`.
- Три случая прошлых выпусков решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.

## English

A Russian word of one letter typed in the English layout right after a Latin term is now
corrected: `nats b redis` → `nats и redis`, `docker b kubernetes` → `docker и kubernetes`,
`lid f номер` → `lid а номер`. English text with a lone letter (`plan b then`, `option c is`,
`vitamin c and`) stays as typed. Everything else is decided as in 0.41.0: its weights are
untouched, and one separate part is trained on top of them. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.42.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.42.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.42.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.42.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.42.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.42.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.42.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Model

- The context model `context-v3-fb87dd994818`: every weight of the 0.41.0 model
  (`context-v3-1a1e595a0dd3`) and a new head for a word of one letter after a Latin word; the
  prefix model stays `prefix-v2-bf3dc28f8567`.
- On the typing logs it converts 726 words right, misses 104 and converts 68 falsely; 0.41.0 made
  720, 110 and 68. There is no new error. On the words whose layout the user's own correction
  shows: 115, 35 and 8, as 0.41.0.
- On the new sealed check it corrupted none of 201 correctly typed rows and, without the early
  switch, restored 180 rows against 159 for the reference pair.

### Fixed

- A Russian word of one letter (а, и, в, с, к, у, о, я) typed in the English layout right after
  a Latin word is corrected together with the next word: when that word is converted to the
  Russian layout (`lid f номер` → `lid а номер`) or stays a Latin term (`nats b redis` →
  `nats и redis`). The model weighs how much the words on either side look like the technical
  terms of Russian text or like English prose, so `plan b then` and `option c is` stay.

### What remains

- A lone letter at the end of a message after a Latin word (`todo b`) stays as typed: without
  a next word the model is not sure enough. `Pause` corrects it.
- A letter after signs (`-- f разве`) or after a Russian word is decided by the earlier rules.
- A rare Russian word the dictionary does not know may still turn Latin at the start of a
  message; undo or `Pause` brings it back.
- `тт` and `ищеа` sent alone are converted (`nn`, `botf`), and the fragment `jxbc` stays instead
  of `очис`, as in 0.41.0.
- An initial at the start of a message is converted: `J. J. Thomson` becomes `О. Ою Thomson`, as
  in 0.38–0.41.
- A rare Russian abbreviation in capitals typed in the English layout stays Latin more often: its
  keys look like a Latin abbreviation. `Pause` corrects it.
- Three cases of earlier releases are decided as before, and no text is corrupted: `jr.` before a
  Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for the next
  word, and `b/bkb` stays as typed.

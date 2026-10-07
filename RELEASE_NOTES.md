# KeySwitch 0.43.0

## Русский

Русская аббревиатура заглавными после русского текста больше не превращается в латинские буквы
своих клавиш: `Страны ЕС приняли` остаётся (было `Страны TC приняли`), как и `Письмо из ФНС`
(было `из AYC`), `Поздравляю с НГ` (было `с YU`) и `Заказал в ТЦ`. Английское сообщение,
которое начинается с инициала, перечня или имени, тоже остаётся как набрано: `J. Smith said`,
`C, D and E`, `C# rocks`, `J. J. Thomson` раньше превращались в `О. Smith said`, `С, В and E`,
`С№ rocks`, `О. Ою Thomson`. Всё остальное модель решает так же, как 0.42.0. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.43.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.43.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.43.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.43.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.43.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.43.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.43.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Модель

- Контекстная модель `context-v3-55d3df05fe3a`: все веса модели 0.41.0, голова одиночной буквы,
  заново обученная на свежем корпусе вместе с исправленным правилом начала сообщения, и новая
  голова для аббревиатур; модель префиксов прежняя, `prefix-v2-bf3dc28f8567`.
- По журналам набора она переводит верно 726 слов, пропускает 104 и ошибочно переводит 68, как
  0.42.0; на словах, раскладку которых показала собственная правка пользователя, — 115, 35 и 8.
  Новых ошибок нет.
- Из 60 русских аббревиатур в русских предложениях она сохраняет 55 (0.42.0 — 50) и, как
  прежде, исправляет 26 из 30 латинских, набранных в русской раскладке.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 214 строк,
  без ранней смены восстановлено 186 строк против 171 у эталонной пары.

### Что исправлено

- Русская аббревиатура заглавными (`ЕС`, `ФНС`, `НГ`, `ТЦ`, `ТЗ`) после русского текста
  остаётся как набрана. Модель сравнивает, как часто встречается каждое прочтение в русском и
  техническом тексте, поэтому латинская аббревиатура, набранная в русской раскладке, по-прежнему
  исправляется (`Открой ФЗШ` → `Открой API`).
- Английское сообщение, которое начинается с инициала, перечня или имени (`J. Smith said`,
  `C, D and E`, `C# rocks`, `D. Knuth wrote it`, `J. J. Thomson`), остаётся как набрано.
  Русское сообщение, набранное в английской раскладке, исправляется как прежде:
  `f? gjyznyj` → `а, понятно`, `z ctujlyz` → `я сегодня`.

### Что осталось

- Сокращения, которых нет в текстах, по которым считались частоты, решаются по-прежнему:
  `в КЗ` становится `в RP`, `ГД` — `UL`, `ФСС` — `ACC`. Их возвращает отмена или клавиша
  `Pause`. Латинские `LLM`, `HR`, `KPI` и `NDA`, набранные в русской раскладке, остаются
  кириллицей, как в 0.42.0.
- Одиночная буква в конце сообщения после латинского слова (`todo b`) остаётся как набрана:
  без следующего слова модели не хватает уверенности. Её исправляет клавиша `Pause`.
- Редкое русское слово, которого нет в словаре, в начале сообщения может уйти в латиницу; его
  возвращает отмена или клавиша `Pause`.
- `тт` и `ищеа`, отправленные отдельно, переводятся (`nn`, `botf`), а обрывок `jxbc` остаётся
  вместо `очис`, как в 0.42.0.
- Редкое русское сокращение заглавными, набранное в английской раскладке, чаще остаётся
  латиницей: его клавиши выглядят как латинская аббревиатура. Его исправляет клавиша `Pause`.
- Три случая прошлых выпусков решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.

## English

A Russian abbreviation in capitals after Russian text no longer turns into the Latin letters of
its keys: `Страны ЕС приняли` stays (it became `Страны TC приняли`), and so do `Письмо из ФНС`
(it became `из AYC`), `Поздравляю с НГ` (it became `с YU`) and `Заказал в ТЦ`. An English
message that opens with an initial, a list or a name stays as typed too: `J. Smith said`,
`C, D and E`, `C# rocks` and `J. J. Thomson` became `О. Smith said`, `С, В and E`, `С№ rocks`
and `О. Ою Thomson`. Everything else is decided as in 0.42.0. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.43.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.43.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.43.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.43.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.43.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.43.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.43.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Model

- The context model `context-v3-55d3df05fe3a`: every weight of the 0.41.0 model, the
  single-letter head trained again on a fresh corpus with the corrected message-start rule, and a
  new head for abbreviations; the prefix model stays `prefix-v2-bf3dc28f8567`.
- On the typing logs it converts 726 words right, misses 104 and converts 68 falsely, as 0.42.0;
  on the words whose layout the user's own correction shows: 115, 35 and 8. There is no new
  error.
- Of 60 Russian abbreviations in Russian sentences it keeps 55 (0.42.0: 50) and, as before,
  restores 26 of 30 Latin ones typed in the Russian layout.
- On the new sealed check it corrupted none of 214 correctly typed rows and, without the early
  switch, restored 186 rows against 171 for the reference pair.

### Fixed

- A Russian abbreviation in capitals (`ЕС`, `ФНС`, `НГ`, `ТЦ`, `ТЗ`) after Russian text stays as
  typed. The model compares how often each reading occurs in Russian and technical text, so a
  Latin abbreviation typed in the Russian layout is still corrected (`Открой ФЗШ` →
  `Открой API`).
- An English message that opens with an initial, a list or a name (`J. Smith said`,
  `C, D and E`, `C# rocks`, `D. Knuth wrote it`, `J. J. Thomson`) stays as typed. A Russian
  message typed in the English layout is corrected as before: `f? gjyznyj` → `а, понятно`,
  `z ctujlyz` → `я сегодня`.

### What remains

- Abbreviations missing from the texts the frequencies were counted on are decided as before:
  `в КЗ` becomes `в RP`, `ГД` becomes `UL`, `ФСС` becomes `ACC`. Undo or `Pause` brings them
  back. The Latin `LLM`, `HR`, `KPI` and `NDA` typed in the Russian layout stay Cyrillic, as in
  0.42.0.
- A lone letter at the end of a message after a Latin word (`todo b`) stays as typed: without
  a next word the model is not sure enough. `Pause` corrects it.
- A rare Russian word the dictionary does not know may still turn Latin at the start of a
  message; undo or `Pause` brings it back.
- `тт` and `ищеа` sent alone are converted (`nn`, `botf`), and the fragment `jxbc` stays instead
  of `очис`, as in 0.42.0.
- A rare Russian abbreviation in capitals typed in the English layout stays Latin more often: its
  keys look like a Latin abbreviation. `Pause` corrects it.
- Three cases of earlier releases are decided as before, and no text is corrupted: `jr.` before a
  Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for the next
  word, and `b/bkb` stays as typed.

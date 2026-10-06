# KeySwitch 0.40.0

## Русский

Короткий термин, набранный в русской раскладке посреди русского текста, теперь исправляется,
даже если следующее слово набрано верно: `есть новые зк проверь` → `есть новые pr проверь`.
Латинская аббревиатура заглавными в английском тексте остаётся латиницей (`the BP oil spill`
больше не становится `ИЗ`). Всё остальное модель решает так же, как 0.39.0: её веса не тронуты,
поверх них обучены две отдельные части — для таких терминов и для заглавных после латиницы.
Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.40.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.40.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.40.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.40.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.40.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.40.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.40.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Модель

- Контекстная модель `context-v3-4f77286aafe8`: все веса модели 0.39.0
  (`context-v3-984962d926d3`) и две новые головы; модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`.
- По журналам набора она переводит верно 716 слов, пропускает 119 и ошибочно переводит 66;
  у 0.39.0 — 707, 128 и 64. Два новых «ошибочных» перевода на деле верны: `сым` → `csv`,
  `йуьг` → `qemu`. На словах, раскладку которых показала собственная правка пользователя, —
  110, 40 и 8 против 108, 42 и 8.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 201 строки
  (у эталонной пары — в одной), без ранней смены восстановлено 175 строк против 157.

### Что исправлено

- Короткое слово, которое ждёт следующего, теперь спрашивается ещё раз, если следующее слово
  набрано верно: термин в русской раскладке посреди русского текста исправляется один (`зк` →
  `pr`, `тзь` → `npm`, `фзл` → `apk`, `вум` → `dev`, `ьв` → `md`), следующее слово остаётся как
  было, а отмена возвращает оба.
- Латинская аббревиатура заглавными посреди английского текста остаётся латиницей, даже если
  её клавиши в русской раскладке дают частое русское слово: `affected by the BP oil spill`
  больше не превращает `BP` в `ИЗ`, `Lakers VS` — в `МЫ`.

### Что осталось

- Одиночный короткий термин без соседних слов, набранный в русской раскладке, остаётся как набран:
  `гш` вместо `ui`, `зщ` вместо `po`, `фш` вместо `ai`. Это самая большая группа оставшихся
  пропусков; его исправляет клавиша `Pause`.
- Инициал в начале сообщения переводится: `J. J. Thomson` становится `О. Ою Thomson`, как и в
  0.38 и 0.39.
- Редкое русское сокращение заглавными, набранное в английской раскладке, чаще остаётся
  латиницей: его клавиши выглядят как латинская аббревиатура. Его исправляет клавиша `Pause`.
- Три случая прошлых выпусков решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.

## English

A short term typed in the Russian layout amid Russian text is now corrected even when the next
word is typed right: `есть новые зк проверь` → `есть новые pr проверь`. A Latin abbreviation in
capitals inside English text stays Latin (`the BP oil spill` no longer becomes `ИЗ`). Everything
else is decided as in 0.39.0: its weights are untouched, and two separate parts are trained on
top of them, one for such terms and one for capitals after Latin text. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.40.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.40.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.40.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.40.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.40.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.40.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.40.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Model

- The context model `context-v3-4f77286aafe8`: every weight of the 0.39.0 model
  (`context-v3-984962d926d3`) and two new heads; the prefix model stays
  `prefix-v2-bf3dc28f8567`.
- On the typing logs it converts 716 words right, misses 119 and converts 66 falsely; 0.39.0 made
  707, 128 and 64. The two new "false" conversions are right: `сым` → `csv`, `йуьг` → `qemu`. On
  the words whose layout the user's own correction shows: 110, 40 and 8 against 108, 42 and 8.
- On the new sealed check it corrupted none of 201 correctly typed rows (the reference pair
  corrupted one) and, without the early switch, restored 175 rows against 157.

### Fixed

- A short word that waits for the next one is asked again when the next word is typed right: a
  term typed in the Russian layout amid Russian text is corrected alone (`зк` → `pr`, `тзь` →
  `npm`, `фзл` → `apk`, `вум` → `dev`, `ьв` → `md`), the next word stays as it was, and undo
  restores both.
- A Latin abbreviation in capitals inside English text stays Latin even when its keys in the
  Russian layout spell a common Russian word: `affected by the BP oil spill` no longer turns `BP`
  into `ИЗ`, nor `Lakers VS` into `МЫ`.

### What remains

- A lone short term with no words around it, typed in the Russian layout, stays as typed: `гш`
  for `ui`, `зщ` for `po`, `фш` for `ai`. This is the largest group of the remaining misses;
  `Pause` corrects it.
- An initial at the start of a message is converted: `J. J. Thomson` becomes `О. Ою Thomson`, as
  in 0.38 and 0.39.
- A rare Russian abbreviation in capitals typed in the English layout stays Latin more often: its
  keys look like a Latin abbreviation. `Pause` corrects it.
- Three cases of earlier releases are decided as before, and no text is corrupted: `jr.` before a
  Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for the next
  word, and `b/bkb` stays as typed.

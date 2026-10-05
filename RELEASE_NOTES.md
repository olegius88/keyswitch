# KeySwitch 0.39.0

## Русский

Новая контекстная модель, обученная по журналам реального набора и проверенная на новой
запечатанной выборке. На словах, которые в журналах исправляли вручную, она ошибается реже 0.38:
верно переведено 97 таких слов против 90, пропущено 34 против 41, ложных исправлений столько же.
Латинские аббревиатуры в русском тексте остаются латиницей, домены и адреса, набранные в другой
раскладке, восстанавливаются, а `ghbdtn@` снова становится `привет"`. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.39.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.39.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.39.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.39.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.39.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.39.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.39.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Модель

- Новая контекстная модель `context-v3-984962d926d3`; модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`.
- По журналам набора она переводит верно 606 слов, пропускает 114 и ошибочно переводит 57;
  у 0.38 — 613, 113 и 63. На словах, раскладку которых показала собственная правка
  пользователя, — 97, 34 и 7 против 90, 41 и 7.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 209 строк
  (у эталонной пары — в одной), без ранней смены восстановлено 188 строк против 177.

### Что исправлено

- Латинская аббревиатура заглавными в русском тексте остаётся латиницей, даже если её
  кириллическое прочтение случайно оказалось редким русским словом: `по версии WBC` больше не
  становится `ЦИС`. Аббревиатуры, которые русский текст пишет постоянно (`США`, `ВДВ`), набранные
  в английской раскладке, по-прежнему исправляются, а свои сокращения пользователя (`ГА`)
  остаются.
- Домен, адрес или имя файла, набранные в другой раскладке, восстанавливаются: `учфьздуюсщь` →
  `example.com`. Раньше слово с точкой внутри не исправлялось, даже когда модель была уверена.
- `ghbdtn@` снова становится `привет"`: русская закрывающая кавычка — это клавиша `@`.
- Если английская фраза целиком набрана в русской раскладке и первое слово случайно оказалось
  русским словом, следующее слово всё равно исправляется: `руку ерун` → `here they`.

### Что осталось

- Инициал в начале сообщения переводится: `J. J. Thomson` становится `О. Ою Thomson`. Так было
  и в 0.38; правило для однобуквенных слов в начале сообщения не видит точку после буквы.
- Редкое русское сокращение заглавными, набранное в английской раскладке, теперь чаще остаётся
  латиницей: его клавиши выглядят как латинская аббревиатура. Его исправляет клавиша `Pause`.
- Три случая прошлых выпусков решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.

## English

A new context model, trained on the logs of real typing and checked on a new sealed sample. On
the words that were corrected by hand in the logs it errs less than 0.38: 97 such words converted
right against 90 and 34 missed against 41, with as many false corrections. Latin abbreviations in Russian text
stay Latin, domains and addresses typed in the other layout are restored, and `ghbdtn@` becomes
`привет"` again. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.39.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.39.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.39.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.39.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.39.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.39.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.39.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Model

- A new context model, `context-v3-984962d926d3`; the prefix model stays
  `prefix-v2-bf3dc28f8567`.
- On the typing logs it converts 606 words right, misses 114 and converts 57 falsely; 0.38 made
  613, 113 and 63. On the words whose layout the user's own correction shows: 97, 34 and 7
  against 90, 41 and 7.
- On the new sealed check it corrupted none of 209 correctly typed rows (the reference pair
  corrupted one) and, without the early switch, restored 188 rows against 177.

### Fixed

- A Latin abbreviation in capitals inside Russian text stays Latin even when its Cyrillic
  reading happens to be a rare Russian word: `по версии WBC` no longer becomes `ЦИС`.
  Abbreviations Russian text writes all the time (`США`, `ВДВ`), typed in the English layout,
  are still corrected, and a user's own abbreviations (`ГА`) stay.
- A domain, an address or a file name typed in the other layout is restored: `учфьздуюсщь` →
  `example.com`. A word with a dot inside used to stay as typed even when the model was sure.
- `ghbdtn@` becomes `привет"` again: the Russian closing quote is the `@` key.
- When an English phrase is typed whole in the Russian layout and its first word happens to be a
  Russian word, the next word is still corrected: `руку ерун` → `here they`.

### What remains

- An initial at the start of a message is converted: `J. J. Thomson` becomes `О. Ою Thomson`.
  0.38 did the same; the rule for one-letter words at the start of a message does not see the
  period after the letter.
- A rare Russian abbreviation in capitals typed in the English layout now stays Latin more
  often: its keys look like a Latin abbreviation. `Pause` corrects it.
- Three cases of earlier releases are decided as before, and no text is corrupted: `jr.` before a
  Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for the next
  word, and `b/bkb` stays as typed.

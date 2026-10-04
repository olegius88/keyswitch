# KeySwitch 0.38.0

## Русский

Этот выпуск сделан по журналам реального набора: около 17 тысяч слов на двух компьютерах,
версии 0.26–0.36.3. Они показали, что 0.37.0 оставляла слово в чужой раскладке почти вдвое
чаще, чем 0.36.3. Контекстная модель снова учитывает, как часто слово встречается в русском и
английском тексте и где оно стоит в строке. Ранняя смена раскладки срабатывает чаще и
исправляет свои ошибки. На словах, которые в журналах исправляли вручную, новая версия ошибается
реже и 0.37.0, и 0.36.3. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.38.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.38.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.38.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.38.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.38.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Ранняя смена раскладки

- Буквы б, ю, ж, х, э и ъ внутри слова больше не мешают ранней смене. В английской раскладке
  это клавиши `,` `.` `;` `[` `'` `]`, и раньше слово с такой буквой ждало пробела; в журналах
  это треть длинных русских слов, набранных в английской раскладке. Ждёт только слово, которое на
  такой клавише заканчивается: это может быть знак препинания.
- Дописанное слово проверяется целиком, в том виде, как его набрали. Если ранняя смена ошиблась
  (например, перевела в латиницу русское сленговое слово, которого нет в словаре), она
  отменяется одной правкой.
- Первые слова строки перед словом, которое перевела ранняя смена, теперь проверяются вместе с
  ним: `dct ujnjdj` становится `все готово`, а не `dct готово`. `Pause` возвращает всю фразу к
  набранным клавишам.
- На проверочном наборе ранняя смена исправляет до пробела 123–126 слов из 128, а не 112–114.

### Модели

- Новая контекстная модель `context-v3-bb8097ea5941`; модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`. Контекстная модель учитывает, как часто каждое прочтение слова
  встречается в русском техническом тексте и в тексте своего языка, где слово стоит в строке и
  начинает ли оно предложения. Английский термин, набранный в русской раскладке посреди русской
  фразы (`зк` вместо `pr`), переводится чаще, а русское сокращение остаётся русским.
- Модель обучена на новом корпусе из публичных источников и прошла новую запечатанную проверку:
  без ранней смены восстановлено 190 строк против 183 у эталона, правильно набранный текст не
  испорчен ни в одном режиме. На калибровочной выборке ложных исправлений примерно вдвое меньше,
  чем у модели 0.37.0.

### Короткие слова и сокращения

- Однобуквенное русское слово сразу после английского термина в русской фразе переводится:
  `поправь env b` → `поправь env и`. Заглавная буква (`plan B`) и буква после английского
  текста не трогаются.
- Одиночную букву, в том числе с цифрами (`1С`, `а1`), переводят только правила из
  проверенного списка. Модель сама по себе больше не превращает `ч` в `x`.
- Русские сокращения и сленг, которые часто встречаются в русском тексте, тогда как их латинские
  клавиши не встречаются нигде (`пдф`, `впн`), остаются как набраны.
- `ghbdtn@` снова становится `привет"`.

### Что осталось

- По журналам эта версия всё ещё пропускает больше слов в чужой раскладке, чем 0.36.3, хотя
  ложных исправлений у неё меньше. Слово, в котором модель не уверена, она оставляет как
  набрано, и его исправляет клавиша `Pause`.
- Три случая из прошлого выпуска решаются по-прежнему, текст при этом не портится: `jr.` перед
  русским словом в начале строки остаётся `jr.`, `ша` после русского слова ждёт следующего
  слова, `b/bkb` остаётся как набрано.
- `2фа` по-прежнему становится `2af`.
- В терминале после приглашения `$` команда, похожая на русское слово в английской раскладке,
  иногда переводится. На проверочном наборе таких слов на одно больше, чем у 0.37.0: 20 из 160.

## English

This release follows the logs of real typing: about 17 thousand words on two computers,
versions 0.26–0.36.3. They showed 0.37.0 leaving a word in the wrong layout almost twice as
often as 0.36.3. The context model again weighs how often a word occurs in Russian and English
text and where it stands on its line. The early layout switch acts more often and takes back
its own mistakes. On the words that were corrected by hand in the logs, the new version errs
less than both 0.37.0 and 0.36.3. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.38.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.38.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.38.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.38.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.38.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Early layout switch

- The letters б, ю, ж, х, э and ъ inside a word no longer hold the early switch back. In the
  English layout they are the keys `,` `.` `;` `[` `'` `]`, and a word with such a letter used
  to wait for the space; in the logs that was a third of the long Russian words typed in the
  English layout. Only a word that ends on such a key waits: it may be punctuation.
- A finished word is checked whole, as it was typed. If the early switch was wrong (say, it
  turned a Russian slang word no dictionary holds into Latin), it is taken back in one
  correction.
- The first words of a line before a word the early switch converted are now checked together
  with it: `dct ujnjdj` becomes `все готово`, not `dct готово`. `Pause` takes the whole phrase
  back to the keys as typed.
- On the test set the early switch corrects 123–126 of 128 words before the space, instead of
  112–114.

### Models

- A new context model, `context-v3-bb8097ea5941`; the prefix model stays
  `prefix-v2-bf3dc28f8567`. The context model weighs how often each reading of a word occurs in
  Russian technical text and in text of its own language, where the word stands on its line and
  whether it opens sentences. An English term typed in the Russian layout amid a Russian phrase
  (`зк` for `pr`) is converted more often, and a Russian abbreviation stays Russian.
- The model is trained on a new corpus built from public sources and passed a new sealed test:
  without the early switch it restored 190 rows against the reference pair's 183, and it
  corrupted no correctly typed text in either mode. On the calibration sample it makes about
  half as many false corrections as the 0.37.0 model.

### Short words and abbreviations

- A one-letter Russian word typed right after an English term in a Russian phrase is converted:
  `поправь env b` → `поправь env и`. A capital letter (`plan B`) and a letter after English
  text are left alone.
- A lone letter, digits around it included (`1С`, `а1`), is converted only by the rules of the
  reviewed list. The model on its own no longer turns `ч` into `x`.
- Russian abbreviations and slang that Russian text uses again and again while their Latin keys
  occur nowhere (`пдф`, `впн`) stay as typed.
- `ghbdtn@` becomes `привет"` again.

### What remains

- By the logs this version still misses more words typed in the wrong layout than 0.36.3, while
  it makes fewer false corrections. A word the model is unsure of stays as typed, and `Pause`
  corrects it.
- Three cases of the previous release are decided as before, and no text is corrupted: `jr.`
  before a Russian word at the start of a line stays `jr.`, `ша` after a Russian word waits for
  the next word, and `b/bkb` stays as typed.
- `2фа` still becomes `2af`.
- In a terminal, after a `$` prompt, a command that looks like a Russian word typed in the
  English layout is sometimes converted. The test set has one such word more than with 0.37.0:
  20 of 160.

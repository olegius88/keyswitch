# KeySwitch 0.46.0

## Русский

Слово, допечатанное после стирания части его Backspace, остаётся одним словом: `первую` стёртое до `перв` и
допечатанное как `первый` больше не становится `первsq`. Пауза после первых двух букв слова больше не переводит их
в другую раскладку (`иг` из `игру` становилось `bu`): слово решается, когда дописано. Выпуск включает и изменения
0.45.0, которая не публиковалась: русская аббревиатура после латинского текста остаётся как набрана
(`/designv2 ТЗ`), термин, в котором модель сомневалась, решается вместе со следующим словом
(`принимай зк и продолжай` становится `принимай pr и продолжай`), дефис между словами больше не мешает исправить
первое слово сообщения (`Ytn - ult` становится `Нет - где`). Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.46.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.46.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.46.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.46.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.46.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.46.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.46.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что исправлено

- Правка слова через Backspace. Если стереть конец слова и допечатать его, KeySwitch раньше видел только
  допечатанные буквы и мог перевести их как отдельное слово. Теперь после такого стирания он читает поле: буквы,
  допечатанные к буквам той же раскладки, остаются частью слова, а к буквам другой раскладки — решаются вместе со
  всем словом.
- Пауза посреди слова. Две буквы, с которых начинаются не меньше ста слов их языка и чьё другое прочтение не слово
  обычного текста, на паузе больше не переводятся: их решает пробел. `ша`, набранное вместо `if`, и `gj` вместо
  `по` на паузе переводятся, как прежде.
- Русская аббревиатура после латиницы (из 0.45.0). `ТЗ`, `ЛС`, `НГ` после команды или английской фразы остаются,
  `ГЫ`, набранное в русской раскладке после английского текста, по-прежнему становится `US`.
- Сомнение модели (из 0.45.0). Слово, которое модель оставляла, хотя перевод для неё не менее вероятен, чем 0.3,
  решается вместе со следующим словом.
- Знак между словами (из 0.45.0). Одиночный знак, который в обеих раскладках печатается одинаково (`-`, цифра),
  больше не разрывает цепочку слов в начале сообщения.

### Диагностика

- Счётчик задержек `input_delay` больше не принимает за опоздавшую клавишу с нулевым временем от другой программы:
  такие клавиши считаются отдельно (`unstamped_keys`) и сами по себе в журнал не пишутся. У клавиш, которые вводит
  другая программа (TeamViewer на управляемом компьютере), меряется время ответа KeySwitch (`foreign_keys`): если
  удалённый ввод «захлёбывается», журнал покажет, задерживал ли его KeySwitch.

### Модель

- Контекстная модель `context-v3-bd091e29a2be` — рецепт 0.45.0 на свежем корпусе, чтобы проверка закрепила
  изменённый движок. Модель префиксов прежняя, `prefix-v2-bf3dc28f8567`.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 182 строк (эталонная пара
  портит одну), без ранней смены восстановлено 164 строки из 180 против 154, с ранней сменой — 11, как у эталонной
  пары.
- По журналам набора, шесть батарей: 834 верных исправления, 124 пропуска, 82 ложных — каждое слово решено так же,
  как в 0.45.0.

### Что осталось

- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не создал ЗК?`): на
  знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.

## English

A word finished after part of it was erased with Backspace stays one word: `первую` cut back to `перв` and
finished as `первый` no longer becomes `первsq`. A pause after the first two letters of a word no longer converts
them (`иг` of `игру` became `bu`): the word is decided once it is finished. The release also carries the changes
of 0.45.0, which was not published: a Russian abbreviation typed after Latin text stays as typed (`/designv2 ТЗ`),
a term the model was unsure of is decided together with the next word (`принимай зк и продолжай` becomes
`принимай pr и продолжай`), and a hyphen between words no longer keeps the first word of a message from being
corrected (`Ytn - ult` becomes `Нет - где`). The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.46.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.46.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.46.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.46.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.46.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.46.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.46.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Fixed

- Editing a word with Backspace. When the end of a word was erased and typed again, KeySwitch saw only the letters
  typed again and could convert them as a word of their own. After such an erasure it now reads the field: letters
  typed onto letters of their own layout stay part of the word, and onto letters of the other layout they are
  decided with the whole word.
- A pause in the middle of a word. Two letters that at least a hundred words of their language begin with, and
  whose other reading is no word of ordinary text, are no longer converted at a pause: the space decides them. `ша`
  typed for `if` and `gj` for `по` still convert at a pause.
- A Russian abbreviation after Latin text (from 0.45.0). `ТЗ`, `ЛС`, `НГ` after a command or English prose stay,
  while `ГЫ` typed in the Russian layout after English text still becomes `US`.
- The model's doubt (from 0.45.0). A word the model kept while finding the conversion at least 0.3 likely is
  decided together with the next word.
- A sign between words (from 0.45.0). A lone sign typed the same in either layout (`-`, a digit) no longer breaks
  the chain of words at the start of a message.

### Diagnostics

- The `input_delay` counter no longer takes a key with a zero time from another program for a late one: such keys
  are counted apart (`unstamped_keys`) and alone log nothing. Keys another program injects (TeamViewer on the
  controlled computer) are timed by KeySwitch's answer (`foreign_keys`): if remote typing stalls, the log shows
  whether KeySwitch held it up.

### Model

- The context model `context-v3-bd091e29a2be` is the 0.45.0 recipe on a fresh corpus, so that the check pins the
  changed engine. The prefix model stays `prefix-v2-bf3dc28f8567`.
- On the new sealed check it corrupted none of 182 correctly typed rows (the reference pair corrupts one) and,
  without the early switch, restored 164 of 180 rows against 154; with the early switch 11, as the reference pair.
- On the typing logs, six batteries: 834 right corrections, 124 missed, 82 false — every word decided as by 0.45.0.

### What remains

- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.

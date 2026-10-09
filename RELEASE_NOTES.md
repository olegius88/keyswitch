# KeySwitch 0.47.0

## Русский

Слово, набранное в неверной раскладке после стирания, снова исправляется сразу. Если поле показывало следующую
букву раньше, чем сдвигало каретку, KeySwitch принимал её за букву соседнего слова и считал, что слово набирается
внутри текста: ранняя смена раскладки ждала, а на пробеле слово иногда оставалось как набрано (`ltkfq` вместо
`делай` в приложении Claude сработало только с третьего раза, `офыы` вместо `jass` в VS Code). Полный перечень —
в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.47.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.47.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.47.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.47.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.47.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.47.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.47.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что исправлено

- Слово, ошибочно принятое за набранное внутри текста. Когда слово начинается после щелчка или стирания,
  KeySwitch читает поле, чтобы понять, не дописывается ли существующее слово. Редактор, который показывает клавишу
  раньше, чем сдвигает каретку, ставил следующую букву самого слова после каретки. За месяц журналов так прочитано
  188 слов, и у 121 из них никакого слова вокруг не было: ранняя смена ждала 55 раз, а 22 слова остались как
  набраны. Теперь клавиши, нажатые следом, считаются набранными, а перед ранней сменой, пробелом и паузой поле
  читается ещё раз: если вокруг слова нет букв, оно решается как обычное слово. Слова, которые действительно
  дописываются внутри текста (`сд|лать`), решаются, как прежде.

### Модель

- Контекстная модель `context-v3-158f1f70aa6e` — рецепт 0.46.0 на свежем корпусе, чтобы проверка закрепила
  изменённый движок. Модель префиксов прежняя, `prefix-v2-bf3dc28f8567`.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 196 строк (эталонная пара
  портит две), без ранней смены восстановлено 174 строки из 196 против 159, с ранней сменой — 9, как у эталонной
  пары.
- По журналам набора, семь батарей: 839 верных исправлений, 124 пропуска, 82 ложных — каждое слово решено так же,
  как в 0.46.0.

### Что осталось

- `ЗК` заглавными перед знаком вопроса после русского текста остаётся кириллицей (`почему ты не создал ЗК?`): на
  знаке препинания слово не ждёт следующего. Его исправляет клавиша `Pause`.
- `КЗ` в русском тексте (`даже в КЗ ходит`) становится `RP`: этой аббревиатуры нет в таблице терминов, которой
  модель проверяет русские аббревиатуры. Вернуть слово можно клавишей `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.

## English

A word typed in the wrong layout after an erasure is corrected at once again. When a field showed the next letter
before moving its caret, KeySwitch took that letter for a letter of a neighbouring word and treated the word as
typed inside text: the early layout switch waited, and at the space the word was sometimes left as typed (`ltkfq`
for `делай` in the Claude app converted only on the third try, `офыы` for `jass` in VS Code). The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.47.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.47.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.47.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.47.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.47.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.47.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.47.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Fixed

- A word taken by mistake for one typed inside text. When a word begins after a click or an erasure, KeySwitch
  reads the field to see whether an existing word is being finished. An editor that shows a key before moving its
  caret put the word's own next letter after the caret. Over a month of logs 188 words were read so, and 121 of
  them had no word around them: the early switch waited 55 times and 22 words were left as typed. The keys pressed
  next now count as typed, and before the early switch, the space and the pause the field is read again: with no
  letters around the word, it is decided as an ordinary word. Words really finished inside text (`сд|лать`) are
  decided as before.

### Model

- The context model `context-v3-158f1f70aa6e` is the 0.46.0 recipe on a fresh corpus, so that the check pins the
  changed engine. The prefix model stays `prefix-v2-bf3dc28f8567`.
- On the new sealed check it corrupted none of 196 correctly typed rows (the reference pair corrupts two) and,
  without the early switch, restored 174 of 196 rows against 159; with the early switch 9, as the reference pair.
- On the typing logs, seven batteries: 839 right corrections, 124 missed, 82 false — every word decided as by 0.46.0.

### What remains

- `ЗК` in capitals before a question mark after Russian text stays Cyrillic (`почему ты не создал
  ЗК?`): at a punctuation mark a word does not wait for the next one. `Pause` corrects it.
- `КЗ` in Russian text (`даже в КЗ ходит`) becomes `RP`: the abbreviation is missing from the term table the model
  checks Russian abbreviations with. `Pause` brings the word back.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.

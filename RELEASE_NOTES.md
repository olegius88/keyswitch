# KeySwitch 0.25.0

## Русский

Выпуск исправляет то, из-за чего короткие слова и имена команд путались между раскладками,
и добавляет правило для случая, когда курсор переносят в уже написанный текст. Модели те же,
что в 0.24.0. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.25.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.25.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.25.0-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Модели те же, кандидат остаётся кандидатом

Пара context-v1 + prefix-v1 остаётся установленной. Кандидат новой схемы прошёл запечатанную
оценку — 512 строк из предложений Tatoeba и индекса пакетов Fedora Rawhide, которых не видела
ни одна прежняя оценка: он восстанавливает 228 неверно набранных строк из 252 против 210 у
установленной пары и реже портит правильный текст.

Сквозные тесты затем показали то, чего эти числа не видят: три слова из курируемого списка
коротких слов (`мы`, `вы`, `if`) кандидат предлагает проверить вместо того, чтобы переключить.
Выпуск выходит на проверенной паре; кандидат ждёт исправления правила, которое решает такие
слова.

### Короткие слова и имена команд

- Имена команд с цифрами разбирает модель, а не общее исключение для «похожего на код»:
  `зь2` становится `pm2`, а набранное как есть `pm2` остаётся собой. Прежде такие токены
  вовсе не рассматривались, и `зь2` оставалось как есть.
- Из русского дополнения словаря убраны 467 двухбуквенных «слов», пришедших из субтитров
  (`зь`, `пм`, `ср`, `фы`). Настоящие двухбуквенные русские слова и так есть в основном
  словаре, а этот шум выдавал короткую латинскую команду за русское слово.
- Одиночная буква в начале сообщения переключается сразу на пробеле: `z ` становится `я `,
  и так же для `f`, `b`, `c`, `d`, `r`, `e`, `j` — восьми клавиш, на которых лежат восемь
  однобуквенных русских слов. Раньше такая буква ждала следующего слова.

### Курсор, перенесённый в готовый текст

Стрелки, `Home`, `End` и `Page Up`/`Page Down` уводят курсор туда, где программа ничего не
видела, а переносят его туда чаще всего затем, чтобы дописать или поправить уже написанное.
Слово, набранное сразу после такого перемещения, больше не переключается автоматически:
`Pause` переключит его вручную. Настройка «Не исправлять слово сразу после перемещения
курсора» отключает это поведение. Если чтение контекста активного поля работает, текст перед
курсором известен по-настоящему и правило не применяется.

### Автозапуск в Windows

Если сборка сообщала о себе как об интерпретаторе, в список автозапуска попадал `python.exe`:
Windows показывала запись «Python» без издателя, и её запуск не запускал KeySwitch. Теперь
команда берётся из установленной раскладки файлов. Состояние автозапуска — какая команда
зарегистрирована, не блокирует ли её Windows, существует ли цель — показывает окно
диагностики, а не только `KeySwitch.exe --diagnose`.

### Известные ограничения

- Изолированное слово, неизвестное словарю (`дюп`), всё ещё может быть переведено.
- Числа выше измерены на запечатанном тестовом корпусе из 512 строк естественного и
  технического текста; они не являются оценкой на произвольном вводе.
- Оценка выполнена на модели ввода в пределах процесса: она не проверяет работу с IME,
  гонки фокуса и подтверждение ввода операционной системой.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

This release fixes what made short words and command names confuse the two layouts, and adds
a rule for text typed after the caret is moved into what is already written. The models are
the ones that shipped in 0.24.0. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.25.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-Setup-0.25.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.25.0-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Same models, the candidate stays a candidate

The context-v1 + prefix-v1 pair stays installed. The new-schema candidate passed its sealed
evaluation — 512 rows from Tatoeba sentences and the Fedora Rawhide package index, none of
which any earlier evaluation had seen: it restores 228 of 252 wrongly typed rows against 210
for the installed pair and corrupts correct text less often.

The end-to-end tests then showed what those numbers cannot: three words of the curated
short-word list (`мы`, `вы`, `if`) are offered as a suggestion instead of being switched.
This release ships the proven pair; the candidate waits for the rule that decides such words
to be repaired.

### Short words and command names

- Command names carrying digits are decided by the models rather than by a blanket
  exemption for anything that looks like code: `зь2` becomes `pm2`, and `pm2` typed as
  itself stays. Such tokens used to be left out of the analysis entirely, so `зь2` stood
  as it was.
- 467 two-letter "words" that came from subtitles (`зь`, `пм`, `ср`, `фы`) were removed from
  the Russian lexicon supplement. The real two-letter Russian words are in the main
  dictionary anyway, and that noise disguised short Latin commands as Russian words.
- A lone letter opening a message is switched on the space itself: `z ` becomes `я `, and
  the same for `f`, `b`, `c`, `d`, `r`, `e` and `j` — the eight keys that carry the eight
  one-letter Russian words. Such a letter used to wait for the next word.

### A caret moved into finished text

Arrow keys, `Home`, `End` and `Page Up`/`Page Down` put the caret where the program has
watched nothing being typed, and it is usually moved there to finish or fix what is already
written. The word typed right after such a move is no longer switched automatically;
`Pause` switches it by hand. The setting "Do not correct a word right after the caret
moves" turns the behaviour off. Where the active field can be read, the text before the
caret is really known and the rule does not apply.

### Windows startup

A build that reported an interpreter as its own program could register `python.exe` in the
startup list: Windows showed an entry named "Python" with no publisher, and starting it did
not start KeySwitch. The command is now taken from the installed layout. The diagnostics
window, not only `KeySwitch.exe --diagnose`, reports the startup state: the command that is
registered, whether Windows blocks it and whether its target still exists.

### Known limitations

- An isolated word unknown to the lexicon (`дюп`) can still be converted.
- The numbers above come from a sealed test corpus of 512 rows of natural and technical
  text; they are not a measurement on arbitrary input.
- The evaluation runs against an in-process input model: it does not cover IME input, focus
  races or acknowledgement of the injected text by the operating system.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

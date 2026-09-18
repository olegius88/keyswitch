# KeySwitch 0.25.2

## Русский

Выпуск исправляет одну вещь: пропадавшее нажатие Enter. Всё остальное — как в 0.25.1,
полный перечень в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.25.2_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.25.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.25.2-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Enter больше не пропадает

Программа ненадолго придерживает Enter и Tab: сначала исправить слово, потом отправить —
иначе улетело бы неисправленное. Отпустить клавишу она соглашается, когда отпущены все
остальные.

Здесь и была ошибка. Нажатие, чьё отпускание потерялось — а это случается при смене фокуса
и при перехвате ввода, — забывалось через три секунды, а придержанный Enter сдавался через
две. Он выбрасывался за секунду до того, как помеха исчезла бы сама. Одной застрявшей
клавиши хватало, чтобы Enter перестал срабатывать до перезапуска программы; в разобранном
сеансе так потерялось шестнадцать нажатий.

Теперь клавиша, зажатая до Enter и всё ещё зажатая к сроку, не считается препятствием:
её забывают, и Enter уходит по назначению. Осторожность остаётся там, где она к месту —
если не видели отпускания самого Enter или если клавишу нажали уже после него.

### Известные ограничения

- Если в те же две секунды нажать кнопку мыши или прокрутить колесо, придержанный Enter
  по-прежнему отменяется: клик может увести каретку, и отправлять после него опаснее, чем
  потерять нажатие. В разобранном сеансе так терялось семь нажатий — все из тех, что уже
  висели из-за потерянного отпускания.
- Исправление устраняет последствие, а не первопричину: почему на Windows теряется
  отпускание клавиши, ещё выясняется.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

This release fixes one thing: a disappearing Enter. Everything else is as in 0.25.1; the
full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.25.2_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-Setup-0.25.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.25.2-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Enter no longer disappears

The program withholds Enter and Tab for a moment: correct the word first, submit second,
or the uncorrected text would be sent. It agrees to let the key go once every other key
has come up.

That is where the fault was. A press whose release is lost — which happens on a focus
change and while input is held — was only forgotten after three seconds, while the
withheld Enter gave up after two. The keystroke was dropped one second before the obstacle
would have cleared itself. A single stuck key was enough to make Enter stop working until
the program was closed; the session examined here lost sixteen presses that way.

A key already held when the Enter arrived and still held at the deadline is no longer
treated as an obstacle: it is forgotten and the Enter goes through. The cautious answer
stays where it belongs — when the Enter's own release was never seen, or when a key
pressed after it is still down.

### Known limitations

- Pressing a mouse button or turning the wheel during those two seconds still cancels a
  withheld Enter: a click can move the caret, and submitting after one is worse than
  losing the keystroke. The examined session lost seven presses that way — all of them
  already hanging on a lost release.
- The fix addresses the consequence, not the cause: why a key release goes missing on
  Windows is still being investigated.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

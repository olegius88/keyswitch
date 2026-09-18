# KeySwitch 0.25.3

## Русский

Выпуск доводит до конца исправление прошлого: причина пропадавшего Enter устранена там,
где она возникает. Всё остальное — как в 0.25.2, полный перечень в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.25.3_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.25.3-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.25.3-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Застрявшие клавиши больше не накапливаются

Прошлый выпуск научил придержанный Enter уходить по назначению, несмотря на клавишу, чьё
отпускание потерялось. Теперь устранено само появление таких клавиш.

Клавиша, зажатая в момент смены окна, сообщает об отпускании уже новому окну — а нередко
никому, кого программа может услышать. Нажатие оставалось в её книгах до истечения
трёхсекундного срока, и всё это время мешало придержанным Enter и Tab. Теперь оно
забывается в момент смены фокуса: именно тогда оно перестаёт что-либо значить. Если
клавиша и правда ещё зажата, не теряется ничего — её отпускание просто не найдёт, что
снимать.

### Известные ограничения

- Если нажать кнопку мыши или прокрутить колесо, пока Enter придержан, он по-прежнему
  отменяется: клик может увести каретку, и отправлять после него опаснее, чем потерять
  нажатие.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

This release finishes what the previous one started: the cause of the disappearing Enter
is removed where it arises. Everything else is as in 0.25.2; the full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.25.3_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-Setup-0.25.3-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.25.3-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Stuck keys no longer accumulate

The previous release taught a withheld Enter to go through despite a key whose release was
lost. Such keys now stop appearing in the first place.

A key still down when the window changes reports its release to the window that took over,
and often to nobody the program can hear. The press stayed in its books until a three
second timer expired, obstructing every withheld Enter and Tab meanwhile. It is now
forgotten at the focus change itself, the moment it stops meaning anything. A key that
really is still down loses nothing: its release, if it ever arrives, simply finds nothing
to clear.

### Known limitations

- Pressing a mouse button or turning the wheel while an Enter is withheld still cancels
  it: a click can move the caret, and submitting after one is worse than losing the
  keystroke.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

# KeySwitch 0.11.2

## Русский

Выпуск про диагностику самой замены: журнал теперь объясняет испорченный
результат и молчаливо отменённое исправление. Сертифицированная модель
намерения `intent-v1-6ece07f881ec` не изменилась.

- Замена записывает, сколько в неё напечатали. В `correction_applied` и
  `correction_failed` появилось `keys_during_injection` — настоящие нажатия,
  пришедшие, пока текст переписывался, — рядом с `queued_events`,
  `replayed_strokes` и `boundary_replayed`. Текст из приложения обратно никто
  не читает, поэтому именно это число объясняет замену, вышедшую с лишними или
  потерянными символами, при том что все шаги отчитались об успехе.
- Запланированное исправление, которое так и не выполнилось, больше не молчит.
  Оно ждёт отпускания клавиши-триггера, и сочетание клавиш, перемещение
  курсора, смена фокуса или второй `Pause` в этом промежутке отменяли его без
  следа; теперь в журнал попадает `pending_correction_dropped` с причиной,
  словом и направлением. `Pause`, которому нечего преобразовывать и некуда
  переключаться, пишет `manual_conversion_impossible`.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.11.2-x64.exe` или переносимый
  `KeySwitch-0.11.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.2_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release about diagnosing the replacement itself: the log now explains a
corrupted result and a correction that was cancelled without a word. The
certified intent model `intent-v1-6ece07f881ec` is unchanged.

- A correction records how much was typed into it. `correction_applied` and
  `correction_failed` carry `keys_during_injection` — the real keystrokes that
  arrived while the text was being replaced — next to `queued_events`,
  `replayed_strokes` and `boundary_replayed`. Nothing reads the text back from
  the application, so this is the number that explains a replacement that came
  out with extra or missing characters while every step reported success.
- A correction that is scheduled but never runs is no longer silent. It waits
  for the release of the key that triggered it, and a shortcut, a caret move, a
  focus change or a second `Pause` in between used to cancel it without a
  trace; the log now records `pending_correction_dropped` with the reason, the
  word and the direction. A `Pause` with neither a word nor a second layout
  records `manual_conversion_impossible`.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.11.2-x64.exe` or the portable
  `KeySwitch-0.11.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.2_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

# KeySwitch 0.7.0

## Русский

Этот выпуск ускоряет обучение локальной модели намерения, исправляет
восстановление окна в Windows и переводит приложение на новый сертифицированный
KSLM-артефакт v15.

- Offline trainer использует все logical CPU, доступные процессу: извлечение
  признаков, оценка эпох, калибровка, выбор порогов и sealed-test scoring идут
  в process pool с каноническим порядком строк, а `--workers N` ограничивает
  число процессов. Online-обновление FTRL-Proximal остаётся последовательным,
  поэтому результат побайтно не зависит от числа worker-ов.
- Изменённый trainer означает новую toolchain identity, поэтому выпущен новый
  кандидат `intent-v1-bec1f1d3dceb`: заново заморожен model-blind
  development-корпус, создан holdout v15 с нулевыми пересечениями, пройдены все
  внутренние gates и независимая strict-проверка (30 из 30 gates). На новом
  holdout из 60 000 негативов ансамбль дал 12 ложных срабатываний при recall
  0,96935; полные метрики и хэши приведены в model card.
- В Windows подсказка обучения больше не «восстанавливает» развёрнутое или
  прикреплённое окно: фокус возвращается без `SW_RESTORE`.
- Добавлен `tools/release_pipeline.py`: один отсоединённый от терминала процесс
  выполняет весь Linux-контур проверки и сборки, параллельно и с учётом
  свободного ОЗУ, и оставляет `SUMMARY.md` с чек-листом runbook.

Локальный контур выпуска: строгий `mypy`, 368 автоматических тестов и 100%
покрытия строк и ветвей.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.7.0-x64.exe` или переносимый
  `KeySwitch-0.7.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.7.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This release speeds up training of the local intent model, fixes window
restoration on Windows and moves the application to a new certified v15 KSLM
artifact.

- The offline trainer uses every logical CPU available to the process: feature
  extraction, epoch evaluation, calibration, threshold selection and sealed-test
  scoring run in a process pool with canonical row order, and `--workers N`
  caps the process count. The online FTRL-Proximal update stays sequential, so
  the result is byte-for-byte independent of the worker count.
- A changed trainer is a new toolchain identity, so a new candidate
  `intent-v1-bec1f1d3dceb` was released: the model-blind development corpus was
  frozen again, a v15 holdout was presealed with zero overlap, and every
  internal gate plus the independent strict evaluation (30 of 30 gates) passed.
  On the new 60,000-negative holdout the ensemble produced 12 false positives
  with recall 0.96935; the complete metrics and hashes are in the model card.
- On Windows the learning prompt no longer "restores" a maximized or snapped
  window: focus returns without `SW_RESTORE`.
- `tools/release_pipeline.py` runs the whole Linux verification and packaging
  contour as one detached process, concurrently and within the available RAM,
  and leaves `SUMMARY.md` with the runbook checklist.

The local release gate passes strict `mypy`, 368 automated tests and mandatory
100% line and branch coverage.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.7.0-x64.exe` or the portable
  `KeySwitch-0.7.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.7.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

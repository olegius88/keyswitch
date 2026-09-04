# KeySwitch 0.11.1

## Русский

Выпуск про диагностику локального обучения: по журналу теперь видно, что
выученные правила сделали со словом. Сертифицированная модель намерения
`intent-v1-6ece07f881ec` не изменилась.

- В каждой записи о слове (`word_evaluation`) появился блок `learning`:
  включено ли обучение, куда ведёт правило для этого слова и сколько
  подтверждений оно набрало, сколько требуется, какую цель форсирует
  подтверждённое правило и какие направления запрещены вашими отменами.
  Правило с одним подтверждением из двух ещё ничего не меняет — раньше такая
  строка ничем не отличалась от слова, для которого правила нет вовсе.
- Запись правила стала отдельным событием: `learning_rule_recorded` называет
  слово, направление, число подтверждений и стало ли правило рабочим, а
  `learning_rejection_recorded` фиксирует отмену, запрещающую исправление.
  Подтверждение подсказки сообщает итоговое число подтверждений.
- Кнопка «Сброс» у отдельной настройки теперь сразу обновляет своё поле, а не
  ждёт очередного такта очереди настроек: показанное значение всегда совпадает
  с сохранённым.

Выпуск 0.11.0 не был опубликован — сборка Windows остановилась на этом же
дефекте, — поэтому здесь же всё, что вошло в него: журнал, который наконец
пишется и подписан версией, цифра внутри слова (`зь2` → `pm2` по Pause),
`word_discarded` с причиной, прокрутка страниц настроек и подсветка изменённых
параметров.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.11.1-x64.exe` или переносимый
  `KeySwitch-0.11.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.1_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release about diagnosing local learning: the log now shows what the learned
rules did with a word. The certified intent model `intent-v1-6ece07f881ec` is
unchanged.

- Every `word_evaluation` carries a `learning` block: whether learning is on,
  the target and confirmation count of the rule for this word, how many
  confirmations it still needs, the target a confirmed rule forces, and the
  directions rejected by your undos. A rule with one confirmation out of two
  changes nothing yet, and such a line used to be indistinguishable from a word
  with no rule at all.
- Writing a rule is an event of its own: `learning_rule_recorded` names the
  word, the direction, the confirmation count and whether the rule became
  active, while `learning_rejection_recorded` records an undo that blocks a
  correction. Confirming the prompt reports the resulting count as well.
- The per-setting reset button updates its control at once instead of waiting
  for the settings queue tick, so the value shown always matches the value
  stored.

Release 0.11.0 was never published — the Windows build stopped on this very
defect — so this release also carries everything prepared for it: a log that is
finally written and stamped with the version, a digit inside a word (`зь2` →
`pm2` on Pause), `word_discarded` with a reason, scrolling settings pages and
the marking of changed settings.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.11.1-x64.exe` or the portable
  `KeySwitch-0.11.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.1_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

# KeySwitch 0.8.0

## Русский

Этот выпуск делает обучение и сертификацию локальной модели намерения в
разы быстрее без изменения её математики и переводит приложение на новый
сертифицированный KSLM-артефакт v20.

- Последовательные эпохи FTRL-Proximal выполняет компилируемое ядро,
  встроенное в trainer: оно повторяет эталонный Python-цикл выражение за
  выражением и перед первой эпохой обязано совпасть с ним бит-в-бит на
  реальных строках. Эпоха занимает секунды вместо минуты, а полный цикл
  кандидата (заморозка корпуса, preseal, обучение, strict-оценка) — около
  30 минут вместо нескольких часов.
- Strict evaluator оценивает строки на worker-процессах; решения, логиты и
  счётчики кэша не зависят от числа worker-ов (однопоточный и параллельный
  отчёты совпадают во всех разделах, кроме замеров времени).
- Таблицы преобразования раскладок US/RU предвычисляются один раз, поэтому
  приложение, trainer и evaluator больше не пересобирают словарь при каждом
  слове; результат посимвольно тот же.
- Сборка DEB и CI переиспользуют один проверенный strict-отчёт вместо
  повторной получасовой оценки; отчёт принимается только при совпадении всех
  записанных в нём хэшей с текущими файлами.
- Новый артефакт `intent-v1-6ece07f881ec` (кандидат v20): все внутренние
  gates, model-blind holdout (6 ложных срабатываний из 60 000, ни одно не
  внесено моделью относительно fallback, recall 0,9445) и 30 независимых
  strict gates пройдены. Кандидаты v16, v17 и v19 не прошли pre-sealed gate,
  v18 отклонён strict-gate `fallback_regression`; полные хэши и метрики — в
  model card и `rejection-v18.json`.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.8.0-x64.exe` или переносимый
  `KeySwitch-0.8.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.8.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This release makes training and certifying the local intent model several
times faster without changing its arithmetic, and moves the application to a
new certified v20 KSLM artifact.

- The sequential FTRL-Proximal epochs run in a compiled kernel embedded in the
  trainer: it mirrors the reference Python loop expression by expression and
  must reproduce it bit for bit on real rows before the first epoch. An epoch
  takes seconds instead of a minute, and a complete candidate cycle (corpus
  freeze, preseal, training, strict evaluation) takes about 30 minutes
  instead of several hours.
- The strict evaluator scores rows on worker processes; decisions, logits and
  cache counters do not depend on the worker count (single-process and
  parallel reports agree in every section except timing).
- The US/RU layout translation tables are precomputed once, so the
  application, trainer and evaluator no longer rebuild the mapping for every
  word; the result is identical character for character.
- The DEB build and CI reuse one verified strict report instead of repeating
  the half-hour evaluation; a report is accepted only when every hash it
  recorded matches the current files.
- The new artifact `intent-v1-6ece07f881ec` (candidate v20) passed every
  internal gate, the model-blind holdout (6 false positives among 60,000, none
  introduced by the model relative to the fallback, recall 0.9445) and all 30
  independent strict gates. Candidates v16, v17 and v19 failed the pre-sealed
  gate and v18 was rejected by the `fallback_regression` strict gate; the
  complete hashes and metrics are in the model card and `rejection-v18.json`.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.8.0-x64.exe` or the portable
  `KeySwitch-0.8.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.8.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

# Обучение моделей в облачной сессии Claude Code

Инструкция для запуска новых этапов обучения и проверок моделей в облачной
сессии Claude Code (claude.ai/code). Облачный контейнер начинается с чистого
клона репозитория и удаляется после простоя. Всё, что не запушено и не
скопировано наружу, теряется.

## Что подготавливается автоматически

При старте облачной сессии запускается хук
[`.claude/hooks/session-start.sh`](../.claude/hooks/session-start.sh),
зарегистрированный в [`.claude/settings.json`](../.claude/settings.json).
Вне облака (`CLAUDE_CODE_REMOTE` не равно `true`) хук ничего не делает. Хук:

- ставит тот же список системных пакетов, что и job Tests: среди них
  `onboard-data` (лексиконы), `hunspell-en-us` и `hunspell-ru` (морфология),
  `libhunspell-1.7-0`, `build-essential` (компилятор для
  `tools/context_optimizer.c`), GTK 4 и Xvfb для `tests/run_coverage.sh`;
- выбирает системный Python, в котором импортируются `gi`, `dbus` и `coverage`
  из пакетов apt, и ставит ссылку на него первой в `PATH` как `python3`. В
  облачном образе `python3` может указывать на другую сборку (на 01.10.2026 это
  Python 3.11, а apt-пакеты Ubuntu 24.04 собраны для 3.12), и тогда GTK-тесты не
  импортируются;
- ставит закреплённый mypy и заглушки PyGObject в `.typing/` тем же
  интерпретатором (`tools/install-typing-tools.sh`);
- экспортирует в сессию `PATH`, `PYTHONPATH=src` и `KEYSWITCH_TYPING_ROOT=.typing`.

Хук идемпотентен: уже установленные пакеты не переустанавливаются. Он начнёт
работать во всех облачных сессиях после слияния в ветку по умолчанию.

## Проверка готовности

```bash
PYTHONPATH=src python3 tools/training_doctor.py
```

[`training_doctor.py`](../tools/training_doctor.py) ничего не обучает и не
скачивает. Он сверяет системные лексиконы и словари Hunspell с хешами в
[`model/intent_v1/config.json`](../model/intent_v1/config.json), загружает
reference-модели всех текущих тренеров, ищет компилятор и приватные данные.
Затем печатает по каждому этапу `ready` с командой запуска или `blocked` с
перечнем недостающего. С `--stage ИМЯ` код выхода ненулевой, если названный
этап заблокирован; так этап можно поставить первым шагом скрипта.

## Что можно запускать в облаке

Проверено 01.10.2026 в контейнере Ubuntu 24.04 (4 CPU, 15 ГБ RAM).

| Этап | Что делает | Требует | Результат проверки |
| --- | --- | --- | --- |
| `replay-prefix-v1` | воспроизводит seal выпущенной prefix-v1 | репозиторий | совпал |
| `replay-boundary-v2` | воспроизводит seal выпущенной boundary-v2 | репозиторий | совпал |
| `replay-ortho-v1` | воспроизводит ortho-v1 | репозиторий | совпал |
| `replay-context-v1` | переобучает context-v1 бит в бит, около 10 минут | системные пакеты хука | совпал после установки пакетов, без них расходится |
| `train-prefix-v2` | новый кандидат по [`model/prefix_v2/recipe.json`](../model/prefix_v2/recipe.json) | репозиторий, компилятор | обучение и calibration прошли за 19 минут; кандидат `prefix-v2-bf3dc28f8567` отличается от `prefix-v2-d1ee002d9ab2` из README context v3 |
| `train-context-v3` | новый кандидат по [`model/context_v3/recipe.json`](../model/context_v3/recipe.json) | приватный корпус | заблокирован без `.t/` |
| `evaluate-context-v3` | последовательности для пары context-v3 + prefix-v2 | приватный корпус и журнал test | заблокирован без `.t/` |

Системные лексиконы `onboard-data` Ubuntu 24.04 и словари Hunspell совпадают
по SHA-256 с закреплёнными, поэтому replay context-v1 даёт те же байты, что в CI.

## Версия Python и расхождения с CI

CI работает на ubuntu-26.04 с более новым Python, чем облачный образ. Проверено
01.10.2026 на Python 3.12 (интерпретатор, выбранный хуком):

- воспроизведение корпусов ortho (`ortho_corpus.py`, `ortho_known.py`,
  `ortho_v2_corpus.py`, `ortho_v2_known.py`, `ortho_v2_verified.py`) зависит от
  версии базы Unicode: на 3.11 и 3.12 (Unicode 14.0 и 15.0) оно расходится уже на
  исходном коде, на `/usr/bin/python3.13` (Unicode 15.1) проходит. Эти шаги
  запускайте через `python3.13`;
- четыре теста Fedora/openSUSE в `tests/test_context_action_holdout.py`
  импортируют `compression.zstd` из Python 3.14 и в образе не запускаются;
- `test_the_default_target_check_reads_the_file_system_and_survives_a_bad_path`
  падает на 3.12: там `Path.is_file()` бросает `OSError` на слишком длинное имя;
- coverage 7.4.4 из Ubuntu 24.04 на Python 3.12 считает отдельными ветвями
  переходы `->exit` генераторных выражений и импорты под `if TYPE_CHECKING:`,
  поэтому показывает 99% там, где CI видит 100%.

Остальные тесты (1695 из 1701) и все прочие проверки моделей из `tests.yml`
проходят в образе.

Результаты обучения пишите в `.t/` (каталог игнорируется git), например
`--output .t/prefix-v2/<имя-запуска>`. Протокол принятия каждой модели описан
в её README; облако его не меняет. Development-отчёт не разрешает продвижение,
а повторная оценка изменённого кандидата на том же test не считается
независимой.

## Приватные данные `.t/`

Замороженный корпус context-action и журнал доступа к test лежат в
`.t/reliable-release-2026-09-12/` на компьютере автора. Через git они в облако
не попадают: полные корпуса и строки отчётов намеренно не публикуются
([context v3](../model/context_v3/README.md)). Для этапов context-v3 есть два
пути:

1. **Перенести те же данные.** Положите архив каталога
   `.t/reliable-release-2026-09-12/` в приватное хранилище, доступное сессии
   (например, отдельный приватный репозиторий, подключённый к сессии), и
   распакуйте его в `.t/` контейнера. Журнал test должен переезжать вместе с
   корпусом: без него evaluator не может подтвердить, что test ещё не
   расходовался. Не коммитьте эти файлы в публичный репозиторий.
2. **Собрать заново из публичных источников.** Это новый корпус с новым
   namespace и новым test, а не воспроизведение прежнего; сравнивать его числа
   с прежними результатами напрямую нельзя. Так собраны корпуса v10–v12
   (01.10.2026), v13–v16 (02.10.2026), v17–v18 (03.10.2026), v19–v20 (04.10.2026) и v21–v27
   (04–05.10.2026); на корпусе v26 обучена модель слов установленной пары context-v3 + prefix-v2 с
   ранней сменой по умолчанию (в 0.38.0 и 0.38.1 — на корпусе v18). Корпуса v25–v27 выбирают
   предложения для обучения с namespace корпуса v23 (`--namespace …-public-v23:fit-v23` у
   `freeze_context_action_fitting.py`), а test — со своим. Заморозки
   holdout и fitting не берут в выборки предложения Tatoeba, на которых записаны вопросы движка
   `model/context_v1/captured` (`tatoeba-authors.tsv.xz`): тренер может учиться на этих вопросах
   (рецепт `captured_curriculum`). Порядок (cwd — корень репозитория, `PYTHONPATH=src:tools`,
   ни один выходной каталог не должен существовать заранее):

   1. Источники. Архивы GitHub из облака недоступны, файлы деревьев UD
      скачиваются по одному с `raw.githubusercontent.com/UniversalDependencies/
      <дерево>/<коммит>/<файл>` (`README.md`, `LICENSE.txt`, `*.conllu`), и
      для каждого дерева пишется `pin.json` с `repository`, `commit` и списком
      `files` (`path`, `sha256`, `sha` — git blob). Коммиты закреплены в
      `PINS` в `tools/freeze_context_action_corpus.py` (база: Taiga, EWT) и
      `tools/freeze_context_action_holdout.py` (holdout). Экспорты Tatoeba
      (`downloads.tatoeba.org/exports/per_language/<язык>/<язык>_sentences.tsv.bz2`)
      и индексы Debian (`Contents-amd64.gz` + `InRelease` для trixie и sid)
      заверяются `source-receipt.json` той формы, которую проверяют
      `verified_tatoeba_source`, `verified_source` и `verified_sid_source`.
   2. `tools/freeze_context_action_corpus.py --source-root <ud> --output <ud0>
      --namespace <ns> --extra-exposure model/context_v1/scenarios.json
      --extra-exposure model/context_v1/holdout-2.json --extra-exposure
      model/context_v1/holdout-3.json`.
   3. `tools/reconcile_context_action_corpus.py --corpus <ud0> --source-root <ud>
      --report <r> --output <ud1>` — добавляет `physical-family-closure.json`.
   4. `tools/context_technical_corpus.py --source-directory <trixie> --ud-corpus
      <ud1> --extra-exposure model/context_v1/holdout-3.json --output <tech>
      --preview-report <r>`.
   5. `tools/merge_context_action_corpora.py --base <ud1> --technical <tech>
      --output <merged>`.
   6. `tools/prefix_exposure_inventory.py --output <inventory.json>` — алиасы
      слов, на которых обучена prefix-модель; holdout и fitting отказывают им в
      test, потому что модели оцениваются вместе.
   7. `tools/freeze_context_action_holdout.py --source-root <holdout-ud>
      --treebank ... --tatoeba-directory <tatoeba> --tatoeba-language rus
      --tatoeba-language eng --sid-directory <sid> --base <merged> --ud-origin
      <ud1> --technical-origin <tech> --prefix-inventory <inventory.json>
      --ledger <ledger> --namespace <ns-test> --report <r> --output <hold>` —
      свежий test; прежний уходит в quarantine.
   8. `tools/freeze_context_action_fitting.py --base <hold> --output <fit>
      --namespace <ns-fit> --tatoeba-directory <tatoeba> --tatoeba-language rus
      --tatoeba-language eng --ud-origin <ud1> --technical-origin <tech>
      --prefix-inventory <inventory.json> --ledger <ledger> --report <r>` —
      естественные предложения Tatoeba в обучающих сплитах.
   9. Обучение и оценка — как в README context v3: `train_context_action_model.py
      --corpus <fit>`, `train_prefix_v2_model.py`, затем
      `evaluate_context_action_sequences.py` на development и один раз на test
      (`--corpus <fit>`; журнал пишется в `LEDGER_ROOT` evaluator'а).

   Журнал доступа начинается пустым: прежние test v1–v9 новому корпусу не
   известны, и их слова он не исключает.

## Как не потерять результат

Контейнер временный. Перед завершением сессии:

- коммитьте и пушьте то, что по протоколу модели входит в репозиторий
  (recipe, candidate, seal, публичный отчёт, README);
- приватные выходы (`features-*.jsonl.gz`, строки отчётов, журналы) переносите
  тем же приватным путём, что и корпус;
- журнал доступа к test сохраняйте вместе с корпусом, иначе следующий запуск
  оценит уже израсходованный test как новый.

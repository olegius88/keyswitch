# Проверка, сборка и выпуск

Инструкция для текущего рабочего дерева. Все команды — из корня репозитория.
Разделы проверок не делают коммит, push или GitHub Release. Последний раздел
публикации меняет Git и внешнее состояние; запускайте его только после явного
решения о выпуске и проверки состава изменений.

## Среда и быстрые проверки

Linux reference CI использует Ubuntu 26.04 и системный Python 3.14.
Минимальная версия Python из `pyproject.toml` не означает, что любая такая
среда может побайтно воспроизвести обучение. Полные зависимости перечислены
в [Tests workflow](../.github/workflows/tests.yml) и
[runbook](intent-model-runbook.md#1-подготовить-эталонную-среду).

```bash
git status --short
git diff --check
./tools/install-typing-tools.sh .typing
KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh
./tools/run-gui-test.sh -- ./tests/run_coverage.sh
PYTHONPATH=src python3 tools/verify_context_model.py
PYTHONPATH=src python3 tools/verify_context_v2.py
PYTHONPATH=src python3 tools/verify_boundary_model.py
PYTHONPATH=src python3 tools/verify_boundary_v2.py
PYTHONPATH=src python3 tools/verify_prefix_model.py
```

100% line/branch coverage относится к [.coveragerc](../.coveragerc): несколько
нативных Win32-модулей и Windows UI исключены. Windows CI имеет собственную
область coverage, настоящий hook/SendInput E2E и проверку установленного EXE.
Linux-моки и `mypy --platform win32` не заменяют эти нативные проверки.

### GUI-тесты без системных порталов и sudo

Штатный запускатель `tools/run-gui-test.sh` создаёт Xvfb, затем личную сессию
D-Bus: активируемые службы получают именно тестовый `DISPLAY`. Общий файл
`tools/gui-test-env.sh` задаёт `GDK_DEBUG=no-portals` для GTK4,
`ADW_DISABLE_PORTAL=1` для отдельного клиента libadwaita, `GTK_USE_PORTAL=0`
для совместимости и локальный GIO. Его используют coverage, исходный X11 E2E,
проверки извлечённого пакета, AT-SPI, CI и релизный конвейер.

В ограниченном процессе с `NoNewPrivs=1` setuid-помощник `fusermount3` не может
получить права на монтирование: ненужная активация `xdg-document-portal`
давала `Operation not permitted`. Одного `GTK_USE_PORTAL=0` недостаточно для
GTK4/libadwaita. Это исправление тестового окружения, не отключение служб
рабочего стола, не подавление stderr и не изменение прав `/dev/fuse`.
Пароль sudo для проверок не нужен. Не переносите эти переменные в профиль
пользователя или обычный запуск приложения.

GTK-тесты и X11 E2E проверяют по D-Bus, что Desktop/Documents portal не
активировались. Настоящие проверки клавиатуры, окна, курсора, выделения и
паролей сохраняются; AT-SPI не отключается общим запускателем. Этот контур
не проверяет интеграцию приложения с системным порталом выбора файлов.

Основания: [GDK_DEBUG в GTK4](https://docs.gtk.org/gtk4/running.html#gdk-debug),
[отдельный клиент libadwaita](https://github.com/GNOME/libadwaita/blob/main/src/adw-settings-impl-portal.c),
[NoNewPrivs в ядре Linux](https://docs.kernel.org/userspace-api/no_new_privs.html).

`verify_context_model.py` проверяет рабочую context-v1, её отчёт и связанные
hashes. `verify_context_v2.py` проверяет эксперимент и требует сохранения
точных рабочих v1-весов: исследовательский кандидат был отклонён. Это быстрые
проверки сохранённых доказательств, а не новое обучение или полевая оценка.

## Повторное обучение и проверка контекстных моделей

Рабочая context-v1 использует лексические оценки, полученные через
`LanguageModel.load()`. Её повторное обучение требует reference-словарей и
среды CI; переопределения моделей или пользовательские Hunspell-словари могут
изменить результат. На произвольном компьютере быстрая проверка артефакта
не равнозначна успешному replay.

```bash
PYTHONPATH=src python3 tools/train_context_model.py --verify
PYTHONPATH=src python3 tools/verify_context_model.py
```

Расширенный эксперимент использует уже зафиксированные публичные предложения
и числовой словарный кэш, без загрузок из сети и без зависимости от установленных
Hunspell-словарей во время replay:

```bash
PYTHONPATH=src python3 tools/context_corpus.py
PYTHONPATH=src python3 tools/context_evidence.py
PYTHONPATH=src python3 tools/train_context_v2.py verify
PYTHONPATH=src python3 tools/evaluate_context_engine.py --verify
PYTHONPATH=src python3 tools/verify_context_v2.py
```

При изменении только движка старый engine-report больше не описывает текущий
код. Для явной регрессионной перепроверки существует
`PYTHONPATH=src python3 tools/evaluate_context_engine.py --refresh-runtime`.
Команда сохраняет предыдущий отчёт в `model/context_v2/engine-history/`,
проверяет неизменность сравниваемых весов и списка фраз и выполняет replay
заново. Новый отчёт помечен как повторная проверка уже наблюдавшегося теста;
это не новый независимый тест и не разрешение продвигать исследовательские
веса. После обновления обычный `--verify` должен воспроизводить его точно.

Для training replay эксперимента нужен Linux C-компилятор; GPU не используется.
Команды выше проверяют сохранённые результаты и не устанавливают candidate
в приложение. `fit`/`evaluate` и `--freeze` предназначены для отдельного цикла
исследования, не для обычной проверки документации или сборки. Не удаляйте seal
и не перезаписывайте раскрытый test, чтобы повторить изменённого кандидата.
[Данные, результаты и границы эксперимента](../model/context_v2/README.md).

Эксперимент границ слова проверяется отдельно от контекстных весов:

```bash
PYTHONPATH=src python3 tools/train_boundary_model.py --verify
PYTHONPATH=src python3 tools/verify_boundary_model.py
PYTHONPATH=src python3 tools/evaluate_boundary_engine.py --verify
```

Кандидат boundary-v1 отклонён и не включён в программу. Его обучение использует слова из
прежнего резерва публичных фраз; этот резерв больше нельзя считать новым
независимым материалом. Подробности и ограничения —
[в отчёте эксперимента](../model/boundary_v1/README.md). Запускайте тяжёлые
обучения и replay последовательно. API-моки не заменяют нативный Windows E2E.

Принятая boundary-v2 проверяется отдельно, без изменения старого v1-test:

```bash
PYTHONPATH=src python3 tools/boundary_v2_corpus.py
PYTHONPATH=src python3 tools/train_boundary_v2.py --verify
PYTHONPATH=src python3 tools/verify_boundary_v2.py
```

Первый sealed test boundary-v2 и его ограничения описаны
[в карточке модели](../model/boundary_v2/README.md). Повторный `--verify`
сравнивает веса/отчёты побайтно, а не создаёт новую независимую проверку.
Общий `evaluate_boundary_engine.py` теперь явно сравнивает старый алгоритм,
отклонённый v1 и активный v2; прежние отчёты архивируются.

Раннюю модель префиксов проверяют отдельные команды (последовательно):

```bash
PYTHONPATH=src python3 tools/prefix_corpus.py
PYTHONPATH=src python3 tools/train_prefix_model.py verify
PYTHONPATH=src python3 tools/evaluate_prefix_engine.py --verify
PYTHONPATH=src python3 tools/verify_prefix_model.py
```

Обучение использует замороженные числовые признаки; сквозной replay требует
эталонных словарей. `verify` не меняет веса, не подбирает порог и не продвигает
модель. Для изменений только движка доступен `evaluate_prefix_engine.py
--refresh-runtime` с архивированием старого отчёта; это регрессия, не свежий
независимый тест. [Методика и ограничения](../model/prefix_v1/README.md).

Нативное чтение поля проверяется отдельно:

```bash
./tools/run-atspi-e2e.sh
```

Этот скрипт сам изолирует Xvfb, D-Bus и runtime-каталог доступности. Для него
не нужен дополнительный внешний `dbus-run-session`. Виртуальный редактор
`evaluate_context_engine.py` проверяет итоговые строки, но не заменяет
Windows/X11/AT-SPI E2E.

## Базовая KSLM и сборка Linux

Правила нового кандидата, необратимая граница seal и два retraining replay
описаны в [runbook](intent-model-runbook.md) и
[cookbook](intent-model-cookbook.md). У их trainer `--dry-run` **не гарантирует
отсутствие записи seal**; не путайте его с `release.py --dry-run`.

Для strict-оценки текущей KSLM в reference-среде и передачи отчёта в упаковку:

```bash
set -euo pipefail
verification_dir="$(mktemp -d /tmp/keyswitch-verification.XXXXXXXX)"
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
PYTHONPATH=src python3 tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
PYTHONPATH=src python3 tools/evaluate_intent_model.py --strict \
  > "$verification_dir/intent-strict.json"
python3 tools/verify_intent_strict_report.py \
  --report "$verification_dir/intent-strict.json"

./tools/install-build-tools.sh .nuitka
KEYSWITCH_NUITKA_ROOT=.nuitka \
  KEYSWITCH_INTENT_STRICT_REPORT="$verification_dir/intent-strict.json" \
  ./packaging/build-deb.sh

package_version="$(sed -nE 's/^version = "([^"]+)"/\1/p' pyproject.toml | head -n 1)"
package="dist/keyswitch_${package_version}_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
desktop-file-validate packaging/io.github.olegius88.KeySwitch.desktop
lintian --fail-on error "$package"

./tools/run-gui-test.sh --noreset -- \
  bash -c 'setxkbmap -layout us,ru && GTK_USE_PORTAL=0 ./tools/run-native-e2e.sh "$1"' \
  _ "$package"
```

Сохраните нужные отчёты из `$verification_dir` до очистки временных файлов.
Переданный strict-отчёт принимается только после сверки gates и текущих hashes.
Без переменной `KEYSWITCH_INTENT_STRICT_REPORT` `build-deb.sh` запускает strict
самостоятельно; с неправильным отчётом сборка падает, не обходит проверку.
Обе нативные сборки также запускают быстрые проверки context-v1, context-v2,
boundary-v1, boundary-v2 и prefix-v1 (включая запрет установки отклонённых
весов) и сверяют рабочие контекстный, префиксный и граничный артефакты
после компиляции.

Windows собирается на Windows через `packaging/build-windows.ps1`; зависимости,
настройка EN/RU, unit/native E2E и silent-install smoke перечислены в job
`windows` [Tests workflow](../.github/workflows/tests.yml). Смена языкового списка
из CI влияет на пользовательские настройки ОС: не копируйте этот шаг в личный
сеанс без проверки. Windows-результат нельзя объявлять проверенным локальным Linux-прогоном.

## Что объединяет `release_pipeline.py`

| Профиль | Состав |
| --- | --- |
| `quick` | Среда, входы KSLM, strict typing, coverage, detector gates, release metadata |
| `app` | `quick` + strict KSLM, X11/tray E2E, DEB build/verifier/Lintian и packaged E2E |
| `release` | `app` + replay development/preseal и два retraining KSLM с побайтным сравнением; strict replay-a включается `--replay-strict` |

```bash
python3 tools/release_pipeline.py phases
python3 tools/release_pipeline.py start --profile app
python3 tools/release_pipeline.py status
python3 tools/release_pipeline.py wait
```

`start` отсоединяет процесс; `run` выполняет тот же выбранный профиль на
переднем плане. Итоги — `dist/release-pipeline/latest/SUMMARY.md` и `summary.json`;
`state.json` и журналы фаз показывают ход работы. `--jobs` и
`--memory-reserve-mib` ограничивают планировщик, но оценка пиков памяти не
гарантирует отсутствие OOM при любой внешней нагрузке. Strict latency-фазы
выполняются отдельно от остальных фаз этого конвейера.

Явный `run --resume <каталог-прогона>` повторно использует успешные фазы
внутри того же каталога. Это отдельный механизм конвейера, не автоматическое
восстановление `release.py`. Используйте его только при неизменных проверяемых
входах; после изменения кода или данных запускайте новый прогон.

**Это не весь CI:** отдельные contextual retraining, corpus/engine replay и
нативный AT-SPI E2E не являются фазами конвейера; выполняйте их командами выше.
Windows-проверки также идут отдельным job. Полный профиль `release` доказывает
базовую KSLM, а не автоматически сертифицирует новую контекстную политику.
Источник состава — [PROFILES/PHASES](../tools/release_pipeline.py).

## Публикация и восстановление

1. Выберите новую версию `X.Y.Z`, проверьте ветку, `git status` и полный diff.
   Скрипт по умолчанию требует `main` и использует `origin`; `git add -A`
   включает **все** изменённые и неигнорируемые новые файлы, не только код агента.
2. Напишите `CHANGELOG.md` под `## Unreleased` (или раздел выбранной версии)
   и `RELEASE_NOTES.md` с заголовком и точными именами DEB, Setup EXE и ZIP.
   Версии в обслуживаемых файлах обновляет `version_sites()` в `release.py`;
   исторические упоминания проверяйте по diff, не считайте замену смысловой.
3. Выполните предварительную проверку. `X.Y.Z` ниже — заполнитель, не готовый номер:

   ```bash
   python3 tools/release.py --version X.Y.Z --dry-run
   ```

4. Только для согласованного выпуска:

   ```bash
   python3 tools/release.py --version X.Y.Z
   ```

Обычный запуск меняет файлы, выполняет выбранный профиль (по умолчанию
`release`), коммитит, создаёт тег, пушит, ждёт release workflow и проверяет
наличие четырёх опубликованных assets. Требуются `gh`, действующая
аутентификация и права push. `--dry-run` не пишет файлы, но проверяет подготовку,
ветку и предусловия; тесты, сборка и публикация не выполняются.

После сбоя прочитайте названную фазу/лог и проверьте состояние Git и Actions.
Повторный запуск обычно повторяет локальные проверки, а не продолжает
последнюю фазу. Если тег уже существует, требуется тот же HEAD и чистое дерево;
для исправленного кода нужна новая версия. Сбой CI не отменяет уже сделанный
push; повтор `release.py` сам не перезапускает проваленный Actions run.
Нужен осознанный повтор подходящего workflow либо новый выпуск после исправления.

`--skip-pipeline` просто пропускает локальный конвейер: он не проверяет и не
выбирает прошлый успешный прогон. `--skip-ci` останавливается после push и
**не доказывает публикацию**. Не выдавайте эти режимы за завершённый полный
релиз. Финальный шаг скрипта сверяет имена assets, а не скачивает и повторно
проверяет их содержимое; сборочные проверки и `SHA256SUMS` формируются workflow.
Источник поведения — [release.py](../tools/release.py) и
[Release workflow](../.github/workflows/release.yml).

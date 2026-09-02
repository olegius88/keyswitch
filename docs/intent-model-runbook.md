# Runbook: обучение и выпуск Layout Intent

Этот документ описывает управляемый выпуск собственной линейной n-граммной
модели KeySwitch. Он отвечает на вопрос «что и в каком порядке делать, чтобы
получить модель, которой разрешено попасть в релиз». Готовые команды и
варианты для повседневной работы вынесены в
[cookbook](intent-model-cookbook.md), устройство модели — в
[карточку модели](../model/intent_v1/MODEL_CARD.md).

Runbook соответствует текущему контуру v15:

- training config schema 13;
- feature schema v5;
- KSLM container schema 4;
- sealed split namespace `keyswitch:intent-v15:physical-signature`;
- reference host Ubuntu 26.04 и Python 3.14;
- сертифицированный artifact `intent-v1-bec1f1d3dceb`.

Номера этих схем независимы. Нельзя автоматически повышать их вместе только
ради нового релиза.

## Результат процесса

Успешный выпуск должен оставить проверяемый набор:

| Артефакт | Назначение |
| --- | --- |
| `model/intent_v1/config.json` | Полная конфигурация и hashes входов |
| `model/intent_v1/sources/*` | Побайтно замороженные EN/RU Onboard-модели и license evidence |
| `unknown-typo-development-vN.json` | Model-blind hard-negative development source |
| `holdout-vN-preseal.json` | Доказательство создания holdout до загрузки KSLM и оценки метрик |
| `seal-registry-vN.json` | Неизменяемое закрепление одного sealed namespace за одним кандидатом |
| `layout_intent_v1.ksm` | Runtime-модель |
| `manifest.json` | Полный provenance и внутренние sealed-метрики; commit marker публикации |
| `test-report.json` | Компактный внутренний отчёт качества |
| strict report | Независимая внешняя, runtime- и performance-проверка |
| `rejection-vN.json` | Неизменяемое объяснение отказа, если кандидат не принят |

Канонический поток выглядит так:

```text
frozen sources
    -> model-blind development freeze
    -> model-blind holdout preseal
    -> train / calibrate / select thresholds
    -> atomic candidate seal claim
    -> internal sealed test
    -> atomic artifact publication
    -> independent strict evaluation
    -> two byte-identical replays
    -> application tests and native packages
```

## Непереговорные правила

1. Единица разделения — физическая последовательность клавиш, а не строка,
   слово или язык. Все варианты одной последовательности принадлежат одному
   split.
2. Train, development, calibration и threshold доступны до seal. Test строится
   только после успешного pre-sealed gate и atomically claimed candidate SHA.
3. Независимый external holdout создаётся model-blind. В preseal receipt обязаны
   остаться `model_loaded=false`, `metrics_evaluated=false` и нулевые overlap.
4. После просмотра sealed или external holdout результата его нельзя
   использовать как доказательство следующего релиза. Результат можно считать
   development evidence, но следующий кандидат обязан получить новые
   split/registry/holdout namespaces.
5. `seal-registry-vN.json` нельзя удалять, заменять или редактировать. Он
   допускает только точный повтор уже закреплённого кандидата.
6. `--dry-run` не является предварительным просмотром: если кандидат проходит
   pre-sealed gate, команда всё равно захватывает seal до sealed scoring.
7. `--diagnostic-output` безопасно сохраняет подробности только когда
   pre-sealed gate уже провален. Если gate пройден, trainer продолжит работу и
   захватит seal. Безопасного preview для проходящего кандидата нет.
8. Нельзя подбирать эпоху, калибровку, пороги или margin по test/holdout.
9. Нельзя глобально обрезать лексику до split. Значение
   `maximum_words_per_language` обязано оставаться нулём.
10. Артефакт принимается только после внутренних gates, независимого
    `--strict`, двух воспроизводимых retraining и полного release-контура.

## 1. Подготовить эталонную среду

Reference environment закреплён workflow
[`tests.yml`](../.github/workflows/tests.yml). Минимум для формирования
model-blind корпусов и обучения:

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  hunspell-en-us hunspell-ru libhunspell-1.7-0 onboard-data \
  python3 python3-pip
```

Для полного Linux release-контура дополнительно нужны пакеты из job `verify`:

```bash
sudo apt-get install --yes --no-install-recommends \
  at-spi2-core build-essential ccache dbus-x11 desktop-file-utils file \
  gir1.2-adw-1 gir1.2-atspi-2.0 gir1.2-gtk-4.0 libglib2.0-bin \
  libx11-6 libxkbcommon0 libxtst6 lintian patch patchelf \
  python3-coverage python3-dbus python3-dev python3-gi \
  x11-xkb-utils xauth xvfb
```

Зафиксировать фактическую среду в журнале выпуска:

```bash
python3 --version
lsb_release -ds
dpkg-query -W -f='${Package} ${Version}\n' \
  onboard-data hunspell-en-us hunspell-ru libhunspell-1.7-0
```

Версия Python, build string, ОС, архитектура, libc и byte order входят в
candidate provenance. Побайтная воспроизводимость обещана только в той же
зафиксированной среде.

`evaluate_intent_model.py --strict` и `--provenance-only` относятся к
сертификационному контуру reference host. Нативная упаковка для другой ОС не
должна заново строить corpus в иной platform identity. Она обязана проверить
SHA-256 KSLM и frozen inputs, неизменяемый seal registry, hashes model
toolchain, полный подписанный manifest и quality evidence внутри KSLM. Общий
release workflow остаётся зелёным только после отдельного полного `--strict`
на reference host.

## 2. Выбрать режим: replay или новый кандидат

### Точный replay принятого кандидата

Replay разрешён, когда неизменны все входы candidate identity:

- config и frozen sources;
- trainer, evaluator, preseal generator, development freezer;
- intent runtime, layouts, language model, detector и protected tokens;
- frozen development source и preseal receipt;
- Python/platform identity;
- параметры и строки pre-sealed dataset.

Существующий registry разрешит повтор только при полном совпадении canonical
candidate record. Replay всегда пишет в отдельные временные выходы и не должен
перетирать официальный artifact.

### Новый кандидат

Новый кандидат нужен после изменения модели, данных, признаков, policy,
порогов, исполняемых provenance-входов или среды. До любого обучения нужно
выбрать новый номер `vN` и как минимум совместно ротировать:

- `SPLIT_NAMESPACE`;
- `SEALED_REGISTRY_RELATIVE_PATH`;
- `UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE`;
- `UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE`;
- `HARD_NEGATIVE_ROLE_NAMESPACE`;
- `HARD_NEGATIVE_SOURCE_RELATIVE_PATH`;
- `PRESEAL_RECEIPT_PATH`;
- `_POLICY` в development freezer;
- соответствующие пути/namespaces в config, packaging-проверках и тестах.

Если требуется новый model-blind development domain, отдельно ротируются
`UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE` и
`UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE`. Уже раскрытый holdout не становится
новым release holdout.

Корневой `schema_version` config повышается только при изменении контракта или
семантики config. Тогда одновременно обновляются строгий validator, packaging
checks и тесты. Feature schema и KSLM schema повышаются только при изменении
их формата/семантики.

Перед правками получить полный список version-bound мест:

```bash
rg -n 'v15|schema_version|SPLIT_NAMESPACE|SEALED_REGISTRY|PRESEAL_RECEIPT|UNKNOWN_TYPO|HARD_NEGATIVE' \
  tools model packaging tests README.md README.en.md DESIGN.md docs
```

Новая registry-запись и новый preseal receipt до соответствующих фаз не
создаются вручную.

## 3. Проверить или обновить frozen lexical sources

Если EN/RU Onboard sources не меняются, достаточно проверки:

```bash
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
```

Если source package намеренно обновляется:

1. Скопировать точные `/usr/share/onboard/models/en_US.lm` и `ru_RU.lm`.
2. Побайтно скопировать `/usr/share/doc/onboard-data/copyright`.
3. Пересоздать `SHA256SUMS` и проверить его.
4. Обновить в config package version, каждый `sha256` и `bytes`.
5. Проверить license stanza `Files: models/*`; не заменять её собственным
   толкованием лицензии.
6. Считать это изменением данных и выпускать только как нового кандидата.

Source-файлы не должны зависеть от последующего обновления системного пакета.
Trainer читает repository copies, а strict evaluator отдельно проверяет
зафиксированный системный Hunspell snapshot.

## 4. Заморозить hard-negative development corpus

Эта фаза выполняется до обучения и без загрузки KSLM:

```bash
PYTHONPATH=src python3 tools/freeze_intent_development_corpus.py \
  --config model/intent_v1/config.json \
  --en-model model/intent_v1/sources/en_US.lm \
  --ru-model model/intent_v1/sources/ru_RU.lm \
  --output model/intent_v1/unknown-typo-development-vN.json
```

Команда атомарно публикует компактный JSON и печатает:

- размер файла;
- SHA-256 файла;
- SHA-256 развёрнутого корпуса;
- число физических сигнатур.

Затем обновить в config:

- `hard_negative_development.source.path`;
- `hard_negative_development.source.sha256`;
- `hard_negative_development.source.bytes`;
- `external_evaluation.unknown_typo_development_corpus_sha256`.

Повторный запуск в отдельный путь обязан дать побайтно тот же файл. Нельзя
изменять frozen source вручную.

## 5. Создать и зафиксировать holdout до модели

После фиксации development source, но до train, запустить:

```bash
PYTHONPATH=src python3 tools/preseal_intent_holdout.py \
  --config model/intent_v1/config.json \
  --en-model model/intent_v1/sources/en_US.lm \
  --ru-model model/intent_v1/sources/ru_RU.lm \
  > model/intent_v1/holdout-vN-preseal.json
```

Из receipt перенести `holdout.corpus_sha256` в
`external_evaluation.unknown_typo_holdout_corpus_sha256`. Затем повторить
генерацию во временный файл и потребовать точный `diff` с сохранённым receipt.

Обязательные проверки:

```bash
jq -e '
  .model_loaded == false and
  .metrics_evaluated == false and
  .development.signature_count > 0 and
  .holdout.signature_count > 0 and
  .overlap_counts.development_holdout == 0 and
  .overlap_counts.sealed_holdout == 0
' model/intent_v1/holdout-vN-preseal.json
```

Если хотя бы одно условие ложно, выпуск останавливается до обучения. Нельзя
«исправлять» receipt вручную.

## 6. Заморозить candidate inputs

До официального train:

1. Завершить изменения trainer/runtime/evaluator/config.
2. Прогнать targeted tests и typecheck.
3. Проверить frozen sources и воспроизводимость preseal.
4. Сохранить diff и SHA-256 всех candidate inputs в журнале выпуска.
5. Не менять эти файлы до завершения strict evaluation и replays.

Минимальный контроль:

```bash
git diff --check
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
PYTHONPATH=src python3 tools/preseal_intent_holdout.py > /tmp/keyswitch-preseal-check.json
diff -u model/intent_v1/holdout-vN-preseal.json /tmp/keyswitch-preseal-check.json
```

Для реальной работы используйте task-scoped каталог из cookbook, а не
фиксированный путь `/tmp/keyswitch-preseal-check.json`.

## 7. Выполнить официальный train

Команда выпуска:

```bash
set -o pipefail
PYTHONPATH=src python3 tools/train_intent_model_release.py --workers 0 | \
  tee /tmp/keyswitch-train-manifest.json
```

`--workers 0` — значение по умолчанию: trainer использует все logical CPU,
доступные через affinity процесса. `--workers N` задаёт верхнюю границу, а
`--workers 1` нужен для диагностики. Process pool применяется только к
независимым вычислениям и возвращает строки в каноническом порядке; online
FTRL update остаётся последовательным и сохраняет точную математику принятого
алгоритма. Число worker-ов не записывается в candidate identity и не должно
менять artifact/manifest/report bytes.

Trainer выполняет последовательно:

1. Проверку config, frozen sources и toolchain snapshot.
2. Построение только train/development/calibration/threshold.
3. Обучение FTRL-Proximal и выбор эпохи по development.
4. Int16 quantization и обучение directional Platt calibration только на
   calibration split.
5. Выбор trigger/direction thresholds и общего margin только на threshold.
6. Safety, veto, context-stress и пробную KSLM serialization до seal.
7. Атомарный claim candidate SHA в новом registry.
8. Построение и оценку внутреннего sealed test.
9. Публикацию `test-report -> artifact -> manifest`; manifest заменяется
   последним как commit marker.

Интерпретация результата:

- exit `1` до registry: pre-sealed кандидат не прошёл, sealed test не открыт;
- появившийся registry означает, что namespace уже израсходован, даже если
  последующая проверка или публикация завершилась ошибкой;
- exit `0` и `quality_gates_passed=true` разрешают перейти к независимому
  strict evaluator, но ещё не разрешают релиз приложения.

## 8. Проверить внутренний результат

```bash
jq -e '.quality_gates_passed == true' model/intent_v1/manifest.json
jq -e '.quality_gates_passed == true' model/intent_v1/test-report.json
jq -e '.sealed_evaluation.candidate_sha256 == $candidate' \
  --arg candidate "$(jq -r .candidate_sha256 model/intent_v1/seal-registry-vN.json)" \
  model/intent_v1/manifest.json
sha256sum src/keyswitch/resources/models/layout_intent_v1.ksm \
  model/intent_v1/manifest.json model/intent_v1/test-report.json \
  model/intent_v1/seal-registry-vN.json
```

Проверить также, что `artifact_sha256` из manifest совпадает с фактическим
файлом и что model version начинается с `intent-v1-` плюс первые 12 символов
`build_provenance_sha256`.

## 9. Выполнить независимую strict evaluation

```bash
mkdir -p dist
PYTHONPATH=src python3 tools/evaluate_intent_model.py --strict \
  > dist/keyswitch-intent-evaluation.json
jq -e '
  .strict_passed == true and
  all(.strict_gates[]; . == true)
' dist/keyswitch-intent-evaluation.json
jq '{strict_passed, gate_count: (.strict_gates | length), performance}' \
  dist/keyswitch-intent-evaluation.json
```

Strict evaluator независимо проверяет provenance, пересобирает internal sealed
evidence, открывает зафиксированный external holdout, запускает production
detector/context/safety проверки и измеряет load/inference latency.

Если strict gate провален:

1. Artifact не выпускается пользователям, даже если trainer успел локально его
   опубликовать.
2. Сохраняются registry, strict report и точные hashes.
3. Создаётся `rejection-vN.json` по образцу существующих rejection receipts.
4. Результат объявляется development evidence.
5. Для исправления создаётся новый кандидат с новой ротацией; тот же holdout
   повторно release evidence не является.

## 10. Доказать воспроизводимость

В той же среде выполнить два полных retraining в независимые каталоги. Registry
разрешит их только при точном совпадении кандидата:

```bash
retrain_root="$(mktemp -d /tmp/keyswitch-intent-retrain.XXXXXX)"
for run in a b; do
  mkdir "$retrain_root/$run"
  PYTHONPATH=src python3 tools/train_intent_model_release.py \
    --artifact "$retrain_root/$run/layout_intent_v1.ksm" \
    --manifest "$retrain_root/$run/manifest.json" \
    --test-report "$retrain_root/$run/test-report.json"
done

for file in layout_intent_v1.ksm manifest.json test-report.json; do
  cmp "${retrain_root}/a/${file}" "${retrain_root}/b/${file}"
done
cmp src/keyswitch/resources/models/layout_intent_v1.ksm \
  "$retrain_root/a/layout_intent_v1.ksm"
cmp model/intent_v1/manifest.json "$retrain_root/a/manifest.json"
cmp model/intent_v1/test-report.json "$retrain_root/a/test-report.json"
```

После `cmp` запустить strict evaluator хотя бы для одного replay. Без
трёхстороннего побайтного равенства official/replay-a/replay-b кандидат не
принимается.

## 11. Проверить приложение и пакеты

Полный Linux quality contour:

```bash
./tools/install-typing-tools.sh .typing
KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh

dbus-run-session -- \
  xvfb-run -a -s "-screen 0 1280x800x24" \
  ./tests/run_coverage.sh

PYTHONPATH=src python3 tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict

dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24 -noreset" \
  bash -c 'setxkbmap -layout us,ru && GTK_A11Y=none PYTHONPATH=src python3 tests/e2e_x11.py'

dbus-run-session -- env PYTHONPATH=src python3 tests/e2e_tray_menu.py
```

Сборка и проверка DEB:

```bash
./tools/install-build-tools.sh .nuitka
KEYSWITCH_NUITKA_ROOT=.nuitka ./packaging/build-deb.sh

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' pyproject.toml | head -n 1)"
package="dist/keyswitch_${version}_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
desktop-file-validate packaging/io.github.olegius88.KeySwitch.desktop
lintian --fail-on error "$package"

dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24 -noreset" \
  bash -c 'setxkbmap -layout us,ru && GTK_USE_PORTAL=0 ./tools/run-native-e2e.sh "$1"' \
  _ "$package"
```

`build-deb.sh` повторно запускает strict evaluator и не собирает пакет при
провале модели. Verifier требует KSLM schema 4, размерные bounds, побайтное
совпадение packaged artifact/frozen EN/RU sources и отсутствие зависимости DEB
от системного Python interpreter.

Windows release выполняется workflow `windows` из
[`tests.yml`](../.github/workflows/tests.yml): strict mypy/tests, настоящий
`WH_KEYBOARD_LL`/`SendInput` E2E, native ZIP/installer, silent install и
диагностика установленной модели.

### Единый прогон

Проверки разделов 3, 5, 8–11 целиком выполняет `tools/release_pipeline.py`.
Профиль `release` проверяет frozen sources и provenance, побайтно
воспроизводит development corpus и preseal receipt, запускает независимый
strict evaluator, два retraining replay с трёхсторонним сравнением (и strict
для replay «a» при `--replay-strict`), затем typecheck, coverage, detector,
X11/tray E2E, DEB, verifier, Lintian и packaged E2E. Независимые фазы идут
параллельно, а планировщик допускает их только при достаточном свободном ОЗУ:

```bash
python3 tools/release_pipeline.py start --profile release --replay-strict
python3 tools/release_pipeline.py wait
```

Итог — `dist/release-pipeline/latest/SUMMARY.md` с чек-листом раздела 12,
hashes всех артефактов и хвостами журналов проваленных фаз. Скрипт не
выполняет официальный train и не трогает registry: новый кандидат по разделам
4–7 создаётся вручную, а pipeline доказывает уже опубликованный артефакт. Уже
запущенные вручную replay передаются через `--replay-dir`. Фаза
`release-metadata` в профиле `release` требует, чтобы версия, changelog, model
card и `.gitattributes` уже соответствовали кандидату.

## 12. Решение о выпуске

Кандидат принимается только если отмечены все пункты:

- [ ] Frozen source checksums и license evidence совпали.
- [ ] Development corpus воспроизводится побайтно.
- [ ] Preseal receipt создан до model load и совпадает побайтно при повторе.
- [ ] Оба overlap-счётчика равны нулю.
- [ ] Использованы новые namespace/registry для изменённого кандидата.
- [ ] Trainer завершился с exit `0` и internal gates прошли.
- [ ] Registry, manifest и test-report согласованы.
- [ ] Независимый strict report прошёл каждый gate.
- [ ] Official/replay-a/replay-b побайтно совпали.
- [ ] Strict typing и 100% line/branch coverage прошли.
- [ ] Detector, X11, tray и packaged native E2E прошли.
- [ ] DEB/Windows artifacts прошли свои verifier/smoke tests.
- [ ] Model card, changelog и hashes обновлены фактическими результатами.

## Сбои и восстановление

| Сбой | Что означает | Разрешённое действие |
| --- | --- | --- |
| Frozen source hash mismatch | Входы уже не те, что объявлены | Восстановить точные bytes либо оформить новый source snapshot и нового кандидата |
| Preseal overlap не ноль | Утечка между development/holdout/sealed | Остановиться, исправить генератор/policy, заново ротировать ещё не раскрытые namespaces |
| Pre-sealed gate fail, registry отсутствует | Test не открыт | Анализировать diagnostic, менять кандидата; перед следующей попыткой всё равно проверить отсутствие claim |
| Registry существует и candidate отличается | Namespace уже занят | Не удалять registry; ротировать split и registry |
| Internal sealed gate fail | Sealed test раскрыт | Сохранить rejection evidence и перейти к новому кандидату |
| Strict external gate fail | External holdout раскрыт | Не выпускать; сохранить rejection и создать новый holdout namespace |
| Toolchain/provenance mismatch | Код или среда изменились после train | Восстановить точную среду для replay либо создать нового кандидата |
| Replay `cmp` fail | Нет byte reproducibility | Не выпускать; найти недетерминированный вход и повторить с новым кандидатом, если identity меняется |
| Ошибка bundle publication | Trainer пытается восстановить прежние bytes | Проверить все три destination и registry; не считать частичный набор релизом |
| Packaging/E2E fail | Модель не доказана в поставляемом runtime | Исправить пакет/runtime и повторить весь затронутый контур; при изменении candidate identity — новый кандидат |

Rollback опубликованного приложения делается возвратом к последнему принятому
DEB/installer вместе с его KSLM, frozen language sources и metadata. Нельзя
подменять только `.ksm` без соответствующих manifest/report/provenance.

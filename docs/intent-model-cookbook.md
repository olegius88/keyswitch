# Cookbook: как создать и обучить Layout Intent

Это практическая пошаговая инструкция с готовыми командами. Для нового
production-кандидата сначала прочитайте обязательные правила и точки отказа в
[runbook](intent-model-runbook.md). Формулы, признаки, split policy и полные
метрики описаны в [MODEL_CARD.md](../model/intent_v1/MODEL_CARD.md).

Все команды ниже выполняются из корня репозитория KeySwitch.

## Что именно создаёт trainer

Перед командами полезно понимать всю конструкцию модели:

1. Из frozen `en_US.lm` и `ru_RU.lm` читаются unigram-слова и частоты.
2. Каждое слово переводится в физическую последовательность клавиш US/RU.
3. Одинаковые EN/RU-последовательности изолируются как collision/safety, а не
   превращаются в обучающие примеры.
4. Для каждой допустимой последовательности строится симметричная пара:
   правильная интерпретация — label `false`, неправильная — label `true`.
5. Обе стороны симметрично дополняются deletion, duplication и adjacent-swap
   опечатками для каждого из шести runtime trigger.
6. Hash физической последовательности заранее назначает весь набор одному из
   train/development/calibration/threshold/test split.
7. Отдельный model-blind Hunspell unknown-typo development source добавляет
   сложные отрицательные и положительные пары только в pre-sealed роли.
8. На train обучается sparse logistic regression FTRL-Proximal; development
   выбирает эпоху, calibration — две directional Platt sigmoid, threshold —
   пороги и общий margin для каждого trigger.
9. Веса квантуются в signed int16. Runtime feature membership сохраняется
   отдельными точными uint64 fingerprints.
10. После pre-sealed gates кандидат атомарно получает seal, проходит внутренний
    test и сериализуется в ограниченный KSLM schema 4 container.
11. Независимый strict evaluator впервые применяет production detector к
    external holdout, safety/context-профилям и измеряет latency.

Обычный пользовательский ввод в этот offline pipeline не попадает. Явные
локальные пользовательские правила остаются отдельным детерминированным слоем
приложения и не меняют общий KSLM artifact.

## 0. Подготовить безопасный рабочий каталог

Не складывайте промежуточные artifact/manifest/report в официальные пути.
Создайте отдельный task-scoped каталог:

```bash
set -euo pipefail

project_root="$(pwd -P)"
test -f "$project_root/pyproject.toml"
test -f "$project_root/tools/train_intent_model.py"

work_root="$(mktemp -d /tmp/keyswitch-intent-cookbook.XXXXXXXX)"
work_identity="$(stat --format='%d:%i:%u' -- "$work_root")"
readonly project_root work_root work_identity

cleanup_work_root() {
  local resolved current_identity
  test -n "$work_root"
  [[ "$work_root" == /tmp/keyswitch-intent-cookbook.* ]]
  test -d "$work_root"
  test ! -L "$work_root"
  resolved="$(realpath --canonicalize-existing -- "$work_root")"
  test "$resolved" = "$work_root"
  current_identity="$(stat --format='%d:%i:%u' -- "$work_root")"
  test "$current_identity" = "$work_identity"
  rm -r -- "$work_root"
}
trap cleanup_work_root EXIT
```

Cleanup удаляет только точный созданный здесь каталог, проверяет путь, symlink
и inode identity и не использует рекурсивный force. Чтобы сохранить
диагностику, перед завершением shell скопируйте нужные файлы из `$work_root` в
явно выбранный постоянный каталог и выполните `trap - EXIT`.

## 1. Установить зависимости

Минимальная reference-среда обучения:

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  hunspell-en-us hunspell-ru libhunspell-1.7-0 \
  onboard-data python3 python3-pip jq
```

Сверить фактическую среду:

```bash
python3 --version
lsb_release -ds
dpkg-query -W -f='${Package} ${Version}\n' \
  onboard-data hunspell-en-us hunspell-ru libhunspell-1.7-0
```

### Использовать все доступные CPU

Trainer по умолчанию эквивалентен `--workers 0`: он читает CPU affinity
процесса и запускает по одному worker-процессу на каждый доступный logical
CPU. На Linux это позволяет обойти GIL в самых дорогих независимых фазах:
извлечении признаков и scoring development/calibration/threshold/test/safety.
Порядок строк после каждого process pool восстанавливается канонически, поэтому
`--workers 1`, `--workers 4` и `--workers 0` должны давать побайтно одинаковый
кандидат в одной и той же reference-среде.

Проверить доступный параллелизм до длительного запуска:

```bash
python3 - <<'PY'
from __future__ import annotations

import os

available = (
    len(os.sched_getaffinity(0))
    if hasattr(os, "sched_getaffinity")
    else (os.cpu_count() or 1)
)
print(f"available logical CPUs: {available}")
PY
```

Обычный запуск не требует флага. Для ограничения RAM/CPU укажите, например,
`--workers 4`; для воспроизведения ошибки без multiprocessing —
`--workers 1`. Progress и длительность фаз пишутся в stderr, а JSON в stdout
остаётся машинно-читаемым.

Online update FTRL-Proximal намеренно не делится между worker-ами: состояние
после каждого примера является входом следующего шага. Недетерминированный
Hogwild update изменил бы модель и нарушил byte-identical replay. Поэтому во
время части `phase=ftrl` возможны последовательные участки, но извлечение
признаков, development scoring каждой эпохи и все большие fixed-model scoring
фазы используют настроенный process pool.

Зафиксировать именно те Hunspell-файлы, которые обнаруживает production code:

```bash
PYTHONPATH=src python3 - <<'PY'
from __future__ import annotations

import hashlib
from pathlib import Path

from keyswitch.spellcheck import HunspellDictionary

for locale in ("en_US", "ru_RU"):
    dictionary = HunspellDictionary(locale)
    if not dictionary.available:
        raise SystemExit(f"Hunspell is unavailable for {locale}")
    dictionary_path = Path(dictionary.source)
    affix_path = dictionary_path.with_suffix(".aff")
    for role, path in (("dictionary", dictionary_path), ("affix", affix_path)):
        payload = path.read_bytes()
        print(locale, role, path, len(payload), hashlib.sha256(payload).hexdigest())
    dictionary.close()
PY
```

Paths, sizes и hashes должны совпасть с
`external_evaluation.hunspell` в config. Учитывайте активный
`KEYSWITCH_HUNSPELL_PATH`: он имеет приоритет над системными каталогами. Если
Hunspell snapshot намеренно меняется, сначала обновите эти четыре поля каждого
языка, затем заново создайте development source и holdout как нового кандидата.

На reference host v15 это Ubuntu 26.04.1 LTS, Python 3.14.4,
`onboard-data 1.4.3+git20260213+ds-2`. Для воспроизводимого replay используйте
ту же Python/platform identity из `manifest.json`, а не только совместимую
версию Python.

## 2. Проверить текущую v15 без переобучения

### Frozen sources

```bash
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
```

### Объявленные и фактические hashes

```bash
sha256sum \
  model/intent_v1/config.json \
  model/intent_v1/unknown-typo-development-v15.json \
  model/intent_v1/holdout-v15-preseal.json \
  model/intent_v1/seal-registry-v15.json \
  model/intent_v1/manifest.json \
  model/intent_v1/test-report.json \
  src/keyswitch/resources/models/layout_intent_v1.ksm
```

Ожидаемый v15 baseline:

| Файл | SHA-256 |
| --- | --- |
| `config.json` | `06fa899534c8e6e0d3984d2ff7e22b46fe9721efde7df4cf02c53b6967d55127` |
| `unknown-typo-development-v15.json` | `a0585bdbd21526434fc77effc64200075269d884321a702fa44bd8a9dc7f963c` |
| `holdout-v15-preseal.json` | `56ceb18efe4e1cdd4372e3cf1eb0d7c01f7d1320075a3961bf9f31ff06a34d1d` |
| `seal-registry-v15.json` | `5dc4d01b59d7f614f1178ab8ca14b7f701ed09f5551c118cdb6328c5d7e512a4` |
| `manifest.json` | `e0070e8e6813da4a8dde1a09eb2c1713f033d002a64216299cba3764032d82f7` |
| `test-report.json` | `05caf3828ff2724fc5f1d22ff2e28d9b31cd2d1bcfcceb64f934bb8bfe84480d` |
| `layout_intent_v1.ksm` | `7631b821bafc958364353a8a13de3abc23e922e51b589bd181075db55fa9e9dc` |

### Internal provenance без внешнего performance-прогона

```bash
PYTHONPATH=src python3 tools/evaluate_intent_model.py \
  --provenance-only > "$work_root/provenance.json"
jq -e '
  .phase == "internal_sealed_evidence" and
  .internal_sealed_evidence_passed == true
' "$work_root/provenance.json"
```

`--provenance-only` проверяет уже созданный кандидат. Эта команда не создаёт и
не захватывает registry, но пересобирает внутреннее sealed evidence, поэтому
она остаётся длительной CPU-проверкой и не заменяет model-blind preseal для
нового кандидата.

Выполняйте этот replay только в reference environment из manifest. Для
Windows/macOS consumer package не пересобирайте corpus: проверяйте точные
artifact/config/source/registry/toolchain hashes и встроенный signed manifest,
а полный `--strict` оставляйте обязательным отдельным job на reference host.

## 3. Воспроизвести v15 preseal без доступа к KSLM

```bash
PYTHONPATH=src python3 tools/preseal_intent_holdout.py \
  --config model/intent_v1/config.json \
  --en-model model/intent_v1/sources/en_US.lm \
  --ru-model model/intent_v1/sources/ru_RU.lm \
  > "$work_root/holdout-v15-preseal.json"

diff -u model/intent_v1/holdout-v15-preseal.json \
  "$work_root/holdout-v15-preseal.json"

jq -e '
  .model_loaded == false and
  .metrics_evaluated == false and
  .development.signature_count == 10000 and
  .holdout.signature_count == 10000 and
  .overlap_counts.development_holdout == 0 and
  .overlap_counts.sealed_holdout == 0
' "$work_root/holdout-v15-preseal.json"
```

Это безопасная проверка: `preseal_intent_holdout.py` не принимает путь к KSLM,
не загружает intent artifact и не считает метрики модели.

## 4. Сделать точный retraining replay v15

Этот рецепт занимает заметное CPU-время. Он разрешён существующим registry
только потому, что ожидается тот же canonical candidate SHA.
Trainer входит в candidate identity: после изменения `tools/train_intent_model.py`
(например, multiprocessing backend в v15) прежний кандидат больше не
воспроизводим из текущего дерева. Точный v14 replay запускается только из
чистого тега `v0.6.1`, чей trainer SHA совпадает с его manifest; текущий v15
registry разрешает replay только текущего trainer. Любое новое изменение
trainer до длительного train требует ротации по разделу 6.

```bash
mkdir "$work_root/replay"

PYTHONPATH=src python3 tools/train_intent_model_release.py \
  --workers 0 \
  --artifact "$work_root/replay/layout_intent_v1.ksm" \
  --manifest "$work_root/replay/manifest.json" \
  --test-report "$work_root/replay/test-report.json" \
  > "$work_root/replay/stdout-manifest.json"

cmp src/keyswitch/resources/models/layout_intent_v1.ksm \
  "$work_root/replay/layout_intent_v1.ksm"
cmp model/intent_v1/manifest.json "$work_root/replay/manifest.json"
cmp model/intent_v1/test-report.json "$work_root/replay/test-report.json"

jq -e '.quality_gates_passed == true' \
  "$work_root/replay/manifest.json"
```

Если trainer сообщает, что namespace уже занят другим кандидатом, не удаляйте
registry: текущие inputs отличаются от v15. Это уже новый кандидат и требует
ротации.

## 5. Запустить полный strict report для текущей модели

```bash
PYTHONPATH=src python3 tools/evaluate_intent_model.py \
  --artifact src/keyswitch/resources/models/layout_intent_v1.ksm \
  --manifest model/intent_v1/manifest.json \
  --strict > "$work_root/strict.json"

jq -e '
  .strict_passed == true and
  all(.strict_gates[]; . == true)
' "$work_root/strict.json"

jq '{
  model: .model,
  strict_passed,
  gate_count: (.strict_gates | length),
  failed: [.strict_gates | to_entries[] | select(.value != true) | .key],
  performance
}' "$work_root/strict.json"
```

Текущий checked report содержит 30 истинных strict gates. Число лучше всегда
брать из JSON: при добавлении нового независимого gate оно закономерно
изменится.

## 6. Создать новый кандидат vN

Ниже `vN` означает ещё не использованный номер, например `v15`. Не делайте
слепую глобальную замену: часть `v1` обозначает поколение модели, а не номер
release holdout.

### 6.1. Найти все version-bound значения

```bash
rg -n 'v15|schema_version|SPLIT_NAMESPACE|SEALED_REGISTRY|PRESEAL_RECEIPT|UNKNOWN_TYPO|HARD_NEGATIVE' \
  tools model packaging tests README.md README.en.md DESIGN.md docs \
  > "$work_root/version-bound.txt"
less "$work_root/version-bound.txt"
```

Обязательная минимальная ротация в `tools/train_intent_model.py`:

```text
SPLIT_NAMESPACE
SEALED_REGISTRY_RELATIVE_PATH
UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE
UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE
HARD_NEGATIVE_ROLE_NAMESPACE
HARD_NEGATIVE_SOURCE_RELATIVE_PATH
PRESEAL_RECEIPT_PATH
```

Также обновите `_POLICY` в `freeze_intent_development_corpus.py`, соответствующие
config-поля, packaging checks и тестовые ожидания. Если требуется новый
development domain, ротируйте обе `UNKNOWN_TYPO_DEVELOPMENT_*_NAMESPACE`.

Не создавайте `seal-registry-vN.json` вручную. Его атомарно создаст trainer для
точного кандидата после всех pre-sealed gates.

### 6.2. Изменить модель или policy

Обычные точки настройки находятся в `model/intent_v1/config.json`:

- `dimension`, seed и `ngram_orders` — контракт признаков;
- `dataset` — длины и symmetric typo augmentation;
- `ftrl` — epochs, patience, alpha/beta/L1/L2;
- `calibration` — отдельная Platt calibration;
- `thresholds` — precision/FPR policy, pause margin, veto и margin cap;
- `quality_gates` — минимальные selection и sealed recall/precision/specificity;
- `hard_negative_development` — распределение model-blind development по
  train/development/calibration/threshold и sample weight.

Изменение `ngram_orders` требует согласованного изменения runtime feature
schema; текущий loader требует точное совпадение `NGRAM_ORDERS`. Изменение
формата KSLM требует нового container schema и совместного обновления writer,
loader, packaging bounds и тестов.

Корневой config schema повышайте только при изменении его контракта. В этом
случае одновременно обновите exact validator в trainer, Windows packaging
check и tests.

### 6.3. Проверить или обновить frozen Onboard sources

Оставить текущий snapshot:

```bash
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
```

Создать намеренно новый snapshot из установленного `onboard-data`:

```bash
install -m 0644 /usr/share/onboard/models/en_US.lm \
  model/intent_v1/sources/en_US.lm
install -m 0644 /usr/share/onboard/models/ru_RU.lm \
  model/intent_v1/sources/ru_RU.lm
install -m 0644 /usr/share/doc/onboard-data/copyright \
  model/intent_v1/sources/COPYRIGHT.onboard-data

(cd model/intent_v1/sources && \
  sha256sum en_US.lm ru_RU.lm COPYRIGHT.onboard-data > SHA256SUMS && \
  sha256sum --check SHA256SUMS)

dpkg-query -W -f='${Version}\n' onboard-data
stat --printf='%n %s bytes\n' model/intent_v1/sources/*
```

После этого вручную обновите точные package version, sizes и hashes в config.
Обновите также provenance в `model/intent_v1/sources/README.md`. Обновление
sources всегда означает нового кандидата.

### 6.4. Заморозить новый development source

Сначала constants и config path/role namespace должны указывать на `vN`.
Поля будущего source SHA/bytes в промежуточном config могут временно содержать
синтаксически допустимые значения, потому что freezer вызывается с
`verify_frozen_source=False`; до train/preseal их обязательно заменить
фактическими.

```bash
PYTHONPATH=src python3 tools/freeze_intent_development_corpus.py \
  --config model/intent_v1/config.json \
  --en-model model/intent_v1/sources/en_US.lm \
  --ru-model model/intent_v1/sources/ru_RU.lm \
  --output "$work_root/unknown-typo-development-vN.json" \
  | tee "$work_root/development-freeze-result.json"

jq . "$work_root/development-freeze-result.json"
sha256sum "$work_root/unknown-typo-development-vN.json"
stat --printf='%s\n' "$work_root/unknown-typo-development-vN.json"
```

Перенесите файл в versioned repository path и обновите config значениями из
`development-freeze-result.json`:

```bash
install -m 0644 "$work_root/unknown-typo-development-vN.json" \
  model/intent_v1/unknown-typo-development-vN.json

jq -r '{
  source_sha256: .sha256,
  source_bytes: .bytes,
  expanded_corpus_sha256,
  signature_count
}' "$work_root/development-freeze-result.json"
```

Соответствие полей:

| Вывод freezer | Config |
| --- | --- |
| `sha256` | `hard_negative_development.source.sha256` |
| `bytes` | `hard_negative_development.source.bytes` |
| `expanded_corpus_sha256` | `external_evaluation.unknown_typo_development_corpus_sha256` |

После обновления config доказать byte reproducibility:

```bash
PYTHONPATH=src python3 tools/freeze_intent_development_corpus.py \
  --output "$work_root/unknown-typo-development-vN-replay.json" \
  > "$work_root/development-freeze-replay-result.json"

cmp model/intent_v1/unknown-typo-development-vN.json \
  "$work_root/unknown-typo-development-vN-replay.json"
```

### 6.5. Создать preseal receipt нового holdout

На этом шаге intent KSLM нельзя загружать или оценивать:

```bash
PYTHONPATH=src python3 tools/preseal_intent_holdout.py \
  --config model/intent_v1/config.json \
  --en-model model/intent_v1/sources/en_US.lm \
  --ru-model model/intent_v1/sources/ru_RU.lm \
  > "$work_root/holdout-vN-preseal-first.json"

jq -e '
  .model_loaded == false and
  .metrics_evaluated == false and
  .development.signature_count > 0 and
  .holdout.signature_count > 0 and
  .overlap_counts.development_holdout == 0 and
  .overlap_counts.sealed_holdout == 0
' "$work_root/holdout-vN-preseal-first.json"

jq -r '.holdout.corpus_sha256' \
  "$work_root/holdout-vN-preseal-first.json"
```

Перенесите выведенный SHA в
`external_evaluation.unknown_typo_holdout_corpus_sha256`, затем сохраните
receipt и повторите генерацию:

```bash
install -m 0644 "$work_root/holdout-vN-preseal-first.json" \
  model/intent_v1/holdout-vN-preseal.json

PYTHONPATH=src python3 tools/preseal_intent_holdout.py \
  > "$work_root/holdout-vN-preseal-second.json"

diff -u model/intent_v1/holdout-vN-preseal.json \
  "$work_root/holdout-vN-preseal-second.json"
```

Если diff не пуст, inputs ещё не заморожены. Не переходите к train.

### 6.6. Preflight перед необратимым seal claim

```bash
git diff --check
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)

PYTHONPATH=src python3 tools/preseal_intent_holdout.py \
  > "$work_root/preseal-final.json"
diff -u model/intent_v1/holdout-vN-preseal.json \
  "$work_root/preseal-final.json"

KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh
PYTHONPATH=src python3 tests/test_intent_training.py
PYTHONPATH=src python3 tests/test_intent_model_release.py
PYTHONPATH=src python3 tests/test_intent_model.py

test ! -e model/intent_v1/seal-registry-vN.json
```

Последняя проверка допустима только для действительно нового номера. Никогда
не удаляйте существующий registry ради прохождения этого `test`.

### 6.7. Обучить и один раз открыть sealed test

Это необратимая граница. Команду запускают только после review всех presealed
inputs:

```bash
set -o pipefail
PYTHONPATH=src python3 tools/train_intent_model_release.py \
  --workers 0 \
  --diagnostic-output "$work_root/presealed-failure.json" \
  | tee "$work_root/train-stdout.json"
```

Не добавляйте `--dry-run` в надежде избежать seal: проходящий кандидат всё
равно создаст registry. `--diagnostic-output` сохранит отдельный файл только
если кандидат провалился до seal; при успехе команда продолжит обычный train.

Проверки успеха:

```bash
test -s model/intent_v1/seal-registry-vN.json
test -s src/keyswitch/resources/models/layout_intent_v1.ksm
jq -e '.quality_gates_passed == true' model/intent_v1/manifest.json
jq -e '.quality_gates_passed == true' model/intent_v1/test-report.json

artifact_sha="$(sha256sum src/keyswitch/resources/models/layout_intent_v1.ksm | cut -d' ' -f1)"
test "$artifact_sha" = "$(jq -r .artifact_sha256 model/intent_v1/manifest.json)"

provenance="$(jq -r .build_provenance_sha256 model/intent_v1/manifest.json)"
version="$(jq -r .artifact_model_version model/intent_v1/manifest.json)"
test "$version" = "intent-v1-${provenance:0:12}"
```

### 6.8. Запустить independent strict evaluation

```bash
PYTHONPATH=src python3 tools/evaluate_intent_model.py --strict \
  > "$work_root/strict-vN.json"

jq -e '
  .strict_passed == true and
  all(.strict_gates[]; . == true)
' "$work_root/strict-vN.json"

jq -r '.strict_gates | to_entries[] | select(.value != true) | .key' \
  "$work_root/strict-vN.json"
```

Пустой вывод последней команды означает, что нет проваленных gates. Сам по
себе пустой список недостаточен: первая `jq -e` проверка тоже должна завершиться
с кодом 0.

### 6.9. Оформить отказ, если gate провален

Не перезапускайте изменённый кандидат на том же namespace. Сохраните:

```bash
sha256sum \
  src/keyswitch/resources/models/layout_intent_v1.ksm \
  model/intent_v1/manifest.json \
  model/intent_v1/test-report.json \
  model/intent_v1/seal-registry-vN.json \
  "$work_root/strict-vN.json"

jq -r '.strict_gates | to_entries[] | select(.value != true) | .key' \
  "$work_root/strict-vN.json"
```

Создайте `model/intent_v1/rejection-vN.json` по структуре предыдущего receipt.
Заполняйте только фактическими данными из manifest/registry/strict report:

```json
{
  "schema_version": 1,
  "phase": "ACTUAL_FAILED_PHASE",
  "decision": "rejected",
  "reason": "ACTUAL_FAILED_GATE_OR_REASON",
  "artifact_published": true,
  "internal_quality_gates_passed": true,
  "independent_external_holdout_evaluated": true,
  "model_version": "FROM_MANIFEST",
  "artifact_sha256": "FROM_SHA256SUM",
  "config_sha256": "FROM_MANIFEST",
  "candidate_sha256": "FROM_REGISTRY",
  "candidate_dataset_sha256": "FROM_REGISTRY",
  "seal_registry_sha256": "FROM_SHA256SUM",
  "manifest_sha256": "FROM_SHA256SUM",
  "internal_report_sha256": "FROM_SHA256SUM",
  "evaluator_sha256": "FROM_SHA256SUM",
  "strict_report_sha256": "FROM_SHA256SUM",
  "failure": {},
  "remediation": "Rotate every consumed split/registry/holdout namespace before the next candidate."
}
```

Boolean fields должны отражать фактическую фазу: например, при отказе до
публикации artifact или до external holdout они не могут оставаться `true`.
Не придумывайте метрики и не копируйте их из другого rejection receipt.

### 6.10. Выполнить два независимых replay

```bash
for run in replay-a replay-b; do
  mkdir "$work_root/$run"
  PYTHONPATH=src python3 tools/train_intent_model_release.py \
    --artifact "$work_root/$run/layout_intent_v1.ksm" \
    --manifest "$work_root/$run/manifest.json" \
    --test-report "$work_root/$run/test-report.json" \
    > "$work_root/$run/stdout.json"
done

for file in layout_intent_v1.ksm manifest.json test-report.json; do
  cmp "$work_root/replay-a/$file" "$work_root/replay-b/$file"
done

cmp src/keyswitch/resources/models/layout_intent_v1.ksm \
  "$work_root/replay-a/layout_intent_v1.ksm"
cmp model/intent_v1/manifest.json "$work_root/replay-a/manifest.json"
cmp model/intent_v1/test-report.json "$work_root/replay-a/test-report.json"

PYTHONPATH=src python3 tools/evaluate_intent_model.py \
  --artifact "$work_root/replay-a/layout_intent_v1.ksm" \
  --manifest "$work_root/replay-a/manifest.json" \
  --strict > "$work_root/replay-a/strict.json"
jq -e '.strict_passed == true' "$work_root/replay-a/strict.json"
```

## 7. Прочитать результат обучения

Краткая карточка artifact:

```bash
jq '{
  artifact_model_version,
  artifact_sha256,
  build_provenance_sha256,
  config_sha256,
  dataset_sha256,
  split_namespace,
  quality_gates_passed,
  training: {
    best_epoch: .training.best_epoch,
    epochs_run: (.training.history | length),
    nonzero_weights: .training.nonzero_weights,
    supported_character_fingerprints: .training.supported_character_fingerprints
  },
  quantization
}' model/intent_v1/manifest.json
```

Порог и selection-метрики каждого trigger:

```bash
jq '.thresholds | with_entries(.value |= {
  global_logit_margin,
  logits,
  selection_metrics,
  selection_typo_metrics
})' model/intent_v1/manifest.json
```

Internal gate breakdown:

```bash
jq '.quality_gate_breakdown' model/intent_v1/test-report.json
```

External/runtime/performance итог:

```bash
jq '{
  strict_passed,
  failed_gates: [.strict_gates | to_entries[] | select(.value != true) | .key],
  unknown_typo: .lexical_disjoint_unknown_typos,
  production_context_ensemble,
  performance
}' "$work_root/strict-vN.json"
```

## 8. Полный тестовый и packaging-рецепт

### Typecheck и 100% coverage

```bash
./tools/install-typing-tools.sh .typing
KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh

dbus-run-session -- \
  xvfb-run -a -s "-screen 0 1280x800x24" \
  ./tests/run_coverage.sh
```

### Detector и Linux E2E

```bash
PYTHONPATH=src python3 tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict

dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24 -noreset" \
  bash -c 'setxkbmap -layout us,ru && GTK_A11Y=none PYTHONPATH=src python3 tests/e2e_x11.py'

dbus-run-session -- env PYTHONPATH=src python3 tests/e2e_tray_menu.py
```

### Native DEB

```bash
sudo apt-get install --yes --no-install-recommends \
  at-spi2-core build-essential ccache dbus-x11 desktop-file-utils file \
  gir1.2-adw-1 gir1.2-atspi-2.0 gir1.2-gtk-4.0 libglib2.0-bin \
  libx11-6 libxkbcommon0 libxtst6 lintian patch patchelf \
  python3-coverage python3-dbus python3-dev python3-gi \
  x11-xkb-utils xauth xvfb

./tools/install-build-tools.sh .nuitka
KEYSWITCH_NUITKA_ROOT=.nuitka ./packaging/build-deb.sh

package_version="$(sed -nE 's/^version = "([^"]+)"/\1/p' pyproject.toml | head -n 1)"
architecture="$(dpkg --print-architecture)"
package="dist/keyswitch_${package_version}_${architecture}.deb"

./tools/verify-native-deb.sh "$package"
desktop-file-validate packaging/io.github.olegius88.KeySwitch.desktop
lintian --fail-on error "$package"

dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24 -noreset" \
  bash -c 'setxkbmap -layout us,ru && GTK_USE_PORTAL=0 ./tools/run-native-e2e.sh "$1"' \
  _ "$package"
```

`build-deb.sh` сам выполняет strict intent evaluation перед Nuitka build.
Чтобы не ждать её второй раз, передайте отчёт, уже полученный в этом же
контуре: `KEYSWITCH_INTENT_STRICT_REPORT="$work_root/strict.json"`. Скрипт
примет его только после `tools/verify_intent_strict_report.py`, который
сверяет каждый hash отчёта с текущими файлами; так же поступают CI и
`tools/release_pipeline.py`.
Успешный package verifier подтверждает, что DEB содержит native executable,
точный KSLM и frozen EN/RU models и не зависит от системного Python.

### Всё сразу одним процессом

Рецепты этого раздела и replay-доказательства раздела 6 объединены в
`tools/release_pipeline.py`:

```bash
python3 tools/release_pipeline.py start --profile app      # как job verify в CI
python3 tools/release_pipeline.py start --profile release  # плюс replay-доказательства
python3 tools/release_pipeline.py status
```

Прогон отсоединён от терминала и переживает закрытие сессии; результат
читается из `dist/release-pipeline/latest/SUMMARY.md`. Уже запущенные вручную
replay передаются через `--replay-dir`, готовый strict-отчёт — через
`--strict-report`; `--jobs` и `--memory-reserve-mib` ограничивают параллелизм
по числу фаз и по свободному ОЗУ, а `--fail-fast` останавливает приём новых
фаз после первого отказа.

## 9. Частые ошибки

### `unsupported training config schema`

Root `schema_version` не совпадает с exact validator в
`TrainingConfig.validate()`. Либо верните правильную схему, либо обновите
контракт, validator, Windows packaging checks и тесты одним изменением.

### `hard-negative development source must match the versioned path`

Config path не совпадает с `HARD_NEGATIVE_SOURCE_RELATIVE_PATH`. Обновите оба
значения согласованно; не отключайте проверку.

### `frozen hard-negative source differs from model-blind development`

Изменились Hunspell snapshot, namespaces, generation policy или frozen file.
Не редактируйте JSON. Проверьте версии/hashes словарей и повторите freeze как
нового кандидата.

### `sealed test namespace is already consumed by another candidate`

Хотя бы один candidate input отличается от закреплённой записи. Registry
работает правильно. Сохраните его и ротируйте split namespace, registry path и
independent holdout.

### Trainer завершился до появления registry

Это pre-sealed отказ. Полный JSON уже напечатан в stdout; если на исходной
команде был `--diagnostic-output`, тот же отчёт сохранён отдельно. Исправление
можно проверять без раскрытия test, но перед каждой попыткой убеждайтесь, что
registry действительно отсутствует.

### Trainer создал registry, но вернул ошибку

Seal уже израсходован. Сохраните evidence и не повторяйте изменённый кандидат.
После анализа нужен новый номер и полная ротация.

### Strict evaluator остановился до external metrics

Смотрите `.phase`, `.provenance` и `.strict_gates` в JSON. Fail-closed
provenance считается настоящим отказом; нельзя запускать «упрощённый» evaluator
и объявлять его эквивалентом strict.

### Load или inference latency нестабильна

Повторите strict run на reference host без конкурирующей тяжёлой нагрузки и
сравните `.performance`. Не меняйте performance gate по результатам одного
неудачного запуска. Если code/artifact меняется, это новый candidate identity.

### Можно ли ускорить обучение GPU

Текущий trainer использует многопроцессный CPU backend для независимого
извлечения признаков и scoring, но online update sparse FTRL-Proximal остаётся
последовательным. GPU backend в коде отсутствует. Критический release contract
требует deterministic byte-identical replay и включает Python/platform identity
в provenance. Поэтому штатный рецепт использует CPU. GPU-реализация была бы
отдельным trainer/toolchain-кандидатом и должна заново доказать численную
эквивалентность, determinism, sealed policy и runtime artifact parity.

## 10. Что обновить после принятия

После фактического прохождения всех gates внесите только измеренные значения:

1. Новый artifact/version/hashes в `MODEL_CARD.md` и `MODEL_CARD.en.md`.
2. Split/config/container/feature versions и краткие метрики в README/DESIGN.
3. Причины архитектурного изменения в detection research.
4. Release note в `CHANGELOG.md`.
5. CI/packaging pins, если изменились schema, paths или source policy.
6. Registry, preseal receipt, frozen development source, manifest, test-report
   и KSLM artifact — одним reviewable release change.

Не копируйте числа из предыдущего кандидата. Источник фактов — новый
manifest, strict report, `sha256sum` и результаты `cmp`.

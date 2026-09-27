# KeySwitch Layout Intent v1

[**Русский**](MODEL_CARD.md) · [English](MODEL_CARD.en.md)

Область карточки — **базовая KSLM**, не приложение целиком. Контекстный
помощник, добавленный в 0.15.0, имеет отдельные признаки, веса и отчёты:
[документация](../../docs/context-assistant.md). Расширенный
[context-v2](../context_v2/README.md) в 0.16.0 отклонён. Числа ниже —
зафиксированное обучение базовой KSLM в namespace v28; это не оценка режима `assist`.
Контрольные суммы связывают файлы и выявляют расхождения, но не являются
цифровой подписью издателя. `signed` для весов/FNV означает знак числа;
подлинность издателя CRC32/SHA-256 сами по себе не доказывают.

## Назначение

`Layout Intent v1` — собственный локальный линейный классификатор намерения
сменить раскладку EN/RU. Он оценивает пару «набранное слово → та же физическая
последовательность в другой раскладке» и используется только внутри
консервативной политики KeySwitch. Жёсткие защиты (короткие слова, код,
исключения, подтверждённые и отклонённые правила пользователя, допустимое
исходное слово) остаются выше модели.

Модель не отправляет текст в сеть, не требует NumPy, scikit-learn, ONNX или
отдельного рантайма и выполняет один скалярный проход по разреженным признакам.

## Текущий артефакт: namespace v28

`v28` обозначает поколение обучающих данных KSLM, а не номер релиза приложения.
Текущий артефакт — `intent-v1-8bccdf50b028`, 12 947 840 байт, SHA-256
`d0b90ad59a48eb7c2aa8c89680a2bc0dd2290c1e9c717068af49087c165878bb`.
[Manifest](manifest.json) фиксирует build provenance
`8bccdf50b0281fe8078b8262247cec8dadb423dc8ca9ce43da6bdc482a41ace9`,
config `2026433380f1a418079c8cb60eafc64cc420fc256b3c074dbb7d6389f6b275fd`
и dataset `f3a250aceb6a065fb403962aa978fc0a9c243b3a942e6b30fda85842ba63d04e`.
По development выбрана эпоха 40 из 44 выполненных; контейнер содержит
711 340 ненулевых весов и 1 031 370 membership-отпечатков.

[Сохранённый test-report](test-report.json) сообщает о прохождении внутренних
условий обучения. Boundary_probe, space, tab, enter и punctuation не дают
ложных срабатываний на 22 435 негативов и показывают recall 0,956004279;
pause — тоже без ложных срабатываний на 22 435 негативов и recall 0,947133815.
SHA-256 manifest —
`c21631d7b484317d6c892d3e77ac6f19983d78fe820f80b1db1e32f3fce410d8`,
test-report — `59a92bda24ffa2d3cb46ec88001f2f04e18fad87ce31edc8fd7efcb2ba58de0d`.
Эти лексические и синтетические результаты не измеряют точность полного
переключателя в пользовательском потоке.

Внешний [strict evaluator](../../tools/evaluate_intent_model.py) выполнил все
30 обязательных условий. SHA-256 полного отчёта —
`2d21c88522e7b18e4941067bbcb2160c96a0a9f77907ae6d4f5e420d0f4c9085`;
отдельный verifier подтвердил identity модели и 14 файлов provenance — на один
больше, чем у предыдущего поколения, за счёт нового
`toolchain.constants_sha256`. Два независимых replay воспроизвели
опубликованные artifact, manifest и test-report побайтно; strict report
одного из них также прошёл все gates. Контекстные проверки и нативная
упаковка остаются отдельными шагами release-контура. Старые измерения из
выпуска v23 не сертифицируют этот артефакт; они сохранены в [исторической
карточке v0.22.0](https://github.com/olegius88/keyswitch/blob/v0.22.0/model/intent_v1/MODEL_CARD.md).
Ни неизменность названия Python, ни один сохранённый флаг успешности не заменяют
текущую проверку байтов и всех обязательных условий.

## Данные и лицензирование

Обучение использует только секции `1-grams` из зафиксированных побайтно моделей
Onboard в репозитории:

- `model/intent_v1/sources/en_US.lm`;
- `model/intent_v1/sources/ru_RU.lm`.

Снимок взят из Ubuntu 26.04 `onboard-data`
`1.4.3+git20260213+ds-2`. Файл
`model/intent_v1/sources/COPYRIGHT.onboard-data` скопирован побайтно из пакета;
его stanza `Files: models/*` содержит точную декларацию `GPL-3+` и атрибуцию.
`SHA256SUMS` и секция `sources` в `config.json` фиксируют SHA-256 и размер всех
трёх файлов. Trainer записывает те же provenance-поля в manifest. Мы не
подменяем декларацию нормализованным SPDX-идентификатором и не делаем в model
card самостоятельных юридических выводов. Сторонние корпуса и сетевые API не
используются при обучении этой KSLM; отдельный эксперимент context-v2
имеет собственные публичные источники и provenance.

Побайтная provenance-запись и контрольные суммы описаны в
[sources/README.md](sources/README.md) и `sources/SHA256SUMS`.

## Защита от утечки

Единица разделения — не строка и не язык, а физическая последовательность
клавиш. Для русского слова она сначала отображается в координаты US-клавиатуры.
До аугментации SHA-256 этой последовательности в namespace
`keyswitch:intent-v28:physical-signature` распределяет её по 40 неизменным
бакетам:

- 26/40 (65%) — обучение;
- 4/40 (10%) — выбор эпохи;
- 4/40 (10%) — sigmoid-калибровка;
- 3/40 (7,5%) — выбор рабочих порогов;
- 3/40 (7,5%) — запечатанный финальный тест.

Одна физическая последовательность никогда не может оказаться в двух split.
EN/RU-слова с одинаковой последовательностью не участвуют в обучении и
попадают в отдельный safety-набор с целевым решением «не переключать».

Dataset строится в две фазы. До захвата seal отдельный pre-pass перечисляет
фактические физические сигнатуры identity-варианта, удаления, дублирования и
перестановки только для train, development, calibration и threshold. Сигнатура
помещается в candidate quarantine, если у неё обнаружены владельцы из разных
pre-sealed split или языков либо пересечение с защищённым
hard-negative/safety токеном. Sealed test на этой фазе не строится и не влияет
ни на строки кандидата, ни на quarantine, scorer или fingerprint.

Только после успешного захвата seal test строится отдельной фазой со своим
quarantine. При асимметричном merge из test удаляются сигнатуры, пересекающиеся
с уже зафиксированными строками, quarantine или safety-данными кандидата;
pre-sealed строки при этом должны остаться побайтно эквивалентными. После merge
набор повторно аудируется
по реально сгенерированным сторонам пар. Канонические SHA-256 обоих quarantine,
список исключённых test-сигнатур и числа исключённых вхождений входят в
provenance модели.

Текущий namespace v28 использует дополнительный frozen source
`unknown-typo-development-v28.json`, созданный model-blind до обучения из
unknown-typo development-корпуса. Он содержит 10 000 уникальных физических
сигнатур — по 5 000 на язык — без test-роли. Независимый namespace
`keyswitch:intent-v28:unknown-typo-development-role` распределяет в каждом
языке 3 500 слов в train и по 500 в development, calibration и threshold.
Loader проверяет размер и SHA-256 source, provenance Hunspell `.dic`/`.aff`,
физическую эквивалентность EN/RU пары, уникальность и точный SHA-256 повторно
развёрнутых 120 000 строк (два label × шесть trigger). После объединения общий
row-level audit снова запрещает cross-split, cross-language, safety и
quarantine пересечения. Компактный source и freezer входят в toolchain
provenance; внешний v28 holdout использует другие rank/choice namespaces.

Model-blind development domain этого шага исключает все физические сигнатуры
уже зафиксированного sealed split, поэтому он смещается вместе с namespace: у
каждого нового поколения `external_evaluation.unknown_typo_development_corpus_sha256`
в config.json, `hard_negative_development.source.bytes` и закреплённый SHA-256
в `tests/test_intent_training.py` могут отличаться от прежнего поколения и
берутся из вывода freezer, а не переносятся по аналогии.

`config.json` schema 13 также содержит policy `sealed_evaluation` schema 1.
Указанный в ней repository-relative
`registry_path: model/intent_v1/seal-registry-v28.json` разрешается от
канонического корня проекта, а не от расположения переданной копии config, и
закрепляет один candidate SHA за одним `split_namespace`. После успешного
прохождения полного pre-sealed gate — threshold/context, safety,
selection-veto и пробной runtime-сериализации KSLM — но до построения test-фазы
или оценки первой sealed-test строки trainer вычисляет канонический hash
кандидата. Пробная сериализация заранее проверяет числовые границы,
quantization parity и лимиты payload/fingerprint. Hash связывает candidate
dataset, toolchain, scorer, selection evidence и точный payload и
runtime-параметры KSLM. Registry сначала полностью записывается и `fsync`-ится
во временный файл того же каталога, затем публикуется атомарной hard-link
операцией без замены существующей записи. Побайтно идентичная запись разрешает
повтор того же кандидата для проверки воспроизводимости. Любое отличие
блокируется до доступа к sealed test; изменённый кандидат требует явной
совместной ротации `split_namespace` и `registry_path`.

`seal-registry-v2.json` сохранён только как неизменяемое свидетельство
отклонённой попытки: кандидат v2 был зафиксирован, после чего фазовый аудит
остановил процесс до оценки закрытого test и до публикации модели. Этот
namespace не переиспользуется.

`seal-registry-v3.json` также сохранён как неизменяемый аудит. Кандидат v3
прошёл pre-seal и обычный sealed-срез, но non-pause typo-срез получил 10 false
positive на 17 392 отрицательных примерах: верхняя 95%-граница Wilson
0,001058171 превысила policy 0,001. Модель не публиковалась. Результат не
используется для повторной настройки того же test; следующий запуск использовал
новый namespace v4 и отдельный registry v4.

`seal-registry-v4.json` сохранён по той же причине. На selection кандидат v4
получил 9 false positive на 23 067 отрицательных общих примерах и 8 на 17 220
typo-примерах. В независимом sealed test это стало 14/23 090 и 13/17 223:
обычные верхние 95%-границы Wilson 0,001017564 и 0,001291083 превысили policy
0,001. Модель снова не публиковалась. Следующий v5 не переиспользовал этот test:
его pre-seal policy заранее применил Bonferroni-корректированную family-wise
95%-границу к 12 первичным FP-проверкам (6 trigger × overall/typo), фиксируя
per-comparison confidence 0,9958333333333333 и z=2,8652602385321333. Sealed
gate остаётся обычной независимой 95%-границей Wilson.

`seal-registry-v5.json` фиксирует следующий непереиспользуемый кандидат. Он
прошёл внутренние sealed gates, но внешняя production-проверка выявила две
проблемы: вторичные эвристические условия снизили recall ансамбля на
unknown-typo до 0,5965 при raw-model recall около 0,98, а 822 из 10 000
извлечённых `.dic`-основ не считались допустимыми открытым runtime Hunspell
handle из-за семантики affix-флагов. После изучения этих результатов внешний
v5 corpus объявлен только development-набором.

`seal-registry-v6.json` и `rejection-v6.json` фиксируют следующую отклонённую
попытку. Обычный non-pause срез прошёл с 9/21 288 false positives и верхней
95%-границей Wilson 0,000803369; Pause также прошёл. Но non-pause typo-срез
получил 9/15 812 и верхнюю границу 0,001081498 при лимите 0,001. Artifact,
manifest и report v6 не публиковались. V7 прошёл 29 из 30 строгих внешних gate,
но был отклонён из-за load-latency p95 655,432857 мс при лимите 500 мс; точное
решение сохранено в `rejection-v7.json`. V8 прошёл остальные 29 gate, включая
исправленный load-latency p95 268,771704 мс, но production-context проверка
обнаружила, что вторичные membership/target-score veto снизили raw sealed
non-pause recall с 0,950488303 до 0,941585283; точное решение сохранено в
`rejection-v8.json`. Раскрытые test v6/v7/v8 больше не используются. V9 удалил
эти post-model veto и прошёл внутренние gates, но независимый внешний
unknown-typo holdout дал 4 false positive из 10 000 отрицательных примеров на
каждый trigger: precision 0,999591378, specificity 0,9996 и верхняя 95%-граница
Wilson 0,001028128 при лимите 0,001. Поэтому production-context gate отклонил
кандидата; точное решение сохранено в `rejection-v9.json`, а раскрытые v9
sealed/holdout данные не используются для настройки. V10 применил выбранный
только на development-корпусе закреплённый cap 2,0 и детерминированно получил
общий margin 0,9938225471937638. Он прошёл pre-seal, но независимый sealed
non-pause recall составил 0,944410276 при минимуме 0,95; точное решение
сохранено в `rejection-v10.json`, а раскрытые строки v10 не переиспользуются.
V11 прошёл внутренние selection и sealed gates, но закреплённый SHA-256
strict-evaluator затем остановился до независимого external holdout: индекс
исключений не поддерживал новое семейство frozen `hunspell-unknown-*` строк.
Артефакт v11 отклонён, точная причина и все хэши сохранены в
`rejection-v11.json`. V12 исправил индекс, но evaluator построил базовый
exclusion-index уже после merge development-корпуса и потому не воспроизвёл его
frozen provenance; `rejection-v12.json` фиксирует отказ до внешних метрик. V13
исправил разделение доменов и дошёл до независимого holdout, где обычные trigger
получили 4 false positive из 10 000 и Wilson upper 0,001028128 при лимите 0,001;
решение сохранено в `rejection-v13.json`. V14 до открытия нового holdout
зафиксировал нулевой selection FP-бюджет и minimum recall 0,956, ротировал
split/registry/source/holdout namespaces и прошёл strict-проверку с 0 false
positive из 60 000 unknown-typo негативов. Новый holdout заранее зафиксирован
без загрузки модели в `holdout-v14-preseal.json`. V15 после перехода trainer на
многопроцессный backend снова ротировал все namespaces, зафиксировал model-blind
holdout в `holdout-v15-preseal.json` и прошёл strict-проверку с 12 false
positive из 60 000 негативов и recall 0,96935. После переноса FTRL в нативное
ядро v16 и v17 не прошли pre-sealed gate (при нулевом FP-бюджете recall на
threshold-split 0,9479 и 0,9458 при минимуме 0,956; registry не создавался). V18
прошёл внутренние gates и независимый holdout (5 false positive из 60 000,
recall 0,9425), но отклонён strict-gate `fallback_regression`: один false
positive, внесённый моделью относительно детерминированного fallback на 5
000-строчной sealed-выборке; решение сохранено в `rejection-v18.json`. V19 снова
не прошёл pre-sealed gate (recall 0,9463 при нулевом FP-бюджете), а v20 прошёл
все внутренние gates, holdout и 30 strict gates и стал текущим сертифицированным
артефактом. Ротация toolchain-bound namespace на v23 приняла кандидата
`intent-v1-b2a2ec8caa8d`, который стал следующим сертифицированным артефактом.
После перевода detector, intent-моделей и обучающих инструментов на именованные
константы из `src/keyswitch/constants` точный replay v23 на новом toolchain
воспроизвёл прежний артефакт, manifest и test-report побайтно, и namespace
ротировался дальше. V24 прошёл обучение, внутренние ворота и независимый
external holdout, но отклонён строгим gate `production_context_ensemble`; точное
решение сохранено в `rejection-v24.json`. V25 не прошёл выбор порога до захвата
seal: recall на selection составил 0,951688289 (у pause — 0,942078365) при
минимуме 0,956; решение сохранено в `rejection-v25.json`, registry не создавался
и sealed test не оценивался. V26 (`intent-v1-1a0edbe60e84`) и V27
(`intent-v1-cff4eaedc88a`) прошли обучение, внутренние условия, независимый
holdout и полный набор из 30 строгих gates, но вместе с переобученной
контекстной моделью не сохранили критерий end-to-end на раскрытых наборах
предложений: с V26 при правке внутри слова восстановлено на одно предложение
меньше, чем с 0.32.0 (17 390 против 17 391), с V27 при обычном наборе — 16 353
предложения против 16 367, поэтому ни один не был выбран в пакет. V28 прошёл все
внутренние условия, независимый holdout и полный набор из 30 строгих gates,
сохранил этот критерий вместе с контекстной моделью и стал текущим
сертифицированным артефактом.

`--dry-run` не отменяет эту гарантию: если полный pre-sealed gate пройден,
registry захватывается до sealed scoring и seal расходуется, даже когда artifact,
manifest и report не публикуются.

## Примеры и признаки

Для каждого слова создаётся симметричная пара:

- слово в правильной раскладке — отрицательный пример;
- та же последовательность в неправильной раскладке — положительный пример.

Удаление, дублирование и перестановка соседних физических клавиш применяются
симметрично к обоим классам. Это не позволяет модели выучить опасное правило
«любая опечатка означает неправильную раскладку». Технические токены, адреса,
версии, пути и идентификаторы добавляются как hard negatives; отдельный набор
таких строк остаётся safety-проверкой.

Feature schema v5 не использует dense-лексические или контекстные признаки.
Вход классификатора строится только из исходного и альтернативного raw token:
знаковых символьных
1–5-грамм, направления, длины и trigger. `context_delta`, `context_group` и все
поля `WordScore` — lexical/frequency/ngram/invalid-ratio, exact и spell-known —
полностью игнорируются даже тогда, когда вызывающая сторона их заполнила.
Реальный краткий контекст остаётся в консервативной эвристике detector и не
учитывается второй раз в linear score. Trainer передаёт заведомо нейтральный
`WordScore` и вызывает тот же runtime extractor с теми же feature/membership
seed и порядками n-грамм. Это делает train/serve feature parity точной и
устраняет зависимость признаков от корпуса language model.

Два train-only scorer, созданные из не помещённых в quarantine identity-слов
split `train`, сохраняются как отдельный проверяемый provenance-объект. Они
используют символьные 2/3/4-граммы без Hunspell, но feature extractor их не
вызывает, поэтому их оценки и train-частоты не могут влиять на классификатор.
Канонический hash входных train-лексиконов, их размеры и число исключённых
quarantine-identity фиксируются во внешнем manifest.
Frozen EN/RU-лексиконы используются целиком после фильтрации;
`maximum_words_per_language` обязан оставаться нулём, поэтому held-out identity
не может влиять на глобальное усечение до назначения split.

Calibration, threshold и sealed test используют нейтральный контекст
(`context_delta=0`, `context_group=None`) как основной срез. Threshold и sealed
test отдельно сертифицируют фиксированные непустые label-independent
context-stress профили. Для feature schema v5 это проверка инвариантности:
изменение только `context_delta`/`context_group` не должно менять feature vector,
logit, probability или решение, а каждый профиль проходит те же per-trigger
precision/recall/specificity/Wilson-FPR gates. Один training trigger равномерно
выбирается hash-функцией из всех runtime triggers; threshold/test оценивают
каждый trigger отдельно.

Восемнадцать stress-профилей покрывают обе связи контекста (`source` и
`target`), границы adversarial-домена ±6, репрезентативные внешние,
внутренние и околонулевые точки ±1,25, ±0,75 и ±0,125, а также нулевую точку:

| `context_delta` | `source` | `target` |
| ---: | --- | --- |
| -6,0 | `source_minimum` | `target_minimum` |
| -1,25 | `source_outer_negative` | `target_outer_negative` |
| -0,75 | `source_inner_negative` | `target_inner_negative` |
| -0,125 | `source_near_zero_negative` | `target_near_zero_negative` |
| 0,0 | `source_zero` | `target_zero` |
| +0,125 | `source_near_zero_positive` | `target_near_zero_positive` |
| +0,75 | `source_inner_positive` | `target_inner_positive` |
| +1,25 | `source_outer_positive` | `target_outer_positive` |
| +6,0 | `source_maximum` | `target_maximum` |

`positive`/`negative` в именах обозначает только знак наблюдаемого
`context_delta`, а не label примера; `minimum`/`maximum` обозначают границы
adversarial-домена. В отчёте поле `context` равно
`neutral_primary_plus_fixed_label_independent_stress`, а точный упорядоченный
список закреплён в `context_stress_profiles`. Pre-sealed доказательства
находятся в
`threshold_selection_gate_breakdown.neutral` и
`threshold_selection_gate_breakdown.context_stress.profiles.<name>.per_trigger`
с отдельными `{overall,typos}` для каждого trigger. Sealed gates имеют ту же
структуру в `quality_gate_breakdown.sealed_test_context_stress`; сырые
sealed-метрики находятся в
`sealed_test_context_stress.<name>.{overall,typos}`.

Отдельный strict production-context gate запускает настоящий
`LanguageDetector` на neutral и шести достижимых extrema: `none_min/-1,75`,
`none_max/+1,75`, `source_min/-2,05`, `source_max/+1,45`,
`target_min/-1,20`, `target_max/+2,30`. Проверяются пять срезов: sealed test,
sealed typos, внешние unknown typos, safety и source-known. На каждом профиле
действуют абсолютные precision/specificity/Wilson-FPR ограничения, суммарное
число false positives не выше contextual fallback и neutral, а также
асимметричная recall policy: абсолютный floor для neutral/target-supporting
контекста и не хуже fallback минус 0,005 для source-supporting контекста.
На конечных safety/source-known наборах действует exact-zero: строки обязаны
останавливаться до модели. Все unknown-typo строки должны достигать её. Полное
доказательство записывается в
`production_context_ensemble` отчёта evaluator.
Source-known строки являются runtime-поднабором frozen lexical-disjoint
корпуса: каждая основа должна фактически приниматься открытым Hunspell handle,
поскольку одна запись `.dic` с affix-флагами не гарантирует самостоятельную
словоформу. Derived-фильтр не меняет закреплённый hash исходного корпуса и
fail-closed требует примеры обоих направлений.

Общий с runtime extractor строит signed feature hashing (FNV-1a 64), символьные
n-граммы порядков 1–5 по обеим интерпретациям, направление раскладки, длину и
тип границы. Вектор содержит
2 097 152 hash-buckets. Размерность, seed, отдельный membership-seed и порядок
n-грамм зафиксированы в `config.json`; feature schema v5 закреплена общими
константами trainer/runtime и встроенным manifest KSLM.

Runtime вызывает классификатор только при максимальной длине двух
нормализованных интерпретаций не менее 5 символов. Dataset builder применяет
тот же минимум после typo augmentation, поэтому укороченная deletion-опечатка
не может присутствовать в обучении или external unknown-typo проверке, а затем
быть пропущенной runtime. Ограничение закреплено в
`gate_policy.model_applicability` embedded и внешнего manifest.
После hard guards этот применимый KSLM-ответ является единственным
статистическим решением: применяется только откалиброванный direction/trigger
threshold. Membership coverage и языковые score остаются диагностикой и не
могут отменить положительный результат порога. При отрицательном ответе
эвристика не получает второй шанс; она остаётся fallback только для коротких
токенов, отключённой модели и отсутствующего артефакта.
Короткие реальные collision-пары из тех же frozen источников сохраняются
только в safety corpus. Они не дают признаков или градиентов модели, но
обеспечивают ненулевое покрытие valid-source hard guard во всех шести trigger.

## Алгоритм обучения

Используется разреженная логистическая регрессия FTRL-Proximal. Для координаты
`i`:

```
w_i = 0,                                      если |z_i| <= L1
w_i = -(z_i - sign(z_i) * L1) /
      ((beta + sqrt(n_i)) / alpha + L2),      иначе

g_i = (sigmoid(w*x) - y) * x_i * sample_weight
sigma_i = (sqrt(n_i + g_i^2) - sqrt(n_i)) / alpha
z_i <- z_i + g_i - sigma_i * w_i
n_i <- n_i + g_i^2
```

Свободный член обучается тем же обновлением без L1/L2. После каждой эпохи только
на development split измеряются log loss и агрегированная high-precision
operating point. Выбор эпохи сначала предпочитает полное прохождение той же
precision/recall/specificity/family-wise-Wilson-FPR policy, затем число
пройденных проверок,
recall, typo recall и защитные метрики; log loss служит последним tie-breaker.
Threshold split в выборе эпохи не участвует. Порядок примеров задаётся отдельным
`random.Random(seed + epoch)`. Конфигурация допускает максимум 64 эпох, минимум
6 и раннюю остановку после 4 эпох без улучшения этого порядка; в manifest
записываются фактически выбранная эпоха и вся история метрик.

Формулы соответствуют FTRL-Proximal из работ [McMahan,
2011](https://proceedings.mlr.press/v15/mcmahan11b.html) и [McMahan et al.,
2013](https://research.google.com/pubs/archive/41159.pdf).

## Квантизация, калибровка и пороги

Веса квантуются симметрично в signed int16. Калибровка и все последующие
метрики считаются уже по квантизованным logits, то есть совпадают с тем, что
исполняет runtime. KSLM schema 4 хранит отсортированные уникальные uint64-
отпечатки полных имён символьных признаков, реально наблюдавшихся на train
split. Для membership используется отдельный unsigned FNV-1a namespace, а не
bucket линейного веса. Runtime ищет точное значение отпечатка бинарным поиском;
поэтому коллизия в ограниченном весовом векторе не создаёт ложное покрытие, как
это происходило бы с occupancy-bitset.

Контейнер состоит из канонического JSON manifest, little-endian int16-весов и
little-endian uint64-отпечатков. Loader проверяет schema/feature version, точную
форму payload, сортировку и уникальность membership, CRC32, SHA-256 и предел
полного файла 14 MiB. Embedded manifest отдельно ограничен 1 MiB, payload —
12 MiB, а число membership-отпечатков — `2^20`. `layout_intent_v1.ksm` —
стабильное имя первого поколения
классификатора; версия формата контейнера независимо повышена до KSLM schema 4.
Не следует смешивать три независимых номера: training config использует
`schema_version: 13`, контейнер и его встроенный manifest — KSLM schema 4, а
внешний публикационный `manifest.json` — `schema_version: 2`.

Для каждого физического направления EN→RU и RU→EN независимая
двухпараметрическая sigmoid-калибровка (Platt scaling) обучается исключительно
на calibration split. Калибровка монотонна и потому сохраняет ранжирование
внутри направления, одновременно исправляя межнаправленный сдвиг score. Её
результат — **техническая уверенность на
синтетическом лексическом распределении**, а не вероятность ошибки реального
пользователя. Ни UI, ни документация не должны называть это real-world
probability.

Для каждого runtime trigger (`boundary_probe`, `space`, `enter`, `tab`,
`punctuation`, `pause`) отдельные logit-пороги EN→RU/RU→EN выбираются совместно
только на threshold split. Контекст нейтрален; каждый направленный operating
curve обязан содержать оба label и в общем, и в typo-срезе. До
вычисления sealed test кандидат одновременно должен пройти полную selection
policy. Для общего среза это precision >= 0,9995, recall >= 0,956,
specificity >= 0,999 и верхняя family-wise 95%-граница Wilson FPR <= 0,001. Для
typo-среза это selection precision >= 0,9995, recall >= 0,91, specificity >=
0,999 и та же граница. Bonferroni-коррекция охватывает 12 первичных сравнений
(6 trigger × overall/typo): per-comparison confidence 0,9958333333333333,
z=2,8652602385321333. Signed gate evidence хранит метод, correction, число
сравнений, confidence, z и endpoint; sealed gate независимо использует обычный
95% Wilson с z=1,959963984540054. Config schema 13 дополнительно требует ноль
false positive на общем и typo selection-срезах каждого trigger; этот
абсолютный бюджет входит в закреплённое hashes evidence и проверяется до materialization
test. После направленного выбора
trainer детерминированно находит на threshold
split максимальный общий margin, который сохраняет весь selection gate, и
прибавляет его к каждому calibrated-logit порогу. Signed cap 2,0 зафиксирован
до v11 по внешнему model-blind unknown-typo development-корпусу; schema 13
выбирает фактический margin только на его frozen threshold-роли. Значение
хранится для каждого trigger, а sealed test в выборе не участвует. Selection
требует recall 0,956/0,91 для общего/typo-среза; у `pause` — 0,91/0,86. Это
сохраняет запас над sealed-порогами 0,95/0,90 и 0,90/0,85. FPR остаётся
<= 0,001, а logit-порог каждого направления
устанавливается минимум на 0,5 выше самого строгого не-pause порога того же
направления; метрики пересчитываются уже после
этого ужесточения. Если полный набор требований невыполним, trainer завершает
работу с `sealed_test_evaluated=false`, не читая результаты sealed test.
В независимом sealed test minimum precision обоих срезов равен 0,999;
усиление selection по precision и recall служит запасом переноса и не меняет
закрытый gate.
Наблюдаемый нулевой FPR без достаточного размера среза не считается
доказательством безопасности. KSLM schema 4 хранит обе пары коэффициентов,
точный calibrated-logit порог для каждой пары trigger/направление и raw-logit
veto threshold для консервативного policy layer. Runtime выбирает точный
`threshold_logit` по trigger и физическому направлению, затем сравнивает с ним
calibrated logit. Sigmoid-derived confidence этого порога вычисляется
только для диагностики и не участвует в сравнении; поэтому насыщение sigmoid не
может превратить разные logit-границы в одно runtime-решение.

Veto-порог не заимствуется у auto-switch: он устанавливается ниже нижнего
0%-квантиля, то есть минимального logit положительных calibration-примеров, ещё
на фиксированный margin 0,25. Manifest и sealed test отдельно фиксируют долю
положительных примеров, которые такой veto мог бы заблокировать; допустимый
false-negative rate равен 0,001.

Строгий safety-gate прогоняет защитный корпус через настоящий production
`LanguageDetector`: runtime lexical scorers, pre-model guards и policy layer.
Коллизии раскладок и защищённые токены обязаны остановиться до модели, а
допустимое число guard failures равно нулю. Прямые ответы линейной модели и
membership coverage на тех же строках сохраняются только как raw diagnostics с
`is_a_gate=false`; они не подменяют проверку фактического production-решения.

## Воспроизводимость

Полный процесс принятия/отклонения нового кандидата описан в
[runbook](../../docs/intent-model-runbook.md), а команды для воспроизведения и
нового обучения — в [cookbook](../../docs/intent-model-cookbook.md).

```bash
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
PYTHONPATH=src:tools python3 tools/preseal_intent_holdout.py | \
  diff -u model/intent_v1/holdout-v28-preseal.json -
PYTHONPATH=src python3 tools/train_intent_model_release.py \
  --build-environment model/intent_v1/build-environment.json
PYTHONPATH=src python3 tools/evaluate_intent_model.py --strict
```

Текущий trainer входит в candidate identity и после выдачи receipt не изменяется.
На границе записи KSLM он преобразует tuple-контейнеры dataclass в
JSON-native массивы и побайтно доказывает неизменность канонического JSON.
`train_intent_model_release.py` остаётся стабильной командой запуска и только
делегирует trainer без monkey-patch или изменения process-global state.

`config.json` schema 13 содержит все параметры и точные frozen-source provenance
без скрытых defaults. Его root-секция `external_evaluation` фиксирует
`schema_version`, `minimum_words_per_group: 5000`, канонический
`trigger_expansion` (`boundary_probe`, `pause`, `space`, `enter`, `tab`,
`punctuation`), а для `hunspell.en_US` и `hunspell.ru_RU` — точные
`dictionary_sha256`, `dictionary_bytes`, `affix_sha256`, `affix_bytes`.
Вложенное значение `external_evaluation.schema_version: 2` версионирует только
эту внешнюю policy и не меняет `schema_version: 13` корневого training config.
Ожидаемые результирующие выборки закреплены полями
`lexical_disjoint_corpus_sha256`,
`unknown_typo_development_corpus_sha256` и
`unknown_typo_holdout_corpus_sha256`. До v11 development-корпус использовался
для выбора serving policy, включая закреплённый cap 2,0. Начиная с v14 новый model-blind
development source распределён по независимым pre-sealed ролям; фактический глобальный
calibrated-logit margin выбирается только на threshold-роли.
Holdout v28 построен другим rank/choice namespace до загрузки модели, исключает
все 288 861 sealed и 10 000 development физических сигнатур и впервые
оценивается только после фиксации candidate receipt. Число исключённых sealed
сигнатур зависит от поколения: у каждого нового namespace свой sealed split, и
`sealed_dataset_exclusions`/`combined_holdout_exclusions` preseal receipt
закрепляют актуальный счёт и SHA-256, которые пиновкой повторяет
`tests/fixture_values/counts.py`
(`EXPECTED_SEALED_EXCLUSION_SIGNATURE_COUNT`,
`EXPECTED_COMBINED_EXCLUSION_SIGNATURE_COUNT`) вместе с
`tests/test_intent_training.py`. Его model-blind provenance
заранее сохранён в `holdout-v28-preseal.json`: `model_loaded=false`,
`metrics_evaluated=false`, оба overlap-счётчика равны нулю. Внешний
manifest schema 2 сохраняет SHA-256 для config, значений именованных констант
toolchain,
frozen-источников,
trainer, внешний evaluator, preseal generator/receipt, development freezer,
runtime intent extractor, layouts, `language_model.py`, detector, frozen
hard-negative source и списка защищённых токенов. Поле
`toolchain.constants_sha256` фиксирует канонический SHA-256 значений, которые
эти файлы берут из `src/keyswitch/constants` — не байты файлов, которые их
импортируют. Дайджест строит `src/keyswitch/value_provenance.py`: он разбирает
исходники без импорта модулей и хеширует canonical JSON отсортированного
отображения `module.NAME -> значение`, поэтому правка byte-идентичного файла
без изменения значения константы не двигает hash, а правка значения константы
в любом файле, который её действительно использует, двигает. Python implementation/version/build,
платформа, архитектура, libc и byte order записаны отдельно в
[build-environment.json](build-environment.json), вне candidate identity.
Build-provenance hash дополнительно
связывает candidate/full dataset, оба quarantine, исключённые test-сигнатуры и
train-only scorer; первые 12 символов этого hash входят в model version. Strict
evaluator пересчитывает эти связи и
проверяет текущие toolchain/protected-token hashes.

Manifest и report содержат один и тот же immutable receipt
`sealed_evaluation`: schema, namespace, candidate/config/candidate-dataset
SHA-256, repository-relative путь registry и SHA-256 его канонических байтов.
Evaluator независимо пересчитывает candidate SHA по загруженному KSLM,
candidate dataset, toolchain, scorer и selection evidence и требует точного
совпадения receipt с локальным registry. Отсутствующий, недоступный,
символьный, слишком большой, изменённый или несогласованный registry приводит
к fail-closed остановке до построения test-фазы и вывода sealed-метрик — даже
без флага `--strict`. После merge отдельно проверяется полный dataset SHA до
вычисления метрик.

Строгие test- и release-задачи CI закреплены на GitHub-hosted runner
`ubuntu-26.04`, поэтому поколение Ubuntu для внешней Hunspell-проверки указано
явно; label опубликован в официальном [перечне образов GitHub
Actions](https://github.com/actions/runner-images/blob/main/README.md#available-images).
Авторитетным гейтом остаются точные hashes и размеры `.dic`/`.aff` из
external-evaluation policy: изменившийся снимок runner приводит к ошибке
проверки, а не к незаметной замене корпуса.

Trainer не публикует ничего при непройденных gates. После успешной проверки он
сначала записывает и `fsync`-ит все временные payload, затем заменяет назначения
в порядке report -> artifact -> manifest. Manifest публикуется последним как
commit marker; ошибка процесса вызывает восстановление прежних байтов или
прежнего отсутствия уже заменённых файлов. Пути трёх выходов должны отличаться
друг от друга и от immutable-входов.

Следующая процедура не является предварительным просмотром sealed test: первый
запуск захватывает настроенный seal, а второй разрешён только потому, что
кандидат побайтно идентичен. Её следует запускать лишь для кандидата, которому
разрешено израсходовать namespace. Registry нельзя удалять или редактировать
для проверки изменённого кандидата — для него нужна явная ротация policy.

Для проверки byte-identical retraining нужны те же байты источников и config и
те же toolchain hashes. Идентичность машины в это условие **не входит**: с v21
снимок инструментария содержит только дайджесты кода и конфига, а описание
машины лежит отдельно, в `build-environment.json`. Среда, которая вычисляет то
же самое, воспроизводит те же байты, как бы она себя ни называла:

```bash
set -euo pipefail
(cd model/intent_v1/sources && sha256sum --check SHA256SUMS)
retrain_root="$(mktemp -d /tmp/keyswitch-intent-retrain.XXXXXX)"
trap 'rm -r -- "$retrain_root"' EXIT
for run in a b; do
  mkdir "$retrain_root/$run"
  PYTHONPATH=src python3 tools/train_intent_model_release.py \
    --artifact "$retrain_root/$run/layout_intent_v1.ksm" \
    --manifest "$retrain_root/$run/manifest.json" \
    --test-report "$retrain_root/$run/test-report.json" \
    --build-environment "$retrain_root/$run/build-environment.json"
done
cmp "$retrain_root/a/layout_intent_v1.ksm" \
    "$retrain_root/b/layout_intent_v1.ksm"
cmp "$retrain_root/a/manifest.json" "$retrain_root/b/manifest.json"
cmp "$retrain_root/a/test-report.json" "$retrain_root/b/test-report.json"
PYTHONPATH=src python3 tools/evaluate_intent_model.py \
  --artifact "$retrain_root/a/layout_intent_v1.ksm" \
  --manifest "$retrain_root/a/manifest.json" --strict
```

Успешные `cmp` подтверждают равенство всех трёх файлов побайтно. Четвёртый
файл, `build-environment.json`, намеренно в сравнение не входит: он описывает
машину и обязан различаться между машинами.

Обещание распространяется на любую среду, которая **считает то же самое**.
Другая сборка Python, другая версия libc, другой дистрибутив — всё это проходит
молча, если ни одно число не изменилось. Если изменилось, расхождение поймают
байты, а не название: веса входят в `candidate_sha256`, поэтому печать откажет
по весам. Чтобы расхождение можно было назвать, а не только заметить, рядом
работает `tools/environment_probe.py`:

```bash
PYTHONPATH=src python3 tools/environment_probe.py --explain libm \
  --against model/intent_v1/build-environment.json
```

Зонд — свидетельство, а не доказательство: пространство double шириной 2**64, а
его сетки конечны, и libm, разошедшийся вне сетки, изменит веса, не сдвинув ни
одной ячейки. Такой случай ловит только полный реплей — поэтому зонд никогда не
голосует за идентичность. Исчерпывающей удалось сделать только ячейку
`unicode`: она обходит все кодовые точки, потому что рантайм нормализует каждый
токен, а обновление базы Unicode — настоящее и тихое изменение поведения.

Версия установленного `onboard-data` на результат не влияет, потому что trainer
читает только frozen-копии. С v21 то же верно и для словарей Hunspell: они
заморожены в `sources/hunspell/`, а обучение к ним не обращается вовсе.

## Ограничения и обновление

- Модель покрывает только статическую пару US/RU.
- Данные лексические и синтетические; реальный поток, приложения, стиль и
  частота ошибок пользователя отличаются.
- Независимый sealed test предотвращает подгонку к отчёту, но не заменяет
  добровольную обезличенную/локальную обратную связь и длительный shadow mode.
- Локальный registry защищает штатные и CI-переобучения, конкурентные claim и
  ошибки пути config, но не является неуничтожимым журналом: владелец файловой
  системы может удалить всю локальную историю. Операционным append-only
  авторитетом должна быть закоммиченная запись registry в защищённой удалённой
  Git-истории с обязательным review; удаление или ротация требуют отдельного
  осознанного изменения policy.
- Новая схема признаков, split namespace или формат данных требуют
  согласования соответствующей feature/config/container version: смена только
  split namespace не требует повышения всех форматов. Существующий `intent_v1` нельзя молча
  переобучать с несовместимой семантикой. Текущие значения — feature schema v5,
  split namespace `keyswitch:intent-v28:physical-signature`, training config
  schema 13, KSLM schema 4 и внешний manifest schema 2.
- Повышение recall запрещено ценой нарушения precision, specificity или
  safety-гейтов из фиксированного config.
- Для false positives отчёт содержит наблюдаемое число, размер отрицательного
  среза, обычную верхнюю 95%-границу Wilson и точную статистическую границу,
  которой принято решение gate. Даже ноль наблюдений не означает нулевой
  real-world риск.
- Дополнительный Hunspell-срез лексически исключает все Onboard-unigrams и
  проверяет end-to-end detector с моделью и без неё минимум на 5000 EN и 5000 RU
  основах. Это не полностью независимый источник: Hunspell также входит в
  runtime language scorer, что явно маркируется в отчёте. Frozen hashes и
  размеры `.dic`/`.aff`, hashes результирующих lexical-disjoint и unknown-typo
  корпусов и полный набор trigger не позволяют незаметно заменить этот внешний
  регрессионный срез другим.

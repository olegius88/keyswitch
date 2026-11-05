# Цикл обучения модели

Все инструменты обучения и выпуска собраны в одной программе: `cargo build --profile train -p ks-train`, двоичный файл `target/train/ks-train`, и `ks-release` рядом с ним. Частные данные (корпуса, журнал прочитанных тестов, набор текста) лежат вне репозитория; пути к ним задаются параметрами.

## Что есть на машине

`ks-train doctor --tree . --corpus <корпус> --ledger <журнал>` показывает, какие шаги можно выполнить и чего не хватает.

## Модель контекста и модель префиксов

1. Заморозить корпус: `ks-train holdout ...`, затем `ks-train fitting ...`. Тест берётся из документов, которых не читал ни один прежний тест.
2. Обучить пару: `ks-train fit --tree . --corpus <корпус> --output <папка>` и `ks-train prefix --tree . --output <папка>`. Рецепты: `model/context_v3/recipe.json`, `model/prefix_v2/recipe.json`. Головы одной пары можно переобучить поверх выпущенной: `ks-train carry --pair ... --refit <голова> --output model/context_v3/frozen-base-<версия>.json`.
3. Сравнить с выпущенной парой на калибровке и разработочной части: `ks-train compare`, оценка последовательностей `ks-train evaluate --split development`, чат-стенд `ks-train devchat --split dev`.
4. Выпустить пару: `ks-train promote`. Команда сначала выполняет все проверки (незакоммиченные файлы, печати, тесты, те же тесты с кандидатом, набор текста владельца против прошлого выпуска, результат чат-стенда) и только потом один раз читает запечатанный тест. При успехе ставит файлы модели и пишет квитанцию `model/context_v3/release-receipt.json`. С `--check-only` останавливается перед чтением теста.
5. Проверить квитанцию: `ks-train verify --tree .`. Все числа и пороги пересчитываются заново; с `--report` пересчитываются и из отдельных случаев частного отчёта.

Печать закрепляет исходные тексты программы: после правки в `crates/` кандидат обучается заново.

## Другие модели и таблицы

- Границы слова: `ks-train boundary --tree . --freeze|--train|--test|--verify|--promote`.
- Орфотактика: `ks-train ortho --tree . --fit|--test|--verify|--promote`.
- Таблицы опечаток: `ks-train typo-frequency --tree . --private <папка> [--verify]`.
- Дополнение словаря: `ks-train supplement --tree . --verify` или `--input <файл>`.
- Детектор: `ks-train detector --tree . --sample 10000 --dictionary-sample 10000 --strict`.
- Модель намерений: см. ниже.
- Прогоны на движке: `ks-train engine boundary|prefix --tree . --verify|--refresh-runtime`.

## Основа корпуса контекста

Заморозки `holdout` и `fitting` читают основу, собранную один раз из закреплённых открытых источников. Новая основа нужна только при смене источников:

1. `ks-train base freeze --tree . --source-root <деревья UD> --output <ud0> --namespace <имя> --extra-exposure model/context_v1/scenarios.json --extra-exposure model/context_v1/holdout-2.json --extra-exposure model/context_v1/holdout-3.json`.
2. `ks-train base reconcile --tree . --corpus <ud0> --source-root <деревья UD> --report <файл> --output <ud1>`.
3. `ks-train base technical --tree . --source-directory <trixie> --ud-corpus <ud1> --extra-exposure model/context_v1/holdout-3.json --output <tech> --preview-report <файл>`.
4. `ks-train base merge --tree . --base <ud1> --technical <tech> --output <merged>`.
5. `ks-train base inventory --tree . --output <файл>`: что видела модель префиксов, тест ей отказывает.

Корпус префиксов: `ks-train prefix-corpus --tree .` собирает его заново и сверяет с файлами в `model/prefix_v1`, `--freeze <папка>` пишет новое поколение. Нужна переменная `KEYSWITCH_HUNSPELL_PATH=model/intent_v1/sources/hunspell`.

Входные данные орфографической модели: `ks-train ortho --tree . --inputs` собирает заново токены, ответ словарей и независимые наборы и сверяет их с замороженными файлами; `--freeze-inputs --output <папка>` пишет новое поколение. Словари веб-форм и прозы выводятся из больших публичных файлов, которых нет в репозитории: `ks-train ortho --tree . --lexicon-sources --web-source <cc.ru.300.vec.gz> --prose-source <папка дампа Wikisource>` сверяет их с замороженными. Новый независимый набор: `ks-train ortho --tree . --freeze-independent <описание.json> --files <папка файлов>`, до него не должно быть папки `model/ortho_v1/independent`; набор открывается один раз.

Семейства, которые больше не обучаются (`boundary_v1`, `context_v1`, `context_v2`, `ortho_v2`), лежат как история и как входные данные других шагов. `ks-train archive` сторожит их файлы. `ks-train retired boundary-v1|context-v2|ortho-v2|prefix-v1 --tree . [--replay]` проверяет их улики, а с `--replay` заново обучает и тестирует их и сравнивает результат с замороженными файлами байт в байт: кандидат, печать, отчёт, у `ortho-v2` ещё корпус, ответ словарей и метки. `retired context-corpus` пересобирает квитанцию разбиения фраз, `retired context-engine` гоняет три варианта context-v2 на сегодняшнем движке. У context-v1 нет файла модели, поэтому его переобучать не с чем сравнивать.

Ворота пакета: `ks-train gates --tree .` за 10 секунд проверяет улики всех установленных и отвергнутых моделей и не читает ни одной тестовой строки. Лицензии компонентов: `ks-train licenses --tree . [--verify]` пишет `packaging/third-party-licenses/` из `Cargo.lock`.

## Замер исправления опечаток

`ks-train typo-evaluation synthetic|clean|field|all --tree . --messages <папка сообщений> --manifest <.t/field/manifest.json> --out <папка> [--tune] [--utc-offset <секунды>]` читает только разработочные части и набор текста владельца. Смещение нужно, чтобы время записей журнала сравнивалось так же, как на машине, где они записаны. Списки слов уходят в частную папку.

## Модель намерений

Модель читает пары «слово в раскладке и оно же в другой», её источник заморожен в `model/intent_v1/sources`: списки слов Onboard и словари Hunspell. Системные словари на результат не влияют.

- Проверить установленную модель: `ks-train intent --tree . --verify` заново обучает её и сверяет байт в байт файл модели, манифест, отчёт теста, кандидата и ответ теста.
- Строгая проверка: `ks-train intent --tree . --evaluate --strict --output <файл>`. Она сверяет происхождение, пересчитывает все записи манифеста через настоящий код программы и прогоняет настоящий детектор на внешних корпусах и на всех крайних значениях контекста. Занимает около минуты, поэтому отчёт не кэшируется.
- Квитанции внешних корпусов, до обучения и без модели: `ks-train intent --tree . --freeze --output <файл>` и `--preseal --output <файл>`.
- Обучить новую модель: сначала сменить поколение (`SPLIT_NAMESPACE`, `HARD_NEGATIVE_POLICY`, `ROLE_NAMESPACE` в `crates/ks-train/src/intent/dataset.rs`, `HOLDOUT_*_NAMESPACE` в `external.rs`, `registry_path` и пути в `model/intent_v1/config.json`), записать новые квитанции и их хэши в конфигурацию, затем `ks-train intent --tree . --fit`. Закрытый тест открывается один раз: кандидат записывается в `seal-registry-vN.json` до чтения теста, а ответ теста в `seal-outcome-vN.json` до записи любых файлов. Изменившийся кандидат требует нового поколения.
- Отвергнуть кандидата после провала строгой проверки: `ks-train intent --tree . --reject --version N --strict-report <файл>`.
- Что считает эта машина: `ks-train intent --probe`. Отчёт строгой проверки и `build-environment.json` несут этот замер, чтобы расхождение можно было объяснить. На подпись модели он не влияет.

Манифест, записанный этим обучением, содержит `toolchain.implementation: "rust"`, и его хэши относятся к исходникам из `crates/`. У установленной модели этого поля нет: её обучил прежний тренер на Python, чьих исходников в дереве нет. Её хэши кода проверка только читает, вместо них работает пересчёт байт в байт, а хэши данных (защищённые слова, квитанция) сверяются как обычно.

## Сообщения для обучения

`ks-train messages` строит сообщения из дампа ru.stackoverflow, Tatoeba и UD Russian-Taiga. `terms`, `alnum`, `short` считают частоты, `select` отбирает сообщения одного класса, `capture` записывает вопросы движка, `sources` пишет авторов, `devchat-messages` собирает комментарии для чат-стенда. Файлы частей `test` не печатаются и не открываются для чтения глазами.

## Выпуск

`ks-release --version X.Y.Z --field-root <дерево с набором текста> --field-released <отчёты прошлого выпуска>`: версия во всех файлах, закрытие раздела в `CHANGELOG.md`, проверки, коммит, тег, отправка, ожидание сборки и проверка опубликованных файлов. Сначала пишутся записи в `CHANGELOG.md` и `RELEASE_NOTES.md`, иначе команда откажется работать. `--dry-run` ничего не пишет.

Тег `vX.Y.Z-rcN` (`ks-release --candidate N`) собирается в предварительный выпуск: проверка обновлений установленных программ его не видит, и APT-репозиторий не обновляется. Тег `vX.Y.Z` заменяет опубликованную версию у всех: сначала обновляется APT-репозиторий, потом появляется выпуск со статусом `latest`.

APT-репозиторий: `ks-train apt-build --tree . --repository <папка> --package <файл.deb>` и `ks-train apt-verify --tree . --repository <папка> --expect-version <версия>`.

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
- Прогоны на движке: `ks-train engine boundary|prefix --tree . --verify|--refresh-runtime`.

## Сообщения для обучения

`ks-train messages` строит сообщения из дампа ru.stackoverflow, Tatoeba и UD Russian-Taiga. `terms`, `alnum`, `short` считают частоты, `select` отбирает сообщения одного класса, `capture` записывает вопросы движка, `sources` пишет авторов, `devchat-messages` собирает комментарии для чат-стенда. Файлы частей `test` не печатаются и не открываются для чтения глазами.

## Выпуск

`ks-release --version X.Y.Z --field-root <дерево с набором текста> --field-released <отчёты прошлого выпуска>`: версия во всех файлах, закрытие раздела в `CHANGELOG.md`, проверки, коммит, тег, отправка, ожидание сборки и проверка опубликованных файлов. Сначала пишутся записи в `CHANGELOG.md` и `RELEASE_NOTES.md`, иначе команда откажется работать. `--dry-run` ничего не пишет.

Тег собирается в предварительный выпуск (`--prerelease --latest=false` в `.github/workflows/rust.yml`): проверка обновлений установленных программ его не видит. Снять эти два флага значит решить, что эта сборка заменяет опубликованную.

APT-репозиторий: `ks-train apt-build --tree . --repository <папка> --package <файл.deb>` и `ks-train apt-verify --tree . --repository <папка> --expect-version <версия>`.

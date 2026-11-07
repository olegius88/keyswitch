# Проверка, сборка и выпуск

Все команды — из корня репозитория. Разделы проверок не делают коммит, push или выпуск. Последний раздел меняет Git и внешнее состояние: запускайте его только после явного решения о выпуске.

## Быстрые проверки

```bash
git status --short
cargo test -p ks-text -p ks-backend -p ks-config -p ks-lang -p ks-context -p ks-engine -p ks-replay -p ks-updates -p ks-ui -p ks-win -p ks-train
cargo test --release -p ks-replay
for target in x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc; do cargo check -p ks-win --target "$target"; done
```

`ks-replay` прогоняет движок по записям поведения в `crates/ks-replay`: что нажато, что вернул бэкенд, какие события вышли. Тесты программы на Linux (`-p ks-linux`) нужны с файлами разработки GTK и под `xvfb-run dbus-run-session`, тесты на macOS (`-p ks-mac`) идут на Mac.

## Строгие типы вместо дерева JSON

Дерево `serde_json::Value` — объединение всех типов, как `any` в TypeScript, а `value["key"]` — доступ по строковому ключу. Данные из файлов читаются в структуры с полями своих типов, а файлы пишутся из таких структур.

- Запрещено Clippy (`clippy.toml`, уровни `disallowed_types`, `disallowed_macros` и `disallowed_methods` в `Cargo.toml`): типы `serde_json::Value`, `serde_json::Map` и `std::any::Any`, макрос `serde_json::json!`, методы, которые читают или строят дерево (`get`, `as_str`, `as_object`, `Value::String(..)`, `Map::insert` и другие), и чтение или запись JSON мимо `ks_text::json`: `serde_json::from_*`, `to_*`, `Deserializer::from_*`, `downcast_ref` у `Any`. Опечатка в пути из этих списков сборку не останавливает: Clippy только предупреждает, что путь `does not refer to a reachable …`, а путь в крейт, от которого код не зависит, молча пропускает. Поэтому новый путь проверяют по выводу Clippy.
- JSON читается и пишется только через `ks_text::json`. `loads` читает документ в типизированное значение и, как `json.loads`, отказывает, если после документа есть что-то, кроме пробелов. `loads_seed` и `read_seed` делают то же для собственного `DeserializeSeed` из памяти или из потока: так читаются большие файлы моделей. `dumps` пишет любое значение с `Serialize` теми же байтами, что `json.dumps` Python, через `ks_text::pyjson`.
- Исключение: модуль, для которого дерево и есть честный тип (запись `json.dumps` по дереву, сравнение произвольных деревьев), начинается с `#![expect(clippy::disallowed_…, reason = "…")]` и записан в список `exceptions` файла `crates/ks-train/tests/data/strict_typing.json` с той же причиной. Сейчас в списке `crates/ks-text/src/pyjson.rs` и `crates/ks-text/src/json.rs`. Исключение на одну функцию или строку не допускается.
- Ещё не переведённые крейты и тесты несут временный `#![expect(...)]` в корне крейта. Его причина начинается со слова `temporary` и называет шаг перевода: 1 — `ks-config`, 2a — `ks-engine`, 2b — `ks-lang` и `ks-context`, 2c — `ks-linux`, `ks-win`, `ks-mac`, `ks-ui` и `ks-updates`, 3 — `ks-replay`, 4 и 5 — `ks-train`, 6 — LogCourier, 7 — тесты каждого крейта вместе с ним. С `unfulfilled_lint_expectations = "deny"` такой `expect` перестаёт компилироваться в тот день, когда крейт чист, и его приходится снять.
- То, чего не видит Clippy, считает тест `cargo test -p ks-train --test strict_typing`: во всех отслеживаемых файлах `.rs` в `crates/`, в `src` без элементов `#[cfg(test)]`, в остальных целиком. `value_tokens` — пути `serde_json::`, кроме `serde_json::Number`, и имена `Value`, `Map`, `json!`, если файл берёт их из `serde_json`; `string_key_index` — индекс строковым литералом (`x["key"]`, но не массив `vec!["a"]`); `literal_compares` — сравнение имени со строкой из двух и более знаков (`mode == "early"`: строка вместо перечисления); `expect_roots` — корни крейтов с временным `expect`. Числа записаны по файлам и по крейтам в `strict_typing.json`; в итог крейта модули-исключения не входят, их строки считаются под их собственным файлом. Рост числа роняет тест с именем файла и строками, снижение — пока его не запишут. При расхождении тест кладёт измеренную запись в `target/tmp/strict_typing.json`: её копируют поверх записи, когда изменение сделано намеренно.
- Проверка, как в CI: `cargo clippy --workspace --all-targets` на Linux, для кода Windows `cargo clippy -p ks-win --all-targets --target x86_64-pc-windows-msvc` (и `i686-pc-windows-msvc`, `aarch64-pc-windows-msvc`), для кода Mac `cargo clippy -p ks-mac --all-targets --target x86_64-apple-darwin`.

## Модели

Обучение и проверка — в `ks-train`, подробности в [цикле обучения](model-cycle.md).

```bash
cargo build --profile train -p ks-train
K=target/train/ks-train
$K verify --tree .                                # квитанция установленной пары, числа пересчитываются
$K boundary --tree . --verify                     # модель границ байт в байт
$K ortho --tree . --verify                        # орфографическая модель байт в байт
$K ortho --tree . --inputs                        # её входные данные из замороженных источников
$K intent --tree . --verify                       # модель намерений: файл, манифест и отчёт теста байт в байт
$K intent --tree . --evaluate --strict --output strict.json
$K archive --tree .                               # файлы семейств, которые больше не обучаются
$K detector --tree . --sample 10000 --dictionary-sample 10000 --strict
dbus-run-session -- $K engine boundary --tree . --verify
dbus-run-session -- $K engine prefix --tree . --verify
```

Модель намерений читает только замороженные в `model/intent_v1/sources` списки слов и словари Hunspell, поэтому её проверка не зависит от установленных в системе словарей; нужна библиотека `libhunspell-1.7-0`. `detector` проверяет то, что установлено: для его чисел нужны `hunspell-en-us`, `hunspell-ru` и `onboard-data`.

Строгая проверка модели намерений занимает около минуты, поэтому её отчёт не кэшируется, а считается заново.

## Покрытие тестами

Каждая строка и каждое ветвление программы должны быть выполнены хотя бы одним тестом. Замер идёт на всех трёх системах, результаты складываются: код Windows считается покрытым прогоном на Windows, код Mac — на Mac.

```bash
packaging/coverage.sh coverage-linux.json
cargo run --release -p ks-train --bin ks-train -- coverage-gate --tree . --report coverage-linux.json --report coverage-Windows.json --report coverage-macOS.json
```

- `packaging/coverage.sh` запускает все тесты, живые тесты на рабочем столе и повторы обучения. Ветвления считает ночная сборка компилятора, закреплённая в сценарии; сама программа собирается стабильной.
- Флаг `-Zcross-crate-inline-threshold=never` обязателен. Без него компилятор встраивает мелкие функции библиотеки в тот, кто их вызывает, а программа, которая их не вызывает, оставляет пустую запись. `llvm-cov` может прочитать её первой, и проверенный код выглядит невыполненным.
- Файлы, которые говорят с системой напрямую, перечислены в `packaging/coverage-native.txt`; их проверяют живые тесты.
- `crates/{ks-lang,ks-context,ks-replay}/tests/fixtures` хранит ответы последней версии на Python (0.66.2) на открытых словах: словари программы и несколько фраз, в правильной и в неправильной раскладке. Тесты сверяют с ними код на Rust при каждом прогоне. Настоящий набор текста в репозиторий не попадает.
- `crates/ks-replay/tests/walk.rs` и такие же обходы подсистем ввода случайно, но одинаково при каждом прогоне смешивают набор с тем, что человек делает между словами: другие окна, паузы, горячие клавиши, правки, сбои окна. Проверяют, что ничего не останавливается.

## Настоящие тесты на рабочем столе

Обычный `cargo test` их пропускает и пишет об этом: им нужны настоящие окна, клавиатура и шина.

- Linux, X11: `packaging/e2e.sh [пакет.deb]`. Отдельный X-сервер с раскладками `us,ru`, отдельные шины: ничего не попадает на рабочий стол. Набор в поле GTK (15 случаев), подсказка правила, окно правила, смена раскладки из меню, переключение по префиксу; значок в панели; чтение поля через AT-SPI, выделение и поле пароля. С пакетом: сама программа из пакета, её меню и выученное правило. Нужны `xvfb`, `dbus`, `setxkbmap`, `at-spi2-core`, `gdbus`.
- Содержимое пакета: `ks-train deb-check --tree . пакет.deb`. Модели те же байты, что в дереве, лицензии на месте, источник APT и ключ, ничего не включается при установке, программа запускается и загружает свою модель.
- Wayland: `cargo test --release -p ks-linux --test e2e_wayland` в настоящем сеансе GNOME, KDE или sway, в виртуальной машине, потому что тест забирает клавиатуру. `KS_E2E_DESKTOP` называет стол. С `KS_E2E_WAYLAND_SOURCE=1` движок работает в процессе теста: 15 случаев набора, отложенный Enter, горячие клавиши, щелчок мышью, Caps Lock и его лампочка, подключение клавиатуры на ходу, поле пароля, правило, префикс, блокировка сеанса. С `KS_E2E_WAYLAND_BINARY=/usr/bin/keyswitch` проверяется пакет, в том числе зависший главный процесс и убитый помощник ввода. Доступ включает `sudo keyswitch --enable-wayland-input`.
- Windows: `cargo test --release -p ks-win --test e2e_windows` с `KS_E2E_BINARY`, путём к установленной программе. Перехватчик, затем настоящее окно: обе стороны исправления, правило через двойной Pause, Enter не утекает в поле, исправление до Enter. В CI идёт на x64, x86 и Arm.
- macOS: `packaging/e2e-mac.command`, открыть в Терминале с разрешением «Универсальный доступ». TextEdit на переднем плане, нажатия на уровне HID: исправление, Shift, Caps Lock, отмена, команды выделения, опечатки. Результат в `e2e-mac.log`.

## Записанный настоящий набор текста

Перед выпуском записанный настоящий набор текста воспроизводится деревом последнего выпуска и проверяемым деревом: слово, которое выпущенная версия делала правильно, а новая делает неправильно, останавливает выпуск. Батареи частные и в git не входят, барьер печатает только числа.

```bash
cargo build --profile parity -p ks-replay --bins
target/parity/field-gate --root <дерево с набором> --released <отчёты прошлого выпуска> [--speller]
```

`field-gate` запускает `ks-replay`, который лежит рядом с ним, поэтому собирать нужно оба. Без `--speller` повтор идёт без Hunspell, с ним — со словарями Hunspell, как работает установленная программа; тогда и отчёты прошлого выпуска должны быть сняты со словарями.

## Сборка

| Что | Команда | Где |
| --- | --- | --- |
| DEB | `packaging/build-rust-deb.sh dist` | Linux, нужны файлы разработки GTK 4 |
| Windows: установщик, ZIP | `packaging/build-rust.ps1 -Architecture x64` | Windows, нужны цель MSVC и Inno Setup |
| macOS | `packaging/build-rust-macos.sh dist` | Mac, подпись через `KEYSWITCH_SIGN_IDENTITY` |

Workflow [`ci.yml`](../.github/workflows/ci.yml) делает всё это на каждый коммит: тесты, повторные обучения, установку и запуск установщика Windows на каждой архитектуре, установку DEB в чистую Ubuntu, запуск приложения macOS и его нотаризацию. На теге `vX.Y.Z-rcN` он публикует файлы как предварительный выпуск, который не видят обновления; на теге `vX.Y.Z` — как выпуск для всех и обновляет репозиторий APT. Публикация ждёт всех заданий, в том числе проверки покрытия.

## Выпуск

```bash
target/train/ks-release --version X.Y.Z --dry-run
target/train/ks-release --version X.Y.Z --field-root <дерево с набором текста> --field-released <отчёты прошлого выпуска>
```

Команда обновляет версию, закрывает раздел `Unreleased` в `CHANGELOG.md`, проверяет `RELEASE_NOTES.md`, запускает проверки, коммитит, ставит тег, отправляет и ждёт workflow. Уже отправленный тег или выпуск не откатывается. APT: `ks-train apt-build` и `ks-train apt-verify`, см. [репозиторий APT](apt-repository.md).

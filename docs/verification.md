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

## Модели

## Настоящие тесты на рабочем столе

Обычный `cargo test` их пропускает и пишет об этом: им нужны настоящие окна, клавиатура и шина.

- Linux, X11: `packaging/e2e.sh [пакет.deb]`. Отдельный X-сервер с раскладками `us,ru`, отдельные шины: ничего не попадает на рабочий стол. Набор в поле GTK (15 случаев), подсказка правила, окно правила, смена раскладки из меню, переключение по префиксу; значок в панели; чтение поля через AT-SPI, выделение и поле пароля. С пакетом: сама программа из пакета, её меню и выученное правило. Нужны `xvfb`, `dbus`, `setxkbmap`, `at-spi2-core`, `gdbus`.
- Содержимое пакета: `ks-train deb-check --tree . пакет.deb`. Модели те же байты, что в дереве, лицензии на месте, источник APT и ключ, ничего не включается при установке, программа запускается и загружает свою модель.
- Wayland: `cargo test --release -p ks-linux --test e2e_wayland` в настоящем сеансе GNOME, KDE или sway, в виртуальной машине, потому что тест забирает клавиатуру. `KS_E2E_DESKTOP` называет стол. С `KS_E2E_WAYLAND_SOURCE=1` движок работает в процессе теста: 15 случаев набора, отложенный Enter, горячие клавиши, щелчок мышью, Caps Lock и его лампочка, подключение клавиатуры на ходу, поле пароля, правило, префикс, блокировка сеанса. С `KS_E2E_WAYLAND_BINARY=/usr/bin/keyswitch` проверяется пакет, в том числе зависший главный процесс и убитый помощник ввода. Доступ включает `sudo keyswitch --enable-wayland-input`.
- Windows: `cargo test --release -p ks-win --test e2e_windows` с `KS_E2E_BINARY`, путём к установленной программе. Перехватчик, затем настоящее окно: обе стороны исправления, правило через двойной Pause, Enter не утекает в поле, исправление до Enter. В CI идёт на x64, x86 и Arm.
- macOS: `packaging/e2e-mac.command`, открыть в Терминале с разрешением «Универсальный доступ». TextEdit на переднем плане, нажатия на уровне HID: исправление, Shift, Caps Lock, отмена, команды выделения, опечатки. Результат в `e2e-mac.log`.

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

## Набор текста владельца

Перед выпуском набор владельца воспроизводится деревом последнего выпуска и проверяемым деревом: слово, которое выпущенная версия делала правильно, а новая делает неправильно, останавливает выпуск. Батареи частные и в git не входят, барьер печатает только числа.

```bash
cargo run --profile parity --bin field-gate -- --root <дерево с набором> --released <отчёты прошлого выпуска>
```

## Сборка

| Что | Команда | Где |
| --- | --- | --- |
| DEB | `packaging/build-rust-deb.sh dist` | Linux, нужны файлы разработки GTK 4 |
| Windows: установщик, ZIP | `packaging/build-rust.ps1 -Architecture x64` | Windows, нужны цель MSVC и Inno Setup |
| macOS | `packaging/build-rust-macos.sh dist` | Mac, подпись через `KEYSWITCH_SIGN_IDENTITY` |

Workflow [`rust.yml`](../.github/workflows/rust.yml) делает всё это на каждый коммит: тесты, повторные обучения, установку и запуск установщика Windows на каждой архитектуре, установку DEB в чистую Ubuntu, запуск приложения macOS и его нотаризацию. На теге он публикует файлы как предварительный выпуск.

## Выпуск

```bash
target/train/ks-release --version X.Y.Z --dry-run
target/train/ks-release --version X.Y.Z --field-root <дерево с набором текста> --field-released <отчёты прошлого выпуска>
```

Команда обновляет версию, закрывает раздел `Unreleased` в `CHANGELOG.md`, проверяет `RELEASE_NOTES.md`, запускает проверки, коммитит, ставит тег, отправляет и ждёт workflow. Уже отправленный тег или выпуск не откатывается. APT: `ks-train apt-build` и `ks-train apt-verify`, см. [репозиторий APT](apt-repository.md).

# KeySwitch

[**Русский**](README.md) · [English](README.en.md)

[![GitHub release](https://img.shields.io/github/v/release/olegius88/keyswitch)](https://github.com/olegius88/keyswitch/releases/latest)
[![Tests](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml)
[![Debian package](https://github.com/olegius88/keyswitch/actions/workflows/release.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/release.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KeySwitch — настольное приложение для Ubuntu/Xubuntu, которое автоматически
исправляет слово, набранное в неверной раскладке. По назначению оно похоже на
Punto Switcher и EveryLang, но работает локально и использует системную пару
раскладок XKB.

## Что уже работает

- глобальное наблюдение за вводом во всех обычных X11-приложениях;
- автоматическое распознавание английского и русского слова после пробела,
  Enter, Tab или знака препинания;
- ансамблевое распознавание по частотному словарю, морфологии Hunspell,
  символьным n-граммам и недавнему контексту;
- консервативная защита URL, путей, кода, аббревиатур, неоднозначных слов и
  распространённых технических терминов;
- локальное самообучение: повторное ручное преобразование создаёт правило, а
  отмена ложного исправления запоминает запрет;
- переключение XKB-группы и исправление уже напечатанного слова;
- сохранение регистра: `Ghbdtn` превращается в `Привет`;
- ручное преобразование последнего слова (`Pause`);
- отмена последнего исправления за 10 секунд (`Ctrl+Alt+Z`);
- глобальная пауза (`Ctrl+Alt+P`);
- выбор приложений-исключений прицелом по активному окну, из каталога
  установленных приложений или вручную по `WM_CLASS`;
- локальная история только выполненных исправлений;
- уведомления, звук, светлая/тёмная тема, XDG Autostart;
- живой системный индикатор раскладки `EN/RU` или флагами стран; щелчок левой
  кнопкой открывает меню с настройками, паузой, звуком, уведомлениями, историей,
  исключениями, сведениями о программе и выходом;
- полное окно настроек с обзором, тестовым полем и диагностикой backend.

Полный поток клавиатуры не записывается. В памяти находится текущее слово, а в
историю при включённой опции попадают только пары вида `ghbdtn → привет`.

## Быстрый запуск

Текущая проверенная среда: Ubuntu 26.04.1 LTS, XFCE, X11, системные раскладки
`us,ru`.

```bash
git clone https://github.com/olegius88/keyswitch.git
cd keyswitch
./run.sh
```

В открывшемся окне есть тестовое поле. Переключитесь на EN, напечатайте
`ghbdtn` и нажмите пробел: поле должно содержать `привет `. Для обратной
проверки включите RU и на тех же физических клавишах напечатайте `hello`:
появившееся `руддщ ` будет заменено на `hello `.

Проверка системного backend без запуска окна:

```bash
./run.sh --diagnose
```

## Установка DEB-пакета

Скачайте `keyswitch_0.2.0_amd64.deb` со страницы
[последнего выпуска](https://github.com/olegius88/keyswitch/releases/latest), затем:

```bash
sudo apt install ./keyswitch_0.2.0_amd64.deb
```

Пакет установит системные зависимости и добавит KeySwitch в меню приложений.
Приложение внутри пакета скомпилировано Nuitka в архитектурный ELF-бинарник:
в `/usr/lib/keyswitch` нет исходных `.py` или байткода `.pyc`, а пакет не
зависит от системного интерпретатора Python. В состав нативного runtime входит
`libpython`, поэтому пакет для `amd64` нельзя устанавливать на другую
архитектуру.

## Установка из исходников для текущего пользователя

```bash
./install.sh
keyswitch
```

Установщик не требует root: он размещает приложение, launcher, desktop-файл и
иконки внутри `~/.local`. После первого запуска XDG Autostart включён по
умолчанию: после перезагрузки KeySwitch стартует при следующем входе в рабочий
стол. Это можно изменить в разделе «Внешний вид и система».

Удаление программы (настройки и история сохраняются):

```bash
./uninstall.sh
```

## Системные зависимости

На стандартной установке текущей Ubuntu необходимые компоненты уже могут быть
установлены. Если проверка установщика сообщит об их отсутствии:

```bash
sudo apt install python3-gi python3-dbus gir1.2-gtk-4.0 gir1.2-adw-1 \
  libx11-6 libxtst6 libxkbcommon0 libhunspell-1.7-0 \
  hunspell-en-us hunspell-ru onboard-data
```

Активную пару XKB можно проверить командой:

```bash
setxkbmap -query
```

В этой версии движок берёт две первые группы. Для ожидаемого сценария вывод
должен содержать `layout: us,ru` либо ту же пару в обратном порядке; языковые
модели в настройках пока привязаны к порядку EN, RU.

## Настройки и данные

- настройки: `~/.config/keyswitch/config.json`;
- история: `~/.local/share/keyswitch/history.jsonl`;
- явно выученные правила и отменённые исправления:
  `~/.local/share/keyswitch/learning.json`;
- необязательные пользовательские словари Hunspell:
  `~/.local/share/keyswitch/dictionaries/<locale>.aff/.dic`;
- журнал ошибок/запуска: `~/.local/share/keyswitch/keyswitch.log`;
- автозапуск: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

Менеджеры паролей `KeePassXC`, `1Password` и `Bitwarden` добавлены в исключения
по умолчанию. X11 не сообщает глобальному наблюдателю семантику конкретного
поля, поэтому для других чувствительных приложений следует добавить их
`WM_CLASS` на странице «Исключения».

## Разработка и проверка

Весь Python-код приложения, тестов и утилит проходит усиленный профиль
`mypy --strict`: запрещены нетипизированные определения, явные `Any`, `Any` из
неописанных импортов и нетипизированные декораторы; дополнительно проверяются
недостижимые ветви, потенциально неопределённые значения и неиспользованные
awaitable. Локальный запуск на Ubuntu:

```bash
sudo apt install python3-pip
./tools/install-typing-tools.sh .typing
KEYSWITCH_TYPING_ROOT=.typing ./tools/typecheck.sh
```

Чтобы не ослаблять проверку на границе `dbus-python`, репозиторий содержит
узкий типовой контракт только используемой части API в `typings/dbus`.

Полный набор unit- и GTK interaction-тестов с обязательным 100% line/branch
coverage (для GTK нужен активный X11 display или Xvfb):

```bash
./tests/run_coverage.sh
```

Для headless-запуска используется та же команда внутри `dbus-run-session` и
`xvfb-run`; именно так набор выполняется в GitHub Actions. Отчёт не позволит
сборке продолжиться, если покрытие опустится ниже 100%.

Воспроизводимый прогон 40 000 частых и широких словарных слов, а также сложных
защитных случаев:

```bash
PYTHONPATH=src tools/evaluate_detector.py \
  --sample 10000 --dictionary-sample 10000 --strict
```

Настоящий сквозной тест в активной X11-сессии:

```bash
PYTHONPATH=src python3 tests/e2e_x11.py
```

Успешный тест печатает `E2E_OK` после четырёх фактических исправлений внутри GTK
Entry. Отдельный интеграционный тест поднимает настоящий StatusNotifierItem и
DBusMenu в изолированной сессии D-Bus:

```bash
dbus-run-session -- env PYTHONPATH=src python3 tests/e2e_tray_menu.py
```

Успешный тест печатает `TRAY_MENU_E2E_OK` после регистрации индикатора, чтения
всех пунктов меню и активации команды «Настройки». Архитектура и критерии
описаны в [DESIGN.md](DESIGN.md), а сравнение с
существующими решениями и обоснование модели — в
[обзоре механизма распознавания](docs/detection-research.md).

Для воспроизводимой нативной сборки DEB-пакета:

```bash
sudo apt install build-essential ccache patch patchelf python3-dev python3-pip
./tools/install-build-tools.sh .nuitka
KEYSWITCH_NUITKA_ROOT=.nuitka ./packaging/build-deb.sh
package="dist/keyswitch_0.2.0_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
```

После остановки уже запущенного экземпляра полный E2E именно бинарника из
пакета можно выполнить в активной X11-сессии:

```bash
dbus-run-session -- ./tools/run-native-e2e.sh "$package"
```

Он проверяет четыре реальные коррекции в GTK Entry, смену XKB-группы, историю,
регистрацию StatusNotifierItem и состав всплывающего DBusMenu. При отправке
тега `v*` GitHub Actions запускает coverage, детекторные барьеры, исходный и
запакованный нативный E2E под Xvfb, проверяет пакет через `lintian`, формирует
`SHA256SUMS` и публикует оба файла в GitHub Release.

## Ограничения

- Backend текущей версии предназначен для X11. В нативной Wayland-сессии
  приложение покажет явную ошибку диагностики и не будет имитировать поддержку.
- Автоматические языковые модели по умолчанию рассчитаны на пару EN/RU.
- Приложения с собственным нестандартным вводом или удалённые рабочие столы
  могут по-разному обрабатывать синтетические XTEST-события; их удобно добавить
  в исключения.

## Лицензия

KeySwitch распространяется на условиях
[GNU General Public License 3.0 или более поздней версии](LICENSE).

## Первичные спецификации

- [X.Org RECORD Extension Library](https://www.x.org/releases/current/doc/libXtst/recordlib.pdf)
  — создание контекста и получение событий;
- [X.Org XTEST Extension Library](https://www.x.org/releases/current/doc/libXtst/xtestlib.pdf)
  — воспроизведение KeyPress/KeyRelease;
- [X.Org XKB Library Specification](https://www.x.org/releases/current/doc/libX11/XKB/xkblib.html)
  — группы, уровни Shift и преобразование keycode в keysym;
- [Freedesktop Autostart Specification](https://specifications.freedesktop.org/autostart/0.5/)
  — пользовательский файл `~/.config/autostart/*.desktop`;
- [Freedesktop StatusNotifierItem](https://specifications.freedesktop.org/status-notifier-item/latest-single/)
  — интеграция значка с панелью;
- [Canonical DBusMenu interface](https://sources.debian.org/src/libdbusmenu/18.10.20180917~bzr492%2Brepack1-2/libdbusmenu-glib/dbus-menu.xml/)
  — нативное всплывающее меню системного индикатора;
- [Nuitka User Manual](https://nuitka.net/user-documentation/user-manual.html)
  — компиляция приложения в standalone-бинарник;
- [официальные release notes Ubuntu 26.04 LTS](https://documentation.ubuntu.com/release-notes/26.04/).

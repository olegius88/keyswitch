# KeySwitch

[**Русский**](README.md) · [English](README.en.md)

[![GitHub release](https://img.shields.io/github/v/release/olegius88/keyswitch)](https://github.com/olegius88/keyswitch/releases/latest)
[![Tests](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/tests.yml)
[![Native packages](https://github.com/olegius88/keyswitch/actions/workflows/release.yml/badge.svg)](https://github.com/olegius88/keyswitch/actions/workflows/release.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KeySwitch — настольное приложение для Windows 10/11 x64 и Ubuntu/Xubuntu X11,
которое автоматически исправляет слово, набранное в неверной раскладке. По
назначению оно похоже на Punto Switcher и EveryLang, но работает локально и
использует системную пару раскладок EN/RU.

## Что уже работает

- глобальное наблюдение за вводом через `WH_KEYBOARD_LL` в Windows и XRecord в
  обычных X11-приложениях Linux;
- автоматическое распознавание английского и русского слова через 1,5 секунды
  без ввода, а также после пробела, Enter, Tab или знака препинания; коррекцию
  по паузе можно отдельно отключить в настройках;
- ансамблевое распознавание по частотному словарю, морфологии Hunspell,
  символьным n-граммам и недавнему контексту;
- консервативная защита URL, путей, кода, аббревиатур, неоднозначных слов и
  распространённых технических терминов;
- локальное самообучение: после ручного преобразования по `Pause/Break`
  появляется подсказка над полем ввода; `Enter` сразу добавляет слово в правила,
  `Esc` отклоняет предложение, а отмена ложного исправления запоминает запрет;
- переключение системной раскладки и исправление уже напечатанного слова через
  Win32 `SendInput` либо XTEST;
- уважение к ручной смене раскладки: первое завершённое слово после выбора
  языка пользователем остаётся без автокоррекции; поведение можно отключить;
- сохранение регистра: `Ghbdtn` превращается в `Привет`;
- ручное преобразование последнего слова (`Pause`);
- отмена последнего исправления за 10 секунд (`Ctrl+Alt+Z`);
- глобальная пауза (`Ctrl+Alt+P`);
- выбор приложений-исключений прицелом по активному окну, из каталога
  установленных приложений, через пикер `.exe` в Windows или вручную;
- локальная история только выполненных исправлений;
- уведомления, звук, светлая/тёмная тема и автозагрузка после входа в ОС;
- живой системный индикатор раскладки `EN/RU` или флагами стран; щелчок левой
  или правой кнопкой открывает меню с настройками, паузой, звуком,
  уведомлениями, историей, исключениями, сведениями о программе и выходом;
- нативное для платформы полное окно настроек с обзором, тестовым полем,
  автокоррекцией, горячими клавишами, исключениями, историей и диагностикой;
- защита от запуска второго экземпляра: повторный запуск активирует уже
  открытое окно KeySwitch.

Полный поток клавиатуры не записывается. В памяти находится текущее слово, а в
историю при включённой опции попадают только пары вида `ghbdtn → привет`.

## Установка в Windows

Скачайте `KeySwitch-Setup-0.4.0-x64.exe` со страницы
[последнего выпуска](https://github.com/olegius88/keyswitch/releases/latest) и
запустите его. Установка выполняется для текущего пользователя в
`%LOCALAPPDATA%\Programs\KeySwitch` и не требует прав администратора. В выпуск
также входит переносимый архив `KeySwitch-0.4.0-windows-x64.zip`.

После запуска KeySwitch появится в области уведомлений. Левый или правый щелчок
по `EN/RU` либо флагу открывает меню. В разделе «Внешний вид и система» можно
отключить автозагрузку, запуск свёрнутым, индикатор, звук и уведомления.

Для запуска из исходников в Windows:

```powershell
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[windows]"
.venv\Scripts\keyswitch
```

Для полной модели распознавания при запуске из исходников задайте
`KEYSWITCH_MODEL_PATH` равным каталогу с `en_US.lm` и `ru_RU.lm` из пакета
Onboard. В Setup EXE и переносимый ZIP эти модели включаются автоматически.

## Быстрый запуск в Ubuntu

Текущая проверенная среда: Ubuntu 26.04.1 LTS, XFCE, X11, системные раскладки
`us,ru`.

```bash
git clone https://github.com/olegius88/keyswitch.git
cd keyswitch
./run.sh
```

В открывшемся окне есть тестовое поле. Переключитесь на EN, напечатайте
`ghbdtn` и остановитесь: примерно через 1,5 секунды поле должно содержать
`привет`, а активной станет раскладка RU. Можно также нажать пробел для
немедленной проверки. Для обратного направления включите RU и на тех же
физических клавишах напечатайте `hello`.

Чтобы обучить KeySwitch собственному исключению, наберите слово и нажмите
`Pause/Break`. После ручной замены над местом ввода появится вопрос «Добавить
слово в правила переключения? Enter - ДА». Нажатие `Enter` немедленно включает
правило для следующих вводов этого слова, `Esc` закрывает предложение. Весь
механизм отключается переключателем «Локальное обучение»; если подсказку не
подтверждать, правило по-прежнему может активироваться после заданного в
настройках количества повторных ручных преобразований.

Проверка системного backend без запуска окна:

```bash
./run.sh --diagnose
```

## Установка DEB-пакета

Скачайте `keyswitch_0.4.0_amd64.deb` со страницы
[последнего выпуска](https://github.com/olegius88/keyswitch/releases/latest), затем:

```bash
sudo apt install ./keyswitch_0.4.0_amd64.deb
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
sudo apt install at-spi2-core python3-gi python3-dbus gir1.2-gtk-4.0 gir1.2-adw-1 \
  gir1.2-atspi-2.0 \
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

В Windows:

- настройки: `%APPDATA%\KeySwitch\config.json`;
- история, обучение, пользовательские словари и журнал:
  `%LOCALAPPDATA%\KeySwitch`;
- автозапуск: пользовательский ключ реестра
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

В Linux:

- настройки: `~/.config/keyswitch/config.json`;
- история: `~/.local/share/keyswitch/history.jsonl`;
- явно выученные правила и отменённые исправления:
  `~/.local/share/keyswitch/learning.json`;
- необязательные пользовательские словари Hunspell:
  `~/.local/share/keyswitch/dictionaries/<locale>.aff/.dic`;
- журнал ошибок/запуска: `~/.local/share/keyswitch/keyswitch.log`;
- автозапуск: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

Менеджеры паролей `KeePassXC`, `1Password` и `Bitwarden` добавлены в исключения
по умолчанию. Глобальный наблюдатель не знает семантику конкретного поля, поэтому
другие чувствительные приложения следует добавить по имени `.exe` в Windows
или `WM_CLASS` в Linux на странице «Исключения».

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

В Windows отдельный E2E запускает настоящий `WH_KEYBOARD_LL`, вводит scan-коды
через `SendInput` в поле Tk и проверяет обе замены, подсказку обучения,
подтверждение правила клавишей `Enter`, повторное автоматическое применение
правила, итоговую раскладку и историю:

```powershell
$env:PYTHONPATH = "src"
py tests/e2e_windows.py
```

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

Успешный тест печатает `E2E_OK` после фактических исправлений, обучения через
`Pause/Break` + `Enter` и проверки защиты слова после ручной смены раскладки
внутри GTK Entry. Отдельный
интеграционный тест поднимает настоящий StatusNotifierItem и
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
package="dist/keyswitch_0.4.0_$(dpkg --print-architecture).deb"
./tools/verify-native-deb.sh "$package"
```

Нативные Windows-артефакты собираются на Windows с Python, Nuitka и Inno Setup:

```powershell
./packaging/build-windows.ps1 `
  -ModelDirectory build/windows-models/models `
  -ModelLicense build/windows-models/COPYRIGHT.onboard-data
```

Команда создаёт standalone `KeySwitch.exe` без исходных `.py`, переносимый ZIP
и per-user Setup EXE. Файлы моделей `en_US.lm` и `ru_RU.lm` берутся из пакета
Onboard вместе с его лицензией.

После остановки уже запущенного экземпляра полный E2E именно бинарника из
пакета можно выполнить в активной X11-сессии:

```bash
dbus-run-session -- ./tools/run-native-e2e.sh "$package"
```

Он проверяет шесть реальных коррекций, одно слово после ручной смены без
исправления, смену XKB-группы, историю, регистрацию StatusNotifierItem и состав
всплывающего DBusMenu. При отправке тега `v*` GitHub Actions независимо
проверяет Linux и Windows, собирает DEB, Windows Setup EXE и переносимый ZIP,
выполняет тихую установку и smoke-тест установленного приложения, формирует
единый `SHA256SUMS` и публикует артефакты в GitHub Release.

## Ограничения

- Linux-backend предназначен для X11. В нативной Wayland-сессии приложение
  покажет явную ошибку диагностики и не будет имитировать поддержку.
- Автоматические языковые модели по умолчанию рассчитаны на пару EN/RU.
- Приложения с собственным нестандартным вводом или удалённые рабочие столы
  могут по-разному обрабатывать синтетические события; их удобно добавить в
  исключения.
- В Windows механизм UIPI не позволяет обычному процессу вводить текст в окно,
  запущенное с более высоким уровнем целостности. Для такого окна KeySwitch
  также должен быть запущен с сопоставимыми правами.
- Windows Setup EXE версии 0.4.0 пока не подписан сертификатом издателя.

## Лицензия

KeySwitch распространяется на условиях
[GNU General Public License 3.0 или более поздней версии](LICENSE).

## Первичные спецификации

- [Microsoft LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
  — низкоуровневый клавиатурный hook и обязательный цикл сообщений;
- [Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
  — синтетический ввод и ограничение UIPI;
- [Microsoft ToUnicodeEx](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-tounicodeex)
  — преобразование виртуальной клавиши с выбранной раскладкой;
- [Microsoft Run and RunOnce keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)
  — пользовательский автозапуск после входа в Windows;
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

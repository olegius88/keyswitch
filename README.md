# KeySwitch

[![GitHub release](https://img.shields.io/github/v/release/olegius88/keyswitch)](https://github.com/olegius88/keyswitch/releases/latest)
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
- переключение XKB-группы и исправление уже напечатанного слова;
- сохранение регистра: `Ghbdtn` превращается в `Привет`;
- ручное преобразование последнего слова (`Pause`);
- отмена последнего исправления за 10 секунд (`Ctrl+Alt+Z`);
- глобальная пауза (`Ctrl+Alt+P`);
- исключения по `WM_CLASS` приложения и по словам;
- локальная история только выполненных исправлений;
- уведомления, звук, светлая/тёмная тема, XDG Autostart;
- системный индикатор StatusNotifier в панели XFCE/KDE-совместимого окружения;
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

Скачайте `keyswitch_0.1.0_all.deb` со страницы
[последнего выпуска](https://github.com/olegius88/keyswitch/releases/latest), затем:

```bash
sudo apt install ./keyswitch_0.1.0_all.deb
```

Пакет установит системные зависимости и добавит KeySwitch в меню приложений.

## Установка из исходников для текущего пользователя

```bash
./install.sh
keyswitch
```

Установщик не требует root: он размещает приложение, launcher, desktop-файл и
иконки внутри `~/.local`. Автозапуск не включается сам — его можно включить в
разделе «Внешний вид и система».

Удаление программы (настройки и история сохраняются):

```bash
./uninstall.sh
```

## Системные зависимости

На стандартной установке текущей Ubuntu необходимые компоненты уже могут быть
установлены. Если проверка установщика сообщит об их отсутствии:

```bash
sudo apt install python3-gi python3-dbus gir1.2-gtk-4.0 gir1.2-adw-1 \
  libx11-6 libxtst6 libxkbcommon0 onboard-data
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
- журнал ошибок/запуска: `~/.local/share/keyswitch/keyswitch.log`;
- автозапуск: `~/.config/autostart/io.github.olegius88.KeySwitch.desktop`.

Менеджеры паролей `KeePassXC`, `1Password` и `Bitwarden` добавлены в исключения
по умолчанию. X11 не сообщает глобальному наблюдателю семантику конкретного
поля, поэтому для других чувствительных приложений следует добавить их
`WM_CLASS` на странице «Исключения».

## Разработка и проверка

Unit-тесты:

```bash
PYTHONPATH=src python3 -m unittest discover -s tests -v
```

Настоящий сквозной тест в активной X11-сессии:

```bash
PYTHONPATH=src python3 tests/e2e_x11.py
```

Успешный тест печатает `E2E_OK` после двух фактических исправлений внутри GTK
Entry. Архитектура и критерии описаны в [DESIGN.md](DESIGN.md).

Сборка DEB-пакета:

```bash
./packaging/build-deb.sh
dpkg-deb --info dist/keyswitch_0.1.0_all.deb
```

При отправке тега `v*` workflow GitHub Actions повторяет unit-тесты, собирает
пакет, формирует `SHA256SUMS` и публикует оба файла в GitHub Release.

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
- [официальные release notes Ubuntu 26.04 LTS](https://documentation.ubuntu.com/release-notes/26.04/).

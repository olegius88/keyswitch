# KeySwitch 0.6.1

## Русский

Этот patch-релиз исправляет два конфликтующих сценария короткого ввода и
добавляет безопасный технический журнал для анализа решений распознавания.

- `ша`, набранное в русской раскладке как физический эквивалент английского
  `if`, теперь исправляется независимо от настройки минимальной длины слова —
  после пробела, другого включённого разделителя или паузы.
- Исключение намеренно узкое: оно требует точного частотного совпадения и не
  включает неоднозначные пары вроде `шт` → `in` и `фе` → `at`.
- Явная ручная смена раскладки имеет абсолютный приоритет для следующего слова.
  Если пользователь выбрал RU и ввёл `ша`, KeySwitch сохраняет русский текст
  как после паузы, так и после завершения слова; это правило сильнее ранее
  выученных автоматических преобразований.
- В Linux и Windows появился выключенный по умолчанию подробный технический
  журнал. Он фиксирует версию приложения и модели, настройки распознавания,
  оценки обеих раскладок, причины решений, ручной выбор языка и результат
  исправления.
- Журнал может содержать введённые слова и названия приложений. Текст из
  приложений-исключений всегда заменяется на `<redacted>`. Файл хранится только
  локально, ограничен 5 МиБ и имеет три ротационные копии.
- Сертифицированный KSLM-артефакт `intent-v1-6bf96537c28f` не изменён; проверка
  его замороженного toolchain и упаковочного контракта остаётся fail-closed.

Локальный контур выпуска: строгий `mypy`, 342 автоматических теста и 100%
покрытия строк и ветвей.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.6.1-x64.exe` или переносимый
  `KeySwitch-0.6.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.6.1_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This patch release fixes two conflicting short-input scenarios and adds a safe
technical log for analysing detection decisions.

- Russian-layout `ша`, the physical equivalent of English `if`, is now
  corrected independently of the configured minimum word length after Space,
  any other enabled boundary, or a typing pause.
- The exception is deliberately narrow: it requires an exact high-frequency
  match and does not admit ambiguous pairs such as `шт` → `in` or `фе` → `at`.
- An explicit manual layout change has absolute priority for the next word. If
  the user selects RU and types `ша`, KeySwitch preserves the Russian text both
  after a pause and at the word boundary, even when an older learned automatic
  rule exists.
- Linux and Windows settings now include detailed technical logging, disabled
  by default. It records application and model versions, detection settings,
  both layout scores, decision reasons, manual language selection and
  correction outcomes.
- The log may contain typed words and application names. Text from excluded
  applications is always replaced with `<redacted>`. Files stay local, are
  capped at 5 MiB and rotate through three backups.
- The certified `intent-v1-6bf96537c28f` KSLM artifact is unchanged; its frozen
  toolchain and packaging contract remain fail-closed.

The local release gate passes strict `mypy`, 342 automated tests and mandatory
100% line and branch coverage.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.6.1-x64.exe` or the portable
  `KeySwitch-0.6.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.6.1_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

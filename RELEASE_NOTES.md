# KeySwitch 0.16.1

## Русский

Этот выпуск актуализирует документацию и пояснения в интерфейсе. Алгоритмы
замены, веса моделей и настройки по умолчанию **не изменены**.

- Синхронизированы русская и английская инструкции: настройка EN/RU,
  защита первого слова после ручной смены языка, Pause/Enter, раннее
  переключение, обновления и сброс настроек.
- Исправлены пояснения порога уверенности, минимальной длины и линейной
  модели. Порог резервных эвристик не описывается как общий порог всех моделей;
  контекстный помощник настраивается отдельно.
- Уточнены фактические пределы журнала, наличие личного текста, ограничения
  распознавания парольных полей и смысл `text_verified=false`. Это уточнение
  существующего поведения, не новая проверка конечного текста.
- Добавлены карта документации и практическая диагностика пустого журнала,
  несработавшего Pause, отправки по Enter и лишних/пропавших символов.
- Обновлены инструкции обучения, сборки и восстановления после сбоя выпуска.
  Разделены проверки базовой KSLM, контекстной политики и нативного ввода;
  контрольные суммы больше не описываются как подпись издателя.

[Карта документации](https://github.com/olegius88/keyswitch/blob/v0.16.1/docs/README.md) ·
[Диагностика ввода](https://github.com/olegius88/keyswitch/blob/v0.16.1/docs/troubleshooting.md).
Исторические метрики сохранены с указанием их области применимости.
Отклонённый эксперимент context-v2 из 0.16.0 по-прежнему не активирован.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.16.1-x64.exe` или
  `KeySwitch-0.16.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.1_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. Поведение контекстного помощника из 0.15.0 сохранено.

## English

This maintenance release updates documentation and settings explanations.
Correction algorithms, model weights and default settings are **unchanged**.

- Synchronize Russian/English guides for layout order, manual-language
  protection, Pause/Enter, prefix correction, updates and settings resets.
- Clarify confidence/minimum-length controls and distinguish the baseline
  linear model from the separately configured contextual assistant.
- Correct log-retention documentation and explain private text, password-field
  limitations and `text_verified=false`. This documents existing behavior;
  it does not introduce final-text verification.
- Add documentation navigation and practical troubleshooting for empty logs,
  missed Pause conversions, Enter submission and missing/extra characters.
- Update training/build/release-recovery instructions, distinguish baseline,
  contextual and native-input evidence, and stop describing checksums as
  publisher digital signatures.

See the [documentation map](https://github.com/olegius88/keyswitch/blob/v0.16.1/docs/README.md).
Historical metrics retain their original scope. The rejected context-v2
experiment from 0.16.0 remains inactive.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.16.1-x64.exe` or
  `KeySwitch-0.16.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.1_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

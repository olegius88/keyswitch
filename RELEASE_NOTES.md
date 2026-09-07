# KeySwitch 0.17.2

## Русский

Более информативный технический журнал без полного перечня стандартных настроек.

- При запуске и включении диагностики записываются только отличия настроек от
  значений по умолчанию этой версии. Изменение и сброс параметра отмечаются
  отдельно; повторное включение журнала создаёт свежий снимок. Содержимое
  списков исключений не раскрывается — только тип и количество элементов.
- Ожидание продолжения короткого слова получает идентификатор, связывающий
  его с отменой, разрешением ожидания и ручной командой Pause. Наблюдаемые
  удаления, навигация, клики и сочетания клавиш помогают восстановить ход ввода.
  Новые события не записывают печатные клавиши или соседний текст поля.
- Диагностика чтения поля различает сбой инициализации и чтения, а также
  безопасные категории ошибок. Текст исключений не записывается.
- Названия запусков GitHub Actions явно различают тесты KeySwitch, выпуск
  KeySwitch и сборки LogCourier, независимо от названия коммита.

Это выпуск диагностики, а не новая языковая модель. Веса, значения настроек
по умолчанию и поведение автозамены не меняются. Одиночная буква без контекста
по-прежнему может ожидать продолжения. Сбойный reader не начинает автоматически
повторять запросы. `text_verified=false` не подтверждает конечный текст в поле.
Старые журналы не преобразуются и не получают отсутствовавшие события.

Технический журнал включается явно и по-прежнему может содержать анализируемые
слова в существующих событиях. Просматривайте его перед передачей. Приватные
логи и инструкции в выпуск не включены. LogCourier этим выпуском не обновляется.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.17.2-x64.exe` или
  `KeySwitch-0.17.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.17.2_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. [Описание диагностики](https://github.com/olegius88/keyswitch/blob/v0.17.2/docs/troubleshooting.md).

## English

More useful opt-in diagnostics without dumping every default setting.

- Versioned settings snapshots contain only known overrides. Changes and resets
  are explicit; re-enabling diagnostics emits a fresh snapshot. Collections,
  including exclusions, expose only their type and item count, not contents.
- Correlated short-word wait events explain cancellation, resolution and manual
  conversion. Observed editing controls help reconstruct input sequences without
  adding printable keystrokes or adjacent field text to the new events.
- Accessibility diagnostics distinguish initialization/read failures and safe
  error categories without exposing exception contents.
- GitHub Actions titles distinguish KeySwitch tests, KeySwitch releases and
  LogCourier builds independently of the triggering commit title.

Model weights, default settings and correction behavior are unchanged. An
ambiguous single letter may still wait for context. Reader retry behavior is
unchanged. `text_verified=false` does not prove the final application text.
Existing logs are not rewritten. Technical logs can still contain evaluated
words in existing events: review them before sharing. Private logs and local
instructions are not published. This release does not update LogCourier.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.17.2-x64.exe` or
  `KeySwitch-0.17.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.17.2_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

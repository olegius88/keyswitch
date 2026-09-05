# KeySwitch 0.15.0

## Русский

Добавлен локальный контекстный помощник: модель выбирает между сохранением
слова, исправлением, ожиданием продолжения и предложением варианта. Это
экспериментальный классификатор, а не большая языковая модель. Его текущая
оценка основана на небольшом авторском корпусе и синтетических сценариях;
она не доказывает отсутствие ошибок в реальной переписке. Сертифицированная
базовая модель `intent-v1-6ece07f881ec` не изменена.

- В настройках доступны `assist` (по умолчанию), `shadow` и `off`. Для
  консервативного знакомства используйте `shadow`: новый помощник наблюдает,
  но автоматические замены определяет прежний распознаватель.
- Модель учитывает недавний ввод, приложение, признаки поля и альтернативы
  EN/RU. Контекст хранится только в памяти, ограничен 512 символами и
  сбрасывается при смене окна, навигации и потере событий.
- Короткое неоднозначное слово может дождаться следующего: `e 'njuj `
  исправляется в `у этого ` с сохранением пробела и совместной отменой.
  В режиме `assist` отключена ранняя замена незаконченного префикса.
- Опция чтения текста рядом с курсором через Windows UI Automation или
  Linux AT-SPI по умолчанию выключена. Буфер обмена не используется;
  распознанные парольные поля и выделения исключаются. Перед заменой
  повторно проверяются поле и точное соответствие текста.
- Исправлен аварийный путь AT-SPI при недоступной службе. Чтение поля
  отключается до перезапуска процесса вместо аварийного завершения программы.
- Исправлена обработка общего пробела в русской группе XKB. Диагностика
  сообщает решения модели, совместные замены коротких слов и отмены
  непредставимых замен. Соседние фразы не добавляются в журнал.
- Сохранены приоритет ручных правил, проверки целостности ввода и Windows
  порядок «исправить слово, затем передать Enter один раз». В Linux/X11
  Enter/Tab остаются недоступными триггерами: XRecord не задерживает клавиши.

Подробности и границы качества:
[контекстный помощник](https://github.com/olegius88/keyswitch/blob/v0.15.0/docs/context-assistant.md).
Автоматического обучения весов на переписке нет. Существующие технические
журналы могут содержать отдельные слова; распознавание парольных полей зависит
от приложения и не является универсальной гарантией приватности.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.15.0-x64.exe` или
  `KeySwitch-0.15.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.15.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
не поддерживается.

## English

This release adds an experimental local contextual action classifier:
keep, convert, wait or suggest. It is not an LLM. Current quality evidence
comes from a small author-created corpus and synthetic scenarios, not a
real-world error-rate certification. The certified baseline intent model
`intent-v1-6ece07f881ec` is unchanged.

- Choose `assist` (default), `shadow` or `off`. Use `shadow` for a conservative
  introduction: the new model observes while the previous detector controls
  automatic corrections.
- Bounded RAM-only context includes recent input, app identity, field evidence
  and EN/RU alternatives. Unsafe editing, focus changes and lost events clear
  the observed phrase.
- A short ambiguous word may wait for its immediate successor. Joint
  correction preserves the space and supports joint undo. Contextual assist
  no longer replaces unfinished prefixes.
- Optional UI Automation/AT-SPI caret text access is off by default. It never
  uses the clipboard, rejects detected passwords/selections and revalidates
  the field and exact text before correction. Failed AT-SPI initialization
  disables native field reads instead of entering the library's fatal path.
- XKB common-key fallback now preserves the Russian-layout space. Diagnostic
  events explain contextual decisions and aborted replacements without
  adding neighboring phrases to the log.
- User rules and input-integrity guards retain priority. Windows keeps
  correct-before-Enter ordering; Linux/X11 cannot defer Enter or Tab.

No model weights are automatically trained on private conversations.
Existing technical logs can contain individual typed words. Protected-field
recognition depends on the application's accessibility provider.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.15.0-x64.exe` or
  `KeySwitch-0.15.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.15.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland sessions
are unsupported.

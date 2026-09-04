# KeySwitch 0.14.1

## Русский

Включает все изменения 0.14.0, установщики которого не были опубликованы.
Исправлен Windows E2E: ожидание обработки очистки поля, UTF-8 в диагностике,
обязательное закрытие окна при ошибке и таймаут с выводом стеков.

Выпуск повышает надёжность ввода и замены EN/RU по результатам пользовательских
обращений. Сертифицированная модель намерения `intent-v1-6ece07f881ec`
не изменена.

- В Windows `ghbdtn` + Enter сначала исправляется в `привет`, затем Enter
  передаётся чату один раз. Аналогично обрабатываются Numpad Enter и Tab.
  Быстрый ввод следующего сообщения остаётся после предыдущего; удержание
  Enter не вызывает повторной отправки.
- Enter в подсказке обучения только подтверждает правило. Shift/Ctrl/Alt
  сочетания остаются у приложения. При смене поля, сбое замены или отсутствии
  отпускания клавиш отложенное действие отменяется с диагностикой.
- Замены учитывают отпускание клавиш, очередь ввода, клики, положение фокуса
  и Caps Lock. Undo не переписывает устаревшее слово. Удержанный ввод
  воспроизводится с отдельной меткой и без повторного захвата.
- После Pause и исправления по таймеру можно продолжить слово, стереть букву
  и снова нажать Pause: сохраняется весь токен, а не только новый суффикс.
- Улучшены границы слов: внутренние дефисы и апострофы, цифры в `pm2`,
  маркеры идентификаторов, пунктуация и Unicode-пробелы. Сложный Unicode
  и слишком длинные токены не запускают опасную замену суффикса.
- Явные выученные правила коротких слов больше не скрываются минимальной длиной.
- Журнал различает нажатия и отпускания, связывает попытки замены, описывает
  перехват/доставку/отмену Enter и явно указывает `text_verified=false`:
  отправка событий не означает, что итоговый текст был прочитан обратно.

В Linux/X11 Enter/Tab остаются недоступными триггерами: пассивный XRecord не
может задержать уже доставленную клавишу. Используйте пробел, паузу, пунктуацию
или Pause до отправки. Матрица сценариев и оставшиеся ограничения доступны в
[docs/input-maturity.md](https://github.com/olegius88/keyswitch/blob/v0.14.1/docs/input-maturity.md).

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.14.1-x64.exe` или переносимый
  `KeySwitch-0.14.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.14.1_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
не поддерживается. Технические журналы могут содержать личный текст.

## English

Includes all 0.14.0 changes; that version's installers were not published.
The Windows E2E now waits for keyboard-driven field clearing, emits UTF-8
diagnostics, always closes its window on failure and has a bounded watchdog.

This release hardens EN/RU input handling based on user-reported failures.
The certified intent model `intent-v1-6ece07f881ec` is unchanged.

- On Windows, `ghbdtn` + Enter is corrected to `привет` before Enter reaches
  the chat exactly once. Numpad Enter and Tab use the same ordering. Rapid
  subsequent messages remain separate, including queued Enter presses.
- Learning confirmation owns Enter exclusively. Shift/Ctrl/Alt shortcuts
  remain application actions. Focus changes, correction errors and missing
  key-up cancel a deferred action with diagnostics.
- Corrections respect key releases, queued input, pointer activity, focus and
  Caps Lock. Stale Undo is refused; held input uses a distinct replay marker.
- Continued typing, Backspace and Pause after a boundary-free correction
  operate on the whole token.
- Internal hyphens/apostrophes, digits, identifier markers, punctuation and
  Unicode whitespace are handled more conservatively. Complex Unicode and
  oversized tokens cannot start an unsafe suffix replacement.
- Explicit learned short-word rules bypass the minimum-length threshold.
- Diagnostics distinguish key presses from releases and report action
  deferral, delivery and cancellation. `text_verified=false` explicitly means
  application text was not read back.

On Linux/X11, Enter/Tab correction remains unavailable because XRecord is
passive. Use Space, idle correction, punctuation or Pause before submitting.
Editor-specific autocomplete, protected fields, IMEs and remote desktops still
need application-specific validation.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.14.1-x64.exe` or
  `KeySwitch-0.14.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.14.1_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland sessions
are unsupported. Technical logs can contain private text.

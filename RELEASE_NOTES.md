# KeySwitch 0.12.0

## Русский

Выпуск про подсказку обучения: клавиша, которой вы отвечаете подсказке, больше
не уходит заодно в текст. Сертифицированная модель намерения
`intent-v1-6ece07f881ec` не изменилась.

- Enter подтверждал правило — и одновременно доходил до редактора под
  подсказкой, а в чате это отправляло недописанное сообщение; у Esc была та же
  двойная жизнь. Теперь, пока подсказка на экране, Windows-хук не пропускает
  Enter, Enter на цифровом блоке и Esc без модификаторов — вместе с парным
  отпусканием, чтобы ни одно окно не получило key-up без key-down. Подсказка
  по-прежнему не забирает фокус, поэтому набор вокруг неё не меняется, а как
  только на неё ответили или истекли её восемь секунд, клавиши снова
  принадлежат приложению. В Linux они проходят как раньше: XRecord только
  наблюдает и удержать клавишу не может.
- Сценарий Windows E2E перед набором убеждается, что вводимые события реально
  доходят до поля, и несколько раз повторяет захват окна вместо того, чтобы
  через десять секунд упасть с пустым полем.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.12.0-x64.exe` или переносимый
  `KeySwitch-0.12.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.12.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release about the learning prompt: the key that answers it no longer lands in
the text as well. The certified intent model `intent-v1-6ece07f881ec` is
unchanged.

- Enter confirmed the rule *and* arrived in the editor underneath the prompt,
  which in a chat sends the half-written message; Esc had the same double life.
  While the prompt is on screen the Windows hook now withholds an unmodified
  Enter, keypad Enter or Esc — together with the matching key release, so no
  window sees a key-up without its key-down. The prompt still does not take the
  focus, so typing around it is unchanged, and once it is answered or its eight
  seconds run out the keys belong to the application again. Linux keeps passing
  them through: XRecord only observes and cannot withhold a key.
- The Windows end-to-end scenario proves that injected input really reaches its
  field before it starts typing into it, and retries the activation a few times
  instead of failing ten seconds later with an empty field.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.12.0-x64.exe` or the portable
  `KeySwitch-0.12.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.12.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

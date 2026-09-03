# KeySwitch 0.9.1

## Русский

Исправительный выпуск по итогам разбора технического журнала реальной работы в
Windows. Основная причина пропущенных исправлений найдена: Windows хранит
раскладку отдельно для каждого окна, поэтому переход между приложениями
выглядел для движка как ручное переключение раскладки и защищал следующее слово
от автокоррекции. Сертифицированная модель намерения `intent-v1-6ece07f881ec`
не изменилась.

- Раскладка, пришедшая вместе с другим окном (Telegram → TeamViewer, редактор →
  браузер), больше не считается ручным переключением и не защищает следующее
  слово. Движок отслеживает активное окно и записывает такую смену как
  раскладку этого окна. Раскладка, выбранная самим движком, снимает прежний
  ручной выбор, поэтому защита больше не оживает через несколько минут при
  возврате в ту же группу.
- Окна самого KeySwitch (настройки, подсказка обучения) больше не влияют на
  раскладку набора. В Windows подсказка обучения показывается неактивируемым
  окном: она не забирает курсор ввода у редактора и не съедает следующую
  нажатую клавишу. `Enter` и `Esc` по-прежнему отвечают на неё через
  глобальный перехват, но, поскольку фокус остаётся у редактора, они теперь
  доходят и до самого редактора.
- Переход в другое окно сбрасывает незавершённое слово и делает последнее
  завершённое слово недоступным для преобразования, поэтому пауза при наборе
  или `Pause` больше не переписывают текст в чужом окне.
- Правило обучения предлагается только для того, что читается как слово, то
  есть содержит хотя бы две буквы. Одиночная буква, преобразованная в знак
  препинания (`б` → `,`), больше не попадает ни в подсказку, ни в счётчик
  подтверждений будущего правила.
- Отмена (`Ctrl+Alt+Z` по умолчанию) во время набора слова, у которого раскладка
  была переключена по началу слова, возвращает именно это начало слова, а не
  предыдущее исправление, и оставляет остаток слова в покое. Раннее
  переключение, брошенное без границы слова (стрелка, смена окна), всё равно
  попадает в историю и доступно для отмены.
- Раннее переключение принимает начало слова, набранное на клавишах пунктуации,
  которые в другой раскладке являются буквами (`nt,z` → `тебя`).
- Технический журнал: у наблюдений за раскладкой появился признак
  `focus_changed`; добавлены события `focus_changed`, `layout_change_ignored` и
  `early_switch_undo_scheduled`; `manual_conversion_scheduled` сообщает
  `learnable`.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.9.1-x64.exe` или переносимый
  `KeySwitch-0.9.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.9.1_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A bugfix release driven by the technical log of real Windows usage. The root
cause of missed corrections is found: Windows keeps a keyboard layout per
window, so moving between applications looked like a manual layout switch to
the engine and protected the next word from autocorrection. The certified
intent model `intent-v1-6ece07f881ec` is unchanged.

- A layout that arrives together with another window (Telegram → TeamViewer, an
  editor → the browser) is no longer treated as a manual switch and no longer
  protects the next word. The engine tracks the focused window and records such
  a change as that window's own layout. A layout the engine selects itself
  drops an earlier manual choice, so the protection no longer revives minutes
  later when the engine returns to that group.
- Windows of KeySwitch itself (settings, the learning prompt) no longer affect
  the typing layout. On Windows the learning prompt is shown as a
  non-activating popup: it does not take the caret away from the editor and
  does not swallow the next typed key. `Enter` and `Esc` still answer it
  through the global hook, and because the editor keeps the focus they now
  also reach the editor itself.
- Moving to another window drops the unfinished word and makes the last
  completed word unavailable for conversion, so a typing pause or `Pause` never
  rewrites text in the wrong window.
- A learning rule is offered only for something that reads as a word, that is
  has at least two letters. A lone letter converted to punctuation (`б` → `,`)
  no longer reaches the prompt or counts towards a future automatic rule.
- Undo (`Ctrl+Alt+Z` by default) during a word whose layout was switched from
  its prefix reverts exactly that prefix instead of the previous correction and
  leaves the rest of the word alone. An early switch abandoned without a word
  boundary (an arrow key, another window) still enters the history and can be
  undone.
- Early switching accepts a prefix typed on punctuation keys that are letters
  in the other layout (`nt,z` → `тебя`).
- Technical log: layout observations carry `focus_changed`; new events
  `focus_changed`, `layout_change_ignored` and `early_switch_undo_scheduled`;
  `manual_conversion_scheduled` reports `learnable`.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.9.1-x64.exe` or the portable
  `KeySwitch-0.9.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.9.1_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

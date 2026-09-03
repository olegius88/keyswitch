# KeySwitch 0.9.0

## Русский

Этот выпуск меняет поведение при наборе: раскладка переключается, не дожидаясь
конца слова, `Pause` преобразует только то, что действительно набрано после
границы слова, пауза перед проверкой настраивается, а технический журнал
позволяет разобрать каждый такой случай. Сертифицированная модель намерения
`intent-v1-6ece07f881ec` (кандидат v20) не изменилась.

- Ранняя смена раскладки. Как только начало слова невозможно в текущем языке и
  явно продолжается в другом (`ghbd` → `прив`), раскладка переключается, а
  начало слова переписывается. Решение принимается по отсортированному индексу
  частотного словаря и основ Hunspell: у префикса не должно быть ни одного
  продолжения в исходном языке, он не должен быть словом сам по себе, а
  альтернативное прочтение обязано начинать много частотных слов другого языка.
  На самых трудных словах (только Hunspell, без частот) это даёт около 0,01%
  ложных переключений для английского и 0,07% для русского при четырёх буквах.
  Минимальная длина префикса настраивается (3–8, по умолчанию 4), функцию можно
  выключить. Переписывание выполняется при отпускании клавиши последней буквы,
  поэтому ни одна буква не теряется при быстром наборе; буква, пришедшая в
  старой раскладке сразу после переключения, преобразуется отдельно. Слово
  попадает в историю и в отмену одной коррекцией на границе, где обычный
  детектор всё ещё может вернуть его обратно.
- `Pause` преобразует только набранное после последней границы слова:
  незавершённое слово, зависящие от раскладки символы (русская кавычка вместо
  `@`) или последнее завершённое слово, пока после него ничего не введено.
  Иначе клавиша просто переключает раскладку и защищает следующее слово.
  Раньше в такой ситуации преобразовывалось предыдущее слово целиком.
- Длительность паузы перед проверкой незавершённого слова настраивается
  (0,3–5 секунд, по умолчанию 1,5). Движок просыпается точно к её истечению, а
  не по сетке в полсекунды, а нажатия без отпускания старше трёх секунд больше
  не блокируют коррекцию.
- Смена раскладки в ту группу, которую движок сам только что выбрал
  (исправление, `Pause`, пункт меню), больше не считается ручной, поэтому
  следующее слово не защищается по ошибке; переход в любую другую раскладку
  остаётся ручным.
- Технический журнал: у `word_evaluation` появились причина пропуска, детали
  защиты, использованный контекст, время простоя и теневое решение детектора
  для защищённых или отключённых слов; `correction_applied` записывает режим,
  число удалённых символов, предыдущую раскладку и время инъекции; добавлены
  события отложенной коррекции по паузе, ранней смены раскладки, запоздавших
  букв, переключения без слова и жизненного цикла подсказки обучения.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.9.0-x64.exe` или переносимый
  `KeySwitch-0.9.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.9.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This release changes how typing feels: the layout switches without waiting for
the end of the word, `Pause` converts only what was really typed after the word
boundary, the idle delay is configurable, and the technical log explains every
such decision. The certified intent model `intent-v1-6ece07f881ec` (candidate
v20) is unchanged.

- Early layout switching. As soon as the beginning of a word is impossible in
  the current language and clearly continues in the other one (`ghbd` →
  `прив`), the layout is switched and the prefix rewritten. The decision uses a
  sorted index of the frequency lexicon and Hunspell stems: the prefix must
  have no continuation at all in the source language, must not be a word
  itself, and the alternative reading must start many frequent words of the
  other language. On the hardest words (Hunspell-only, without frequencies)
  that yields roughly 0.01% false switches for English and 0.07% for Russian at
  four letters. The minimum prefix length is configurable (3–8, 4 by default)
  and the feature can be disabled. The rewrite happens when the last letter's
  key is released, so no letter is lost during fast typing; a letter that
  arrives in the old layout right after the switch is converted on its own. The
  finished word enters the history and the undo buffer as a single correction
  at the boundary, where the ordinary detector can still turn it back.
- `Pause` converts only what was typed after the last word boundary: the
  unfinished word, layout-dependent symbols (the Russian quote meant as `@`) or
  the last completed word while nothing else was typed after it. Otherwise the
  key only switches the layout and protects the next word. Previously the whole
  previous word was converted in that situation.
- The typing pause before an unfinished word is checked is configurable
  (0.3–5 seconds, 1.5 by default). The engine wakes exactly when the delay
  elapses instead of on a half-second grid, and key presses without a release
  older than three seconds no longer block the correction.
- A change to the layout the engine itself just selected (a correction,
  `Pause`, the menu action) is no longer treated as a manual switch, so the
  next word is not protected by mistake; a change to any other layout stays
  manual.
- Technical log: `word_evaluation` now carries the skip reason, protection
  details, the context that was used, the idle time and a shadow detector
  verdict for protected or disabled words; `correction_applied` records the
  mode, deleted characters, previous layout and injection time; new events
  cover deferred pause corrections, early switches, late strokes, layout
  switches without a word and the learning prompt lifecycle.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.9.0-x64.exe` or the portable
  `KeySwitch-0.9.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.9.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

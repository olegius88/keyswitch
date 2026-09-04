# KeySwitch 0.11.0

## Русский

Выпуск про журнал и про окно настроек: технический журнал наконец пишется в
файл и подписан версией, окно настроек прокручивается, а изменённые параметры
видны и возвращаются к значению по умолчанию одной кнопкой. Сертифицированная
модель намерения `intent-v1-6ece07f881ec` не изменилась.

- Технический журнал мог оставаться пустым весь сеанс. Обработчик файла
  устанавливался через `logging.basicConfig`, а тот молча ничего не делает,
  если журналирование уже кем-то настроено: файл создавался и никогда не
  заполнялся. Теперь обработчик подключается к корневому логгеру напрямую,
  каждый сеанс начинается строкой с версией, платформой и бюджетом ротации, а
  на странице «Обслуживание» видно, ведётся ли журнал и какого он размера.
- Каждая строка журнала содержит версию, которая её записала. Файл, переживший
  обновление, больше нельзя прочитать как журнал текущей версии.
- Цифра или символ внутри слова больше не стирает слово. `зь2` оставляло
  пустой буфер, поэтому `Pause` нечего было преобразовывать и он лишь
  переключал раскладку; теперь получается `pm2`. Автокоррекция не изменилась:
  токен с цифрой детектор по-прежнему считает кодом и сам не трогает.
- Когда движок всё же выбрасывает незавершённое слово — сочетание с Ctrl,
  перемещение курсора, смена раскладки посреди слова, переход в другое окно, —
  в технический журнал попадает `word_discarded` с причиной. Несостоявшееся
  исправление теперь объясняется журналом, а не догадками.
- Каждая страница настроек прокручивается. Страница выше окна получает свою
  полосу прокрутки и слушает колесо мыши и PageUp/PageDown, а не прячет
  настройки за нижним краем; колесо над числовым полем или списком прокручивает
  страницу, а не меняет значение. Описания переносятся по реальной ширине
  колонки, поэтому страницы выдерживают и узкое окно.
- Параметр, отличающийся от значения по умолчанию, отмечен как изменённая
  строка в редакторе: акцентная полоса слева, акцентный заголовок и кнопка
  «Сброс», возвращающая именно это значение. Место под отметки зарезервировано,
  поэтому при изменении ничего не съезжает. Строки настроек теперь на фоне
  страницы, а не на серой полосе системной темы.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.11.0-x64.exe` или переносимый
  `KeySwitch-0.11.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release about the log and about the settings window: the technical log is
finally written to its file and stamped with the version, every settings page
scrolls, and a changed setting is visible and revertible with one button. The
certified intent model `intent-v1-6ece07f881ec` is unchanged.

- The technical log could stay empty for a whole session. The file handler was
  installed through `logging.basicConfig`, which does nothing once anything
  else has configured logging first — silently, so the file was created and
  never written. The handler is now attached to the root logger directly, every
  session opens with a line naming the version, the platform and the rotation
  budget, and the maintenance page states whether the journal is really being
  written and how large it is.
- Every log line carries the version that wrote it, so a file that outlives an
  update can no longer be read as the current version's log.
- A digit or a symbol typed inside a word no longer throws the word away. `зь2`
  used to leave an empty buffer, so `Pause` had nothing to convert and only
  switched the layout; it now becomes `pm2`. Automatic correction is unchanged:
  the detector still treats a token carrying a digit as code.
- Whenever the engine does drop an unfinished word — a shortcut, a caret move,
  a layout change mid-word, a focus change — the log records `word_discarded`
  with the reason, so a correction that never happened can be explained from
  the log instead of guessed at.
- Every settings page scrolls. A page taller than the window carries its own
  scrollbar and answers the wheel and PageUp/PageDown instead of hiding the
  settings below the fold; the wheel over a number field or a drop-down scrolls
  the page rather than changing the value. Descriptions wrap to the width the
  column really has, so the pages survive a narrow window too.
- A setting that differs from its shipped default is marked the way an editor
  marks an edited line: an accent bar beside it, an accented title and a
  "Сброс" button that restores that one value. The gutters holding them are
  reserved, so nothing moves as the markers appear. Setting rows now share the
  page background instead of the grey band the platform theme drew behind them.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.11.0-x64.exe` or the portable
  `KeySwitch-0.11.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.11.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

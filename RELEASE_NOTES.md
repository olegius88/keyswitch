# KeySwitch 0.23.1

## Русский

Выпуск убирает две причины, по которым портился правильно набранный текст, и чинит
автозагрузку в Windows. Номер 0.23.1: тег 0.23.0 существует, но его сборка остановилась
на проверке покрытия в Windows, и файлы выпуска не были опубликованы. Модели те же, что в
[0.22.0](https://github.com/olegius88/keyswitch/releases/tag/v0.22.0); изменилось то,
какой словарь читает движок и что он делает до конца слова. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.23.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.23.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.23.1-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Разговорные русские слова больше не переводятся

Движок читает поставляемое дополнение словаря: 199 680 русских форм из частотного
списка OpenSubtitles 2018, которых нет во встроенном словаре. Файл входил в пакет и
раньше, но его читали только инструменты обучения. Пять из шести раскрытых дефектов
0.22.0 исчезли на тех же весах: `гифку`, `флуд`, чат-последовательности и комментарий
в редакторе кода сохраняют написание.

Остался один раскрытый случай: изолированное слово, неизвестное ни встроенному
словарю, ни дополнению (`дюп`), по-прежнему переводится. Он зафиксирован в наборе
тестов как ожидаемое падение и закрывается новой контекстной моделью: она прошла независимую
запечатанную оценку 16.09.2026, но в этот выпуск не входит.

### Раннее переключение выключено по умолчанию

Раньше раскладка менялась уже по первым четырём буквам. На проверочной выборке это
портило 13 правильно набранных слов из 256 и восстанавливало 116 неверных из 229; без
него — 4 порчи и 109 восстановлений. Порча уже набранного текста хуже пропущенного
исправления, а решение по завершённому слову возвращает почти всё, что давало раннее
переключение. Настройка осталась на месте: «Ранняя смена раскладки» в разделе
поведения.

### Автозагрузка в Windows

Windows хранит отдельную отметку разрешения для каждой записи автозапуска
(«Диспетчер задач → Автозагрузка приложений»). Если KeySwitch там был выключен,
Windows игнорировала запись при каждом входе, а KeySwitch показывал автозагрузку
включённой и молча перезаписывал её при каждом старте. Теперь состояние читается
честно: переключатель показывает, запустится ли приложение при следующем входе,
`KeySwitch.exe --diagnose` печатает команду, отметку Windows и признак пропавшего
файла, а окно настроек объясняет, что делать. Переключение автозагрузки в самом
KeySwitch снимает блокировку Windows; автоматическая синхронизация при каждом запуске
больше не отменяет выбор, сделанный в Windows.

### Известные ограничения

- Изолированные слова, неизвестные словарю, всё ещё могут быть переведены (случай
  `дюп` выше). Обход: добавить слово в исключения (`exclusions.words`) или нажать
  клавишу отмены (`Ctrl+Alt+Z`).
- Числа выше измерены на проверочной выборке из 490 последовательностей и не являются
  независимой оценкой качества на произвольном вводе.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

This release removes two causes of correctly typed text being changed and fixes
autostart on Windows. It is numbered 0.23.1: the 0.23.0 tag exists, but its build stopped
at the Windows coverage gate and no files were published. The models are the same as in
[0.22.0](https://github.com/olegius88/keyswitch/releases/tag/v0.22.0); what changed is
the lexicon the engine reads and what it does before a word ends. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.23.1_amd64.deb` — Ubuntu/Xubuntu, X11 session.
- `KeySwitch-Setup-0.23.1-x64.exe` — installer for Windows 10/11 x64. It is not signed
  with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.23.1-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Colloquial Russian words are no longer converted

The engine now reads the packaged lexicon supplement: 199,680 Russian forms from the
OpenSubtitles 2018 frequency list that the onboard lexicon lacks. The file shipped
before, but only the training tools read it. Five of the six disclosed 0.22.0 defects
are gone with the same weights: `гифку`, `флуд`, the chat sequences and the
code-editor comment keep their spelling.

One disclosed case remains: an isolated word that neither the onboard lexicon nor the
supplement knows (`дюп`) is still converted. It stays in the test suite as an expected
failure and is fixed by the new context model, which passed its independent sealed evaluation on
16 September 2026 but is not part of this release.

### The early layout switch is off by default

The layout used to change after the first four letters. On the development population
that corrupted 13 correctly typed words out of 256 and restored 116 wrong ones out of
229; without it, 4 corruptions and 109 restorations. Changing text the user typed
correctly is the worse failure, and the completed-word decision recovers most of what
the early switch was restoring. The setting is still there, under the behaviour
section.

### Windows autostart

Windows keeps a separate approval record for every startup entry (Task Manager,
Startup apps). If KeySwitch was disabled there, Windows skipped the entry at every
logon while KeySwitch reported autostart as enabled and silently rewrote it at every
launch. The state is now reported honestly: the switch says whether the next logon
will start the application, `KeySwitch.exe --diagnose` prints the command, the Windows
block and a missing target, and the settings window explains what to do. Toggling
autostart inside KeySwitch clears a Windows block; the automatic sync at every launch
no longer overrules a choice made in Windows.

### Known limitations

- Isolated words unknown to the lexicon can still be converted (the `дюп` case above).
  Workaround: add the word to the exclusions (`exclusions.words`) or press the undo key
  (`Ctrl+Alt+Z`).
- The numbers above come from a development population of 490 sequences and are not an
  independent measurement of quality on arbitrary input.

Technical logs may contain evaluated words. Review them before sharing. Private logs
and conversations are excluded from the release.

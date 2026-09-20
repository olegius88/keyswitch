# KeySwitch 0.26.1

## Русский

Исправление двух ошибок. Одна портила текст на всех системах, вторая держала
выключенной целую возможность на Windows. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.26.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.26.1-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.26.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-0.26.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.26.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Слово больше не превращается в знаки препинания

Шесть русских букв — б, ю, ж, э, х, ъ — стоят на клавишах запятой, точки и кавычек.
Поэтому у короткого русского слова «другое прочтение» иногда оказывается не словом, а
россыпью знаков: набранное `дюп` в другой раскладке — это `l.g`. Такую замену
запрещал только один из слоёв программы, и правильно набранное слово изредка
превращалось в мусор.

Теперь запрет действует на всех слоях. Восстановление имён файлов вроде `.dist` и
сокращений вроде `don't` работает как прежде: точка в начале и один апостроф внутри
слова — это по-прежнему слово.

### На Windows снова читается текст перед курсором

Программа умеет заглядывать в поле ввода и видеть то, что там уже написано: это
помогает, когда слово дописывают к набранной фразе, и заметно улучшает выбор языка.
На Windows эта возможность не открывалась ни в одной установленной версии — только на
той машине, где выпуск собирался.

Причина оказалась в том, как устроена библиотека доступа к интерфейсу Windows: она
записывает в сгенерированный ею модуль время изменения системного файла
`UIAutomationCore.dll` и отказывается загружать этот модуль там, где файл другой, —
если только программа не объявляет себя «замороженной» сборкой. KeySwitch теперь
объявляет это ровно на время одного вызова. Возможность включена по умолчанию;
выключается она в настройках, «Автокоррекция» → «Читать контекст активного поля».

Убедиться, что всё в порядке, можно командой `KeySwitch.exe --diagnose`: в поле
`context_field_access` должно быть `"available": true`.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

Two fixes. One of them damaged text on every system, the other kept a whole capability
switched off on Windows. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.26.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.26.1-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.26.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-0.26.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.26.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### A word is no longer turned into punctuation

Six Russian letters — б, ю, ж, э, х, ъ — sit on the comma, period and quote keys, so the
other reading of a short Russian word is sometimes not a word at all but a scattering of
marks: typed `дюп` reads `l.g` in the other layout. Only one of the layers inside the
program refused such a replacement, and a correctly typed word was occasionally turned
into rubbish.

The refusal now covers every layer. Restoring a file name such as `.dist` or a
contraction such as `don't` works as before: a leading dot and one apostrophe inside a
word still make a word.

### Windows reads the text in front of the caret again

KeySwitch can look into the input field and see what is already written there. That helps
where a word is being added to an existing phrase, and it markedly improves the choice of
language. On Windows the capability had never opened in any installed version — only on
the machine where the release was built.

The cause was in the library that reaches the Windows interface: it writes the
modification time of the system file `UIAutomationCore.dll` into the module it generates
and then refuses to load that module wherever the file differs, unless the program
declares itself a frozen build. KeySwitch now declares it for the length of that one
call. The capability is on by default and is switched off under Autocorrection, "Читать
контекст активного поля", in the settings.

`KeySwitch.exe --diagnose` confirms it: the `context_field_access` field should read
`"available": true`.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

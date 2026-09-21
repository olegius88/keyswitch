# KeySwitch 0.27.0

## Русский

Три изменения о доверии к программе: правило запоминается только по вашему «да»,
кавычка в Telegram перестала превращаться в «@» где попало, а чтение поля на Windows
больше не выключается до перезапуска. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.27.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.27.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.27.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-0.27.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.27.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Правило запоминается только по Enter

Преобразование слова по Pause раньше сразу записывало это слово в правила, а подсказка
«Добавить слово в правила переключения?» лишь предлагала закончить начатое. Если на неё
не ответить — продолжить набор, щёлкнуть мышью, перейти в другое окно или дать ей
исчезнуть, — запись оставалась, и два таких исправления одного слова делали правило
действующим само по себе. В собранных журналах за тринадцать дней так появилось
семнадцать правил при одном-единственном Enter.

Теперь преобразование только спрашивает. `Enter` записывает правило сразу, а любой
другой исход равнозначен `Esc`: правила не меняются. Правила, накопленные прежними
версиями и не дошедшие до порога, остаются недействующими — их можно подтвердить или
стереть кнопкой «Очистить самообучение».

### Кавычка в Telegram: «@» только там, где это упоминание

Кавычка, набранная в русской раскладке, превращалась в «@» сразу при нажатии клавиши.
Это удобно для упоминания, но цитата внутри русской фразы тоже становилась «@», да ещё
и с переключением на английскую раскладку — дальше текст шёл латиницей. Нажатие Pause
следом повторяло ту же замену вместо отмены.

Теперь решает слово, которое идёт за кавычкой, и оценивается оно без неё: «ощрт» в
русском не значит ничего, а «john» — имя. Поэтому `"john` становится `@john`, а
`"привет` остаётся цитатой. Кавычка, набранная вплотную к слову, закрывает цитату: в
`"john"` остаются обе кавычки, исправляется только имя. Кавычка после перемещения
курсора или после того, как раскладку выбрали вручную, не трогается вовсе. Правило
по-прежнему отключается в настройках, в разделе «Особенности программ».

### Windows: чтение поля больше не выключается до перезапуска

Программа заглядывает в поле ввода, чтобы видеть уже написанное, — от этого зависит
выбор языка для коротких слов. Окна Chromium и Qt (редакторы, мессенджеры, браузеры)
показывают поле, но не отдают его текст, пока не поднимут собственное дерево
доступности. Такой ответ считался поломкой: одного поля хватало, чтобы израсходовать
все попытки восстановления и выключить чтение до перезапуска программы. В собранных
журналах из 1628 обращений успешными были 46.

Теперь поле без текста — это поле без текста, а не сбой: попытки не расходуются.
Повторы после настоящей ошибки замедляются до одного в минуту, но не прекращаются;
навсегда выключают чтение только отсутствие системной библиотеки и явный отказ в
доступе. Время ожидания ответа поднято с 50 до 200 миллисекунд — прежнего не хватало
окну, которое отвечает впервые.

В журнале теперь видно, что именно отказало: класс ошибки, код HRESULT и длительность
последнего чтения. Текст самой ошибки по-прежнему не записывается, чтобы в журнал не
попал набранный текст. `KeySwitch.exe --diagnose` дополнительно показывает, отдаёт ли
текущее поле текст.

## English

Three changes about trusting the program: a rule is remembered only when you say yes, a
quote in Telegram no longer turns into `@` wherever it appears, and the Windows field
reader no longer switches itself off until a restart. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.27.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.27.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.27.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-0.27.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.27.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### A rule is learned only from Enter

Converting a word with Pause used to write that word into the rules straight away, and the
prompt that followed merely offered to finish the job. Leaving the prompt unanswered —
typing on, clicking elsewhere, switching windows or letting it expire — kept the
half-written rule, and two such conversions of the same word made it active on their own.
Thirteen days of collected logs hold seventeen recorded rules against a single Enter.

A conversion now only asks. `Enter` records the rule at once; every other outcome leaves
the rules unchanged, exactly as `Esc` always did. Rules left half-confirmed by earlier
versions stay inactive until they are confirmed or cleared.

### Telegram: `@` only where a mention is meant

A quote typed in the Russian layout used to become `@` the moment the key came up. That
suits a mention, but a quotation inside a Russian sentence became `@` as well, and the
layout switched to English, so the rest of the line came out in Latin. Pressing Pause
afterwards repeated the same replacement instead of undoing it.

The word that follows now decides, and it is judged without the quote: `ощрт` means
nothing in Russian, `john` is a name. So `"john` becomes `@john` while `"привет` stays a
quotation. A quote typed against the end of a word closes a quotation: `"john"` keeps both
quotes and only the name is corrected. A quote typed after a caret move, or after the
layout was chosen by hand, is left alone. The convention can still be switched off in the
settings, under application conventions.

### Windows: the field reader stays alive

KeySwitch looks into the input field to see what is already written there; the choice of
language for short words depends on it. Chromium and Qt windows — editors, chat clients,
browsers — expose the field but not its text until their own accessibility tree is up.
That answer counted as a failure: one such field was enough to spend every recovery
attempt and switch reads off until the program was restarted. In collected logs 46 of 1628
attempts succeeded.

A field without text is now a field without text, not a failure, and it costs no attempts.
Retries after a real error slow down to one a minute instead of stopping; only a missing
system library and an explicit denial of access switch reads off for good. The request
timeout is raised from 50 to 200 milliseconds, which the earlier value denied to a window
answering for the first time.

The log now names what refused: the exception class, the provider's HRESULT and how long
the last read took. The exception text itself is still never written, so that typed text
cannot reach the log. `KeySwitch.exe --diagnose` additionally reports whether the focused
field offers text at all.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

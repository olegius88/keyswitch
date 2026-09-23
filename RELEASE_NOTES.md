# KeySwitch 0.30.0

## Русский

Контекстная модель переобучена: правильно набранные русские слова после английского
текста или кода больше не превращаются в латиницу — «еще» не становится «tot», «ты» не
становится «ns». Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.30.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.30.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.30.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.30.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.30.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Русские слова после английского текста остаются русскими

Контекстная модель смотрит, какой алфавит преобладает в тексте перед курсором, и
выучила, что русское слово после почти целиком английского текста — ошибка раскладки.
В чате под редактором кода или в комментарии «еще» превращалось в «tot», а «ты» — в
«ns», даже сразу после «все» или «почему».

Модель обучена заново. В сценарии добавлены разговорные русские слова, включая «еще»
в обычном написании без «ё», короткие английские слова, которых там не было вовсе, и
поля с кодом, журналами и английским текстом. В таких полях правильно набранные
русские и английские слова остаются, а набранные не в той раскладке исправляются, в
равной мере, поэтому алфавит поля больше не решает сам по себе.

Проверка на 500 фразах из субтитров фильмов, которых не видела ни одна модель (300
русских и 200 английских), каждая набрана через программу в пяти видах полей:

- правильно набранный текст новая модель испортила в 8 фразах из 2 500, прежняя — в
  23, из них 11 раз это было «еще» → «tot»;
- набранное не в той раскладке новая модель исправила в 2 181 фразе, прежняя — в 2 172.

Один случай стал хуже: «мы», набранное в английской раскладке первым словом сразу
после английского текста или кода, теперь остаётся «vs» (24 из 1 500 русских фраз).
Pause исправляет его.

## English

The context model is retrained: correctly typed Russian words after English text or code
are no longer turned into Latin — "еще" no longer becomes "tot", "ты" no longer becomes
"ns". The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.30.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.30.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.30.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.30.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.30.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Russian words after English text stay Russian

The context model looks at which alphabet dominates the text in front of the caret, and
it had learned that a Russian word after mostly English text is a layout error. In a
chat under a code editor or in a code comment, "еще" became "tot" and "ты" became "ns",
even right after "все" or "почему".

The model is trained anew. The scenarios add conversational Russian words, including
"еще" in its everyday spelling without "ё", the short English words they lacked
entirely, and fields holding code, logs and English prose. In such fields correctly typed
Russian and English words stay and either one typed in the other layout is converted,
in equal measure, so the alphabet of the field no longer decides on its own.

Checked on 500 sentences from film subtitles that no model had seen (300 Russian, 200
English), each typed through the program in five kinds of fields:

- the new model spoiled correctly typed text in 8 sentences out of 2,500, the previous
  one in 23, 11 of them "еще" → "tot";
- the new model restored text typed in the wrong layout in 2,181 sentences, the previous
  one in 2,172.

One case got worse: "мы" typed in the English layout as the first word right after
English text or code now stays "vs" (24 of the 1,500 Russian sentences). Pause converts it.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

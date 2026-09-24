# KeySwitch 0.32.0

## Русский

Буквы, набранные внутрь уже написанного слова, теперь решаются вместе с этим словом: поставили
курсор щелчком в «сд|лать» и набрали пропущенную «е» — программа видит всё слово. Полный
перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.32.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.32.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.32.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.32.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.32.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Правка внутри слова

Чтобы исправить слово, курсор ставят щелчком в его середину и набирают недостающие буквы. Раньше
программа не видела текста вокруг курсора и судила набранные буквы как отдельное слово: верная
«е», вставленная в «мня», после паузы становилась «t», а набранная не в той раскладке буква часто
оставалась, если сама по себе на что-то походила.

Теперь при первом нажатии после щелчка, перемещения курсора или перехода в другое окно программа
читает поле и видит буквы вплотную до и после курсора. Если они в другой раскладке, чем
набранное, решается всё слово целиком, а заменяются только набранные буквы; клавиша, которая в
одной раскладке знак, а в другой буква, внутри слова считается буквой («,» между «те» и «е» —
это «б»). Если буквы вокруг в той же раскладке, набранное не трогается, и раннее переключение
раскладки внутри слова не срабатывает. Для этого нужно чтение поля (настройка включена по
умолчанию) и приложение, текст которого система может прочитать, — например, Telegram и VS Code;
удалённый рабочий стол и терминалы свой текст не отдают.

Проверка на 1500 предложениях, в которые возвращали пропущенные буквы: испорченных правок было
2619, стало 0; правок в чужой раскладке исправлено 7464 из 7500 вместо 4390. Обычный набор не
изменился. Проверка на 500 новых предложениях, которых до этого никто не видел: испорченных
правок 921 → 0, исправленных 1460 → 2473 из 2500 (в Firefox 715 → 0 и 1120 → 2472).

## English

Letters typed into a word that is already written are now decided together with that word:
click into "сд|лать", type the missing "е", and the program sees the whole word. The full list
is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.32.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.32.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.32.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.32.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.32.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Fixing a word in place

To fix a word, one clicks into its middle and types the missing letters. The program did not
see the text around the caret and judged the typed letters as a word of their own: a correct
"е" put back into "мня" became "t" at the pause, and a letter typed in the wrong layout often
stayed when it looked like something on its own.

Now the first key after a click, a caret move or another window makes the program read the
field and see the letters right before and after the caret. When they are in the other layout
than the typed ones, the whole word is decided and only the typed letters are replaced; a key
that is punctuation in one layout and a letter in the other counts as a letter inside a word
("," between "те" and "е" is "б"). When the letters around are in the same layout, the typed
ones are left alone, and the early layout switch does not fire inside a word. This needs field
reading (on by default) and an application whose text the system can read, such as Telegram and
VS Code; remote desktops and terminals do not expose theirs.

Checked on 1,500 sentences with missing letters typed back: corrupted fixes went from 2,619 to
none; 7,464 of 7,500 wrong-layout fixes were restored instead of 4,390. Ordinary typing is
unchanged. On 500 new sentences nobody had seen before: corrupted fixes 921 → none, restored
1,460 → 2,473 of 2,500 (in Firefox 715 → none and 1,120 → 2,472).

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

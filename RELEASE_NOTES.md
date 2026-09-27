# KeySwitch 0.33.1

## Русский

Законченные слова теперь решаются одинаково в любом приложении, а слово, набранное не в той
раскладке, исправляется и тогда, когда в нём опечатка или лишняя клавиша. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

0.33.0 не был опубликован: сборка для Windows не приняла новую контекстную
модель. 0.33.1 содержит все изменения 0.33.0 и исправленную сборку.

### Файлы выпуска

- `KeySwitch-Setup-0.33.1-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.33.1-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.33.1_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.33.1-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.33.1-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Одинаково в любом приложении

Контекстная модель знала пять приложений по имени, а в остальных — Firefox, Edge, терминалах — её
ответ отбрасывался в пользу более простого детектора. Теперь модель обучена без имён приложений и
решает везде одинаково; роль поля (текст, код, терминал) по-прежнему учитывается.

### Опечатки, лишняя клавиша и цифры

Модель видит, что прочтение слова отличается от словарного одной опечаткой: лишней, пропущенной или
неверной буквой, переставленными соседними. «aghbdtn» (залипшая «a» перед «ghbdtn») снова становится
«фпривет», а технические имена, которые детектор ошибочно исправляет, например «nextjs» и «.zshrc»,
остаются. Цифра читается вместе с раскладкой букв вокруг неё: «pm2» и путь «/c,jhrb2» остаются,
«зь2» в русской раскладке становится «pm2».

### Сленг и редкие слова

Модель, которая разрешает замену там, где детектор не уверен, теперь знает русские словоформы из
веб-текстов и из прозы, перешедшей в общественное достояние. Поэтому сленг и редкие слова вроде
«фнафер», «куафер» или название раскладки «ЙЦУКЕН», набранные как задумано, остаются, а растянутые
слова («муууу») и лишняя клавиша распознаются. Файл модели вырос с 4.8 до 15 МБ, загрузка занимает
около 1.2 с вместо 0.4 с.

### Проверка

На 3 500 предложениях субтитров, набранных через программу в пяти видах полей, правильно и в чужой
раскладке: испорченных правильных предложений 82 → 41 (в Firefox 45 → 41), восстановленных
16 367 → 16 579 (в Firefox 15 442 → 16 579). На 500 новых предложениях, которых до этого никто не
видел: испорченных 13 → 5 (в Firefox 8 → 5), восстановленных 2 318 → 2 360 из 2 500 (в Firefox
2 193 → 2 360); правка внутри слова 2 480 → 2 485.

## English

Completed words are now decided the same way in every application, and a word typed in the wrong
layout is converted even with a typo or a stray key in it. The full list is in
[CHANGELOG.md](CHANGELOG.md).

0.33.0 was never published: the Windows build rejected the new context model.
0.33.1 carries every change of 0.33.0 and the fixed build.

### Release files

- `KeySwitch-Setup-0.33.1-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.33.1-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.33.1_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.33.1-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.33.1-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### The same in every application

The context model knew five applications by name, and in any other one - Firefox, Edge, a
terminal - its answer was thrown away for the simpler detector's. It is now trained without
application names and decides the same way everywhere; the role of the field (text, code,
terminal) still counts.

### Typos, a stray key and digits

The model now sees when a reading is one typo away from a dictionary word: a letter extra, missing
or wrong, or two neighbours swapped. "aghbdtn" - a stuck "a" before "ghbdtn" - becomes "фпривет"
again, while technical names the detector converts by mistake, such as "nextjs" and ".zshrc", stay.
A digit is read together with the layout of the letters around it: "pm2" and a path like "/c,jhrb2"
stay, "зь2" typed in the Russian layout becomes "pm2".

### Slang and rare words

The model that licenses a conversion where the detector is unsure now knows Russian word forms from
web text and from public-domain prose. Slang and rare words such as "фнафер", "куафер" or the layout
name "ЙЦУКЕН" typed as intended stay, and stretched words ("муууу") and a stray extra key are
recognised. The model file grows from 4.8 to 15 MB and takes about 1.2 s instead of 0.4 s to load.

### How it was checked

On 3,500 subtitle sentences typed through the program into five kinds of field, correctly and in
the other layout: spoiled correct sentences 82 -> 41 (45 -> 41 in Firefox), restored sentences
16,367 -> 16,579 (15,442 -> 16,579 in Firefox). On 500 new sentences nobody had seen before:
spoiled 13 -> 5 (8 -> 5 in Firefox), restored 2,318 -> 2,360 of 2,500 (2,193 -> 2,360 in
Firefox); edits inside a word 2,480 -> 2,485.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

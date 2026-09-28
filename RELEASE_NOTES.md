# KeySwitch 0.33.3

## Русский

Короткое слово, набранное в чужой раскладке, теперь исправляется и само по себе: `ns` в пустом
поле становится «ты». Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.33.3-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.33.3-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.33.3_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.33.3-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.33.3-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Короткое слово само по себе

`ns`, набранное в пустом поле в английской раскладке, на пробеле становится «ты» — так же и после
русского текста. «Ты», набранное как задумано, остаётся, даже если после курсора есть текст. Если
следующее слово всё же английское — `vs code`, набранное подряд, после смены раскладки выходит
«мы сщву», — одна правка возвращает оба слова: `vs code`. Решает контекстная модель: когда следующее
слово известно, программа спрашивает её о первом ещё раз; ясное слово остаётся исправленным —
«спасибо team», «hello привет».

### Что ещё видит модель

Контекстная модель знает, что слово правится внутри (остальные буквы уже в другой раскладке), что
текст после слова — это следующее слово, которое программа собирается исправить, а не текст поля, и
что прямо перед словом стоит косая черта. На 4 000 предложениях субтитров, набранных через программу
в пяти видах полей правильно и в чужой раскладке, испорченных правильных стало 42 вместо 46, а
исправленных — 19 427 вместо 18 939 из 20 000; буквы, вставленные внутрь слова, исправлены в 19 948
предложениях вместо 19 907, и ни одно правильное по-прежнему не испорчено. Имя пакета, набранное
верно после русского слова и косой черты, заменяется по ошибке реже: 66 раз вместо 128 из 23 664.
На 500 новых предложениях, которых не видела ни одна модель: испорченных 3 вместо 5, исправленных
2 413 вместо 2 342 из 2 500.

Цена: шесть латинских сокращений перед «привет» (`cd`, `km`, `lt`, `rd`, `td`, `re`) больше не
становятся русскими словами («св привет», «ку привет»), а имена пакетов, набранные одни в пустом поле,
заменяются по ошибке 16 раз вместо 12 из 1 972.

## English

A short word typed in the other layout is now corrected on its own too: `ns` in an empty field
becomes "ты". The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.33.3-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.33.3-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.33.3_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.33.3-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.33.3-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### A short word on its own

`ns` typed into an empty field in the English layout becomes "ты" at the space, and so it does
after Russian text. "ты" typed as intended stays, even with text after the caret. If the next word
turns out to be English after all - `vs code` typed in one go comes out as "мы сщву" once the layout
follows - one correction returns both words: `vs code`. The context model decides: once the next
word is known, the program asks it about the first word again; a word that reads clearly stays
corrected - "спасибо team", "hello привет".

### What the model now sees

The context model is told that a word is being edited in place (the rest of it already in the
other layout), that the text after a word is the next word the program plans to correct rather
than text already in the field, and that a slash stands right before the word. On 4,000 subtitle
sentences typed through the program into five kinds of field, correctly and in the other layout,
spoiled correct sentences went from 46 to 42 and restored ones from 18,939 to 19,427 of 20,000;
letters put back inside a word restored 19,948 sentences instead of 19,907, still spoiling none. A
package name typed as intended after a Russian word and a slash is replaced by mistake less often:
66 instead of 128 of 23,664. On 500 new sentences no model had seen: spoiled 3 instead of 5,
restored 2,413 instead of 2,342 of 2,500.

The cost: six Latin abbreviations typed before "привет" (`cd`, `km`, `lt`, `rd`, `td`, `re`) no
longer turn into Russian words ("св привет", "ку привет"), and package names typed alone into an
empty field are replaced by mistake 16 times instead of 12 of 1,972.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

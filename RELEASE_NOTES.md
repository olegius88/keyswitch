# KeySwitch 0.33.2

## Русский

На Windows программа читает только текст поля, в котором стоит курсор. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.33.2-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.33.2-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.33.2_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.33.2-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.33.2-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Только текст самого поля

В окнах на Chromium — VS Code и других приложениях на Electron, Chrome, Edge — чтение поля брало до
512 символов страницы или окна вокруг поля ввода. Слово, набранное в пустое окно чата, модель решала
по переписке над ним и подписям под ним: русское «ты» становилось «ns». Все чтения поля в VS Code на
реальной машине начиная с 0.30 содержали 450 и больше символов перед курсором; в Firefox картина та
же. Теперь чтение останавливается на краях поля: пустое поле ввода в Edge читается пустым, а не как
512 символов абзаца над ним, заполненное — ровно своим текстом.

## English

On Windows the program now reads only the text of the field the caret is in. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.33.2-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.33.2-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.33.2_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.33.2-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.33.2-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Only the field's own text

In Chromium-based windows - VS Code and other Electron applications, Chrome, Edge - reading a field
took up to 512 characters of the page or window around the input. A word typed into an empty chat
box was judged by the conversation above it and the controls below: a Russian "ты" became "ns".
Every field read in VS Code on a real machine since 0.30 held 450 or more characters before the
caret, and Firefox reads showed the same pattern. The reader now stops at the edges of the field:
an empty input in Edge reads as empty instead of 512 characters of the paragraph above it, and a
filled one reads as its own text.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

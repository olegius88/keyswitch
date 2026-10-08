# KeySwitch 0.44.0

## Русский

Автоматическая смена раскладки больше не пропадает при быстром наборе. Если следующую букву
нажать раньше, чем отпущен пробел, исправление отменялось, и слово оставалось как набрано вместе
со следующим: `ghbdtn vbh`, `z ctujlyz`, `rfr ltkf` теперь становятся `привет мир`, `я сегодня`,
`как дела`. Ранняя смена раскладки теперь работает в Firefox, приложении Claude, VS Code и других
программах на Electron: они показывают последнюю букву чуть позже, и раньше ранняя смена в них
часто отказывала (в Firefox — в пяти словах из шести). Прокрутка колесом мыши больше не сбрасывает набираемое слово. Полный
перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.44.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.44.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.44.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан
  сертификатом издателя.
- `KeySwitch-0.44.0-windows-arm64.zip` — переносимый архив для Windows на ARM. У ARM64-сборки
  пока нет автоматических обновлений: новую версию скачивайте со страницы выпусков.
- `keyswitch_0.44.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.44.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.44.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Что исправлено

- Быстрый набор. Исправление выполняется, когда отпущена клавиша, которая его запустила; при
  чтении поля (на Windows оно включено по умолчанию) проверка перед исправлением видела в поле
  уже нажатую следующую букву и отменяла замену. Теперь эти буквы ожидаются: на журналах набора,
  воспроизведённых так, как печатает быстрый наборщик, 0.43.0 исправляла верно 218 слов и
  пропускала 911, эта версия — 555 и 73.
- Буква следующего слова, нажатая до отпускания пробела, после исправления печатается в новой
  раскладке и начинает следующее слово как обычно: пара `nats b redis` снова становится
  `nats и redis`.
- Ранняя смена раскладки в браузерах и редакторах на Electron: поле, которое ещё не показало одну
  или две последние буквы, больше не считается изменённым. Если оно отстаёт и в момент замены,
  слово исправляется на своём пробеле, как раньше.
- Колесо мыши больше не считается щелчком: прокрутка не сбрасывает набираемое слово и слова
  перед ним.

### Диагностика

- Если клавиши доходят до KeySwitch с опозданием (подвисание набора), технический журнал раз в
  минуту пишет `input_delay`: сколько клавиш пришло на 100 мс позже и сколько ответов самого
  KeySwitch заняли 20 мс и дольше. Так видно, кто задержал ввод.
- Диагностика называет и модель префиксов (`prefix_model`), которая переключает раскладку во
  время набора, рядом с моделью намерения (`intent-v1-…`) и контекстной моделью (`context-v3-…`).
- Отменённое исправление пишет в журнал, какая проверка поля не прошла (`field_check`).

### Модель

- Контекстная модель `context-v3-2ae75d2b9e38` обучена тем же рецептом, что в 0.43.0, на свежем
  корпусе, чтобы проверка закрепила исправленный движок; решения те же: по журналам набора 726
  верных, 104 пропуска, 68 ложных, как в 0.43.0. Модель префиксов прежняя,
  `prefix-v2-bf3dc28f8567`.
- На новой запечатанной проверке правильно набранный текст не испорчен ни в одной из 195 строк,
  без ранней смены восстановлено 174 строки против 159 у эталонной пары.

### Что осталось

- Короткий латинский термин строчными, набранный в русской раскладке после русского текста,
  может остаться кириллицей: `принимай зк и продолжай` остаётся с `зк` вместо `pr`. Его
  исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере
  по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на
  компьютере, с которого печатают.

## English

Automatic layout switching no longer drops out under fast typing. When the next letter went
down before the space came up, the correction was cancelled and the word stayed as typed, its
next word with it: `ghbdtn vbh`, `z ctujlyz`, `rfr ltkf` now become `привет мир`, `я сегодня`,
`как дела`. The early layout switch now works in Firefox, the Claude app, VS Code and other
Electron programs: they show the last letter a moment later, and the early switch was often
refused there (five words in six in Firefox). Scrolling with the mouse wheel no longer discards the word being typed.
The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.44.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.44.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.44.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a
  publisher certificate either.
- `KeySwitch-0.44.0-windows-arm64.zip` — portable archive for Windows on Arm. The ARM64 build
  does not update itself yet: download a new version from the releases page.
- `keyswitch_0.44.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.44.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.44.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice
versa.

### Fixed

- Fast typing. A correction runs once the key that triggered it is up; with the field reader on
  (the Windows default), the check before the correction saw the next letter already in the field
  and cancelled the replacement. Those letters are now expected: on the typing logs replayed the
  way a fast typist types, 0.43.0 converted 218 words right and missed 911, this version 555
  and 73.
- A letter of the next word pressed before the space came up is typed again in the new layout
  after the correction and starts the next word as usual: `nats b redis` becomes
  `nats и redis` again.
- The early layout switch in browsers and Electron editors: a field that has not shown the last
  one or two letters yet is no longer taken for a changed field. If it is still behind when the
  switch would run, the word is corrected at its space, as before.
- The mouse wheel is no longer taken for a click: scrolling does not discard the word being typed
  and the words before it.

### Diagnostics

- When keys reach KeySwitch late (typing stalls), the technical log writes `input_delay` once a
  minute: how many keys came 100 ms late and how many answers of KeySwitch itself took 20 ms or
  more. It shows who held the input up.
- The diagnostics also name the prefix model (`prefix_model`), which switches the layout while a
  word is typed, beside the intent model (`intent-v1-…`) and the context model (`context-v3-…`).
- A cancelled correction logs which field check failed (`field_check`).

### Model

- The context model `context-v3-2ae75d2b9e38` is trained with the 0.43.0 recipe on a fresh corpus,
  so that the check pins the corrected engine; it decides the same: 726 right, 104 missed and
  68 false on the typing logs, as 0.43.0. The prefix model stays `prefix-v2-bf3dc28f8567`.
- On the new sealed check it corrupted none of 195 correctly typed rows and, without the early
  switch, restored 174 rows against 159 for the reference pair.

### What remains

- A short lower-case Latin term typed in the Russian layout after Russian text may stay
  Cyrillic: `принимай зк и продолжай` keeps `зк` for `pr`. `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the
  controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the
  typing comes from corrects it.

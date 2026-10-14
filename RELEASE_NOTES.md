# KeySwitch 0.56.0

## Русский

В выпуске появилась сборка для 32-битной Windows 10, на которой x64-сборка не запускается. Сборки для Windows на ARM и x86 теперь обновляются сами, каждая установщиком своей архитектуры. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.56.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.56.0-windows-x64.zip` — переносимый архив для Windows x64.
- `KeySwitch-Setup-0.56.0-arm64.exe` — установщик для Windows 11 на ARM. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.56.0-windows-arm64.zip` — переносимый архив для Windows на ARM.
- `KeySwitch-Setup-0.56.0-x86.exe` — установщик для 32-битной Windows 10. Тоже не подписан сертификатом издателя.
- `KeySwitch-0.56.0-windows-x86.zip` — переносимый архив для 32-битной Windows.
- `keyswitch_0.56.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.56.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.56.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и наоборот.

### Что изменилось

- Сборка для 32-битной Windows 10: `KeySwitch-Setup-0.56.0-x86.exe` и `KeySwitch-0.56.0-windows-x86.zip`. На 64-битной Windows ставьте сборку x64 (или ARM64 на компьютере с ARM): 32-битная там тоже работает, но обновляется только 32-битными выпусками.
- Сборки для Windows на ARM и x86 обновляются сами, как x64: каждая скачивает установщик своей архитектуры. Сборка x64, запущенная на компьютере с ARM, тоже начинает обновляться. Прежние версии на ARM об этих установщиках не знают, поэтому эту версию на таком компьютере нужно один раз поставить вручную.
- В техническом журнале при запуске записывается, для какой платформы собрана программа (`build_platform`): у одной версии теперь три сборки для Windows.

### Модель

- Контекстная модель `context-v3-20be427844f6` (корпус v61) обучена заново тем же рецептом, что и модель 0.55.0, чтобы печать закрепила новые вызовы Windows; модель префиксов прежняя, `prefix-v2-bf3dc28f8567`. На калибровочных данных она делает на 2 и 1 верных исправления больше модели 0.55.0 и на одно ложное меньше, а на журналах реального набора решает так же, как она. На запечатанном test v61 правильный текст сохранён во всех 181 строках, а без ранней смены восстановлено 179 строк из 180, набранных не в той раскладке, против 177–178 у базовой пары.

### Что осталось

- Сборку x86 проверяли тестами и проверкой нажатий на 64-битной Windows, где 32-битные программы работают через WOW64: машин с 32-битной Windows в CI нет.
- Короткое техническое слово, впервые набранное в русской раскладке посреди русской фразы (`цуиоы` вместо `webjs`, `мгуоы` вместо `vuejs`, `реьч` вместо `htmx`), часто остаётся кириллицей: без того же слова раньше в тексте модель не отличает его от незнакомого русского слова, сленга или опечатки. Его исправляет `Pause`, теперь и после пробелов и знаков за словом.
- Английское слово через дефис, набранное целиком в русской раскладке (`зку-щквук` вместо `pre-order`), исправляется только после дефиса: получается `зку-order`.
- Термин, процитированный в сообщении об ошибке так, как он набрался (`webjs набралось как цуиоы`), переводится в `webjs`, если этот же термин уже набран выше. Вернуть набранное можно клавишей `Pause` сразу после исправления.
- Латинская аббревиатура, набранная в русской раскладке после английского текста, восстанавливается не всегда: `Ask РК about the leave` (`HR`) остаётся `РК`. Русская аббревиатура после латиницы без словарей Hunspell иногда становится латиницей: `deploy staging ЗП` становится `deploy staging PG`. Слово исправляет или возвращает клавиша `Pause`.
- `Pause` сразу после исправления на пробеле по-прежнему возвращает слово, даже если исправление было верным: по времени нажатия не отличить, отменяете вы ошибочное исправление или нажали по привычке.
- `ЗК` заглавными в конце русской фразы (`почему ты не создал ЗК?`) остаётся кириллицей, а не становится `PR`: следующего слова, с которым модель решила бы его, нет. Его исправляет клавиша `Pause`.
- `кз` в русской фразе (`говорит он кз домене`) может стать `rp`: в таблицах частот, которыми модель проверяет слова, нет разговорного русского, а `rp` там встречается. Вернуть слово можно клавишей `Pause`.
- Русская аббревиатура заглавными, набранная в английской раскладке посреди русской фразы (`DKRCV` вместо «ВЛКСМ»), остаётся латиницей: по имеющимся данным её не отличить от латинской аббревиатуры (`по версии WBC`). Её исправляет клавиша `Pause`.
- Набор через TeamViewer и другие программы удалённого доступа на управляемом компьютере по-прежнему не обрабатывается (настройка «Ввод от других программ»): исправляет KeySwitch на компьютере, с которого печатают.
- В окнах программ, запущенных с правами администратора, KeySwitch не видит нажатий, пока сам не запущен с такими же правами (ограничение Windows).

## English

This release adds a build for 32-bit Windows 10, which cannot run the x64 one. The Windows on Arm and x86 builds now update themselves, each with the installer of its own architecture. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.56.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.56.0-windows-x64.zip` — portable archive for Windows x64.
- `KeySwitch-Setup-0.56.0-arm64.exe` — installer for Windows 11 on Arm. It is not signed with a publisher certificate either.
- `KeySwitch-0.56.0-windows-arm64.zip` — portable archive for Windows on Arm.
- `KeySwitch-Setup-0.56.0-x86.exe` — installer for 32-bit Windows 10. It is not signed with a publisher certificate either.
- `KeySwitch-0.56.0-windows-x86.zip` — portable archive for 32-bit Windows.
- `keyswitch_0.56.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer installed, this version arrives through the ordinary system update.
- `KeySwitch-0.56.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.56.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is needed: the Apple silicon build does not run on Intel and vice versa.

### Changed

- A build for 32-bit Windows 10: `KeySwitch-Setup-0.56.0-x86.exe` and `KeySwitch-0.56.0-windows-x86.zip`. On 64-bit Windows install the x64 build (or the ARM64 one on an Arm PC): the 32-bit one runs there too, but updates only with 32-bit releases.
- The Windows on Arm and x86 builds update themselves, as the x64 one does: each downloads the installer of its own architecture. An x64 build running on an Arm PC updates itself too now. Earlier versions on Arm do not know of these installers, so this version has to be installed there by hand once.
- The technical log records at start which platform the program was built for (`build_platform`): one version now ships three Windows builds.

### Model

- The context model `context-v3-20be427844f6` (corpus v61) is retrained with the recipe of the 0.55.0 model, so that its seal pins the new Windows calls; the prefix model is still `prefix-v2-bf3dc28f8567`. On the calibration data it makes 2 and 1 more right conversions than the 0.55.0 model and one false conversion fewer, and on logs of real typing it decides as that model does. On the sealed test v61 it kept all 181 correctly typed rows and, without the early switch, restored 179 of the 180 typed in the wrong layout against 177–178 for the baseline pair.

### What remains

- The x86 build is tested, keystrokes included, on 64-bit Windows, where 32-bit programs run through WOW64: CI has no machine with a 32-bit Windows.
- A short technical word typed in the Russian layout amid a Russian phrase for the first time in a text (`цуиоы` for `webjs`, `мгуоы` for `vuejs`, `реьч` for `htmx`) often stays Cyrillic: without the same word earlier in the text the model cannot tell it from an unknown Russian word, slang or a typo. `Pause` corrects it, now also past the spaces and signs after the word.
- An English word joined by a hyphen and typed whole in the Russian layout (`зку-щквук` for `pre-order`) is converted only after the hyphen: it becomes `зку-order`.
- A term quoted in a bug report the way it came out (`webjs набралось как цуиоы`) is converted to `webjs` when the same term was typed earlier in the text. `Pause` right after the correction brings back what was typed.
- A Latin abbreviation typed in the Russian layout after English text is not always restored: `Ask РК about the leave` (`HR`) stays `РК`. Without Hunspell dictionaries a Russian abbreviation after Latin-script text sometimes becomes Latin: `deploy staging ЗП` becomes `deploy staging PG`. `Pause` corrects the word or brings it back.
- `Pause` right after a correction made at a space still converts the word back, even a right one: the moment of the press cannot tell an undo of a wrong correction from a press out of habit.
- `ЗК` in capitals at the end of a Russian phrase (`почему ты не создал ЗК?`) stays Cyrillic instead of becoming `PR`: there is no next word for the model to decide it with. `Pause` corrects it.
- `кз` in a Russian phrase (`говорит он кз домене`) may become `rp`: the frequency tables the model checks words with hold no conversational Russian, while `rp` occurs there. `Pause` brings the word back.
- A Russian abbreviation in capitals typed in the English layout amid a Russian phrase (`DKRCV` for `ВЛКСМ`) stays Latin: with the evidence at hand it cannot be told from a Latin abbreviation (`по версии WBC`). `Pause` corrects it.
- Typing through TeamViewer and other remote-control programs is still not handled on the controlled computer (the «Ввод от других программ» setting): KeySwitch on the computer the typing comes from corrects it.
- In windows of programs running as administrator KeySwitch sees no keys unless it runs with the same rights (a Windows restriction).

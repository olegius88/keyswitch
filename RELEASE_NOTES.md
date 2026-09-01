# KeySwitch 0.6.0

## Русский

Главное изменение выпуска — собственная локальная линейная n-граммная модель
для более точного распознавания текста, набранного в неправильной раскладке.

- Модель сравнивает EN/RU-интерпретации одной физической последовательности
  клавиш. Она использует символьные 1–5-граммы, направление раскладки, длину и
  способ завершения слова, работает полностью локально и не отправляет
  пользовательский текст в сеть.
- Жёсткие защиты URL, путей, кода, аббревиатур, коротких и неоднозначных слов,
  приложений-исключений и пользовательских правил остаются выше модели. Если
  модель недоступна, KeySwitch безопасно возвращается к консервативному
  детерминированному распознаванию.
- Сертифицированный кандидат `intent-v1-6bf96537c28f` прошёл все 30 strict
  gates. На независимом model-blind unknown-typo holdout зафиксировано 0 ложных
  переключений из 60 000 негативных примеров; два независимых переобучения
  побайтно воспроизвели KSLM, manifest и test report.
- Точные frozen EN/RU Onboard-источники, их исходная лицензия и
  checksum-validated KSLM теперь входят в Linux и Windows пакеты. Обычный ввод
  пользователя не используется для общего обучения модели.
- В меню индикатора появилась команда выбора языка, противоположного текущему:
  при RU доступен английский, при EN — русский.
- Linux и Windows packaging выполняют fail-closed проверку модели, provenance,
  размеров контейнера и фактически установленного native runtime. DEB не
  зависит от системного интерпретатора Python.
- Добавлены подробные runbook и cookbook по созданию корпуса, preseal,
  обучению, независимой проверке, воспроизводимости и выпуску модели.

Проверочный контур включает строгую типизацию Python, 100% покрытия строк и
ветвей, detector/model quality gates, настоящие X11/DBusMenu и Win32 E2E,
проверку нативного DEB, тихую установку и автообновление Windows Setup.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.6.0-x64.exe` или переносимый
  `KeySwitch-0.6.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.6.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

The main addition in this release is a first-party local linear n-gram model
for more accurate detection of text entered with the wrong keyboard layout.

- The model compares the EN/RU interpretations of the same physical key
  sequence. It uses character 1–5-grams, layout direction, token length and the
  word-completion trigger, runs entirely locally and never sends user text over
  the network.
- Hard guards for URLs, paths, code, abbreviations, short or ambiguous words,
  excluded applications and explicit user rules remain above the model. If the
  artifact is unavailable, KeySwitch safely falls back to its conservative
  deterministic detector.
- The certified `intent-v1-6bf96537c28f` candidate passed all 30 strict gates.
  On an independent model-blind unknown-typo holdout it produced 0 false
  switches among 60,000 negative examples; two independent retraining runs
  reproduced the KSLM, manifest and test report byte for byte.
- Exact frozen EN/RU Onboard sources, their original license evidence and the
  checksum-validated KSLM are now bundled in both Linux and Windows packages.
  Ordinary user input is not used to train the shared model.
- The tray menu now offers the language opposite to the active one: English
  while RU is active and Russian while EN is active.
- Linux and Windows packaging fail closed on model provenance, container
  bounds and the model loaded by the installed native runtime. The DEB does not
  depend on the system Python interpreter.
- A detailed runbook and cookbook now document corpus creation, preseal,
  training, independent evaluation, reproducibility and model release.

The release quality contour includes strict Python typing, mandatory 100% line
and branch coverage, detector/model quality gates, real X11/DBusMenu and Win32
E2E tests, native DEB verification, silent Windows installation and automatic
update relaunch testing.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.6.0-x64.exe` or the portable
  `KeySwitch-0.6.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.6.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

# KeySwitch 0.4.0

## Русский

В этом выпуске автокоррекция стала ближе к EveryLang и Punto Switcher.

- Неверно набранное слово теперь может исправиться уже через 1,5 секунды паузы,
  не дожидаясь пробела или знака препинания. Эту проверку можно отдельно
  отключить в настройках.
- После ручного преобразования слова клавишей `Pause/Break` над местом ввода
  появляется вопрос о добавлении слова в правила. `Enter` подтверждает правило,
  `Esc`, посторонний ввод или тайм-аут закрывают предложение.
- Подсказка располагается у текстового курсора, когда приложение предоставляет
  его координаты; после закрытия фокус возвращается исходному окну.
- В Windows флаги EN/RU занимают максимально доступную площадь значка в области
  уведомлений и больше не имеют фиолетовой рамки.
- Сохранено обучение по повторным ручным преобразованиям как настраиваемый
  резервный механизм.

Пакеты выпуска проходят строгую проверку типов, 100% покрытия строк и ветвей,
реальные X11/Win32 E2E, проверку нативного DEB и тихую установку Windows Setup.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.4.0-x64.exe` или переносимый
  `KeySwitch-0.4.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.4.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This release brings automatic correction closer to EveryLang and Punto
Switcher.

- A likely wrong-layout word can now be corrected after 1.5 seconds of idle
  time, before a separator is typed. Idle correction has its own settings
  switch.
- After a manual `Pause/Break` conversion, a prompt above the input position
  offers to add the word to switching rules. `Enter` confirms it; `Esc`,
  unrelated input or a timeout dismisses it.
- The prompt follows the text caret when its coordinates are available and
  restores focus to the original target after closing.
- Windows EN/RU flag icons now fill the available notification-area icon canvas
  and no longer have the purple frame.
- Repeated manual conversions remain available as a configurable fallback
  learning mechanism.

Release packages pass strict type checking, 100% line and branch coverage,
real X11/Win32 E2E tests, native DEB verification and a silent Windows Setup
installation test.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.4.0-x64.exe` or the portable
  `KeySwitch-0.4.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.4.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.

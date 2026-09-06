# KeySwitch 0.16.2

## Русский

Исправления надёжности ввода и более точная диагностика. Рабочие веса моделей
и пользовательские настройки по умолчанию сохранены.

- Контекст фразы и целое текущее слово больше не сбрасываются, если во время
  замены повторно доставлены только подтверждённые отпускания клавиш,
  без новых нажатий или неизвестного ввода.
- Повторный Pause не переключает язык поверх ожидающей ручной замены.
  Если отпускание клавиши не получено, ручная замена или отмена безопасно
  прекращается через три секунды с понятным статусом. Таймаут не имитирует
  отпускание физически удерживаемой клавиши.
- Пунктуация при отмене замены сохраняется в раскладке, в которой была
  набрана, в Windows и Linux X11.
- Журнал различает решение контекстной модели, резервное решение,
  неподдержанный контекст и shadow-режим. Добавлены состояние читателя поля
  и причина сброса контекста при инъекции — без записи соседнего текста.
- Добавлены экспериментальные отложенные границы слова, сохранение хвоста
  пунктуации и проверки порядка «замена → Enter». Отдельный boundary-v1
  обучен на публичных данных, но **отклонён и не включён в приложение**:
  на 11 992 отложенных примерах — одна ошибка трактовки пунктуации и 9 998
  отказов от решения. Прежний рабочий алгоритм границ остаётся активным.
- Сборка защищена от случайного включения отклонённых весов. Обучение и
  регрессионные сравнения воспроизводимы; предыдущие отчёты сохранены.

Это не обещание исправить все неоднозначные слова, опечатки или случаи
смешанного ввода. `text_verified=false` по-прежнему означает, что конечный
текст в стороннем приложении не был прочитан и проверен после инъекции.
Приватные журналы не входят в исследовательские данные. LogCourier не меняется.

[Эксперимент и ограничения](https://github.com/olegius88/keyswitch/blob/v0.16.2/model/boundary_v1/README.md) ·
[Диагностика ввода](https://github.com/olegius88/keyswitch/blob/v0.16.2/docs/troubleshooting.md).

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.16.2-x64.exe` или
  `KeySwitch-0.16.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.2_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается.

## English

Input reliability fixes and clearer diagnostics. Shipping model weights and
user defaults remain unchanged.

- Preserve phrase context and whole-word continuation when replay consists
  only of confirmed key releases, without new presses or unknown input.
- Do not switch layouts on a repeated Pause while a manual correction is
  waiting. Cancel pending manual corrections/undo safely after a three-second
  release timeout; never infer a physical key-up solely from elapsed time.
- Replay literal punctuation in its original layout during correction undo
  on Windows and Linux X11.
- Distinguish contextual decisions, baseline fallbacks, unsupported context
  and shadow results in diagnostics. Report field-reader state and context
  reset reasons without logging surrounding text.
- Add experimental delayed segmentation and punctuation-tail execution with
  correct-before-Enter regression coverage. Train a separate public-data
  boundary ranker, but **reject it for activation**: one segmentation error
  and 9,998 abstentions on 11,992 held-out examples. The existing boundary
  policy remains active; this is not a model-quality upgrade.
- Add reproducible training/replays and package guards against accidental
  rejected-weight installation; retain earlier regression evidence.

Ambiguous words, spelling errors and mixed-language input still have known
limitations. `text_verified=false` still means the final application text was
not read and verified after injection. No private logs are included in the
research corpus. LogCourier is unchanged.

See the [experiment report](https://github.com/olegius88/keyswitch/blob/v0.16.2/model/boundary_v1/README.md).

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.16.2-x64.exe` or
  `KeySwitch-0.16.2-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.2_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

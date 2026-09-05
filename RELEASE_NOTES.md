# KeySwitch 0.16.0

## Русский

Этот выпуск развивает обучение и проверку контекстной модели. Рабочие веса,
режим автозамены и настройки пользователя **не изменены**: новый кандидат
не прошёл условия качества и не включён в установщики как рабочая модель.

- Добавлен зафиксированный CC0-корпус: 64 537 русских и английских предложений.
  Похожие тексты объединяются до разделения на обучение и независимую проверку.
  Есть отдельный тест незнакомых целевых слов и неиспользованный резерв.
- Подготовлено 224 693 ситуации: ошибки раскладки, правильные слова, опечатки,
  короткие слова, апострофы, дефисы, команды, переменные, адреса и смешанный
  технический текст. Исходные фразы реальные; ошибки и метки действий созданы
  искусственно. Частные логи и переписка в корпус не включены.
- Обучение, выбор порога и итоговые тесты разделены. Словарные оценки
  зафиксированы для двух условий: с эталонным Hunspell и без него.
  Повторное обучение воспроизводит веса и отчёты побайтно.
- Проверен полный путь через движок и виртуальное текстовое поле:
  учитывается смена раскладки после каждой замены, сравнивается конечный текст.
  Это дополнение к существующим нативным тестам, не имитация всех приложений ОС.
- На проверке новых фраз без Hunspell кандидат сделал 1 ложную замену против
  4 у текущей модели, но выполнил лишь 7 006 нужных замен против 8 024.
  В тесте целых фраз восстановил 43/128 против 90/128. Поэтому его веса
  оставлены исследовательским артефактом и не активируются в приложении.
- Обе платформенные сборки теперь проверяют происхождение результатов и
  запрещают случайную подмену рабочей модели отклонённым кандидатом.

Подробности, исходники и ограничения:
[отчёт эксперимента](https://github.com/olegius88/keyswitch/blob/v0.16.0/model/context_v2/README.md).
Большой словарь и хорошие синтетические показатели сами по себе не доказывают
понимание намерений пользователя или отсутствие ошибок в реальной переписке.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.16.0-x64.exe` или
  `KeySwitch-0.16.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. Поведение контекстного помощника из 0.15.0 сохранено.

## English

This release expands context-model training and validation. Shipping weights,
correction behavior and user settings are **unchanged**. The new candidate
failed promotion requirements and is not activated in the application.

- Freeze 64,537 CC0 English/Russian sentences with source-family splitting,
  development/calibration/test separation, focus-lexical holdout and an unused
  reserve. Layout/spelling errors and action labels are synthetic interventions
  on public text, not observed human intent or private logs.
- Build 224,693 action situations, including correct prose, spelling mistakes,
  short words, apostrophes/hyphens and technical keep cases. Reproduce training
  using fixed portable and reference-Hunspell lexical evidence and a
  parity-tested training-only native optimizer.
- Seal weights before test scoring and replay both model decisions and the
  actual engine through a visible-editor harness. Native platform E2E tests
  remain separate and required.
- Preserve the failed result: on the portable phrase test the candidate makes
  1 false conversion versus 4 for the shipping policy, but only 7,006 correct
  conversions versus 8,024. Whole-phrase replay restores 43/128 versus 90/128.
  Fewer false conversions do not justify this recall regression.
- Both native package builds verify evidence provenance and prevent accidental
  installation of the rejected research weights as the active model.

See the [experiment report](https://github.com/olegius88/keyswitch/blob/v0.16.0/model/context_v2/README.md)
for exact scope, limitations and reproduction commands. No universal language
understanding or real-world error-rate guarantee is claimed.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.16.0-x64.exe` or
  `KeySwitch-0.16.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.16.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

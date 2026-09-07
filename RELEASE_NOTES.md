# KeySwitch 0.17.1

## Русский

Исправлена ошибка сборки Windows из 0.17.0: вывод русских примеров в отчёте
проверки модели падал с `UnicodeEncodeError` при кодировке `cp1252`.
JSON теперь использует Unicode-экранирование без потери содержимого; проверки
качества не отключены и веса не изменены. Добавлен запуск настоящего CLI
в тестах с `cp1252`, ASCII и UTF-8 и проверкой полного совпадения данных.
Публикация 0.17.0 была остановлена; этот выпуск включает её доработки ниже.

Обученная ранняя смена раскладки при включённом контекстном помощнике:
например, `ghbd` → `прив` ещё до окончания слова, с продолжением набора
в русской раскладке. Сохранённые настройки и значения по умолчанию не сбрасываются.

- Отдельная локальная модель prefix-v1 обучена на последовательностях
  незаконченных слов. Она работает в `assist`, если включены «Учитывать
  контекст» и «Ранняя смена раскладки». Модель завершённых слов не подменяется.
- Проверенная область ранней модели — 4–12 символов. Настройка минимума может
  отложить проверку; значение 3 действует только в словарном алгоритме.
  При сомнении модель ждёт продолжения. В `off`/`shadow` или без контекста
  сохраняется прежний алгоритм раннего переключения.
- Перед заменой после отпускания клавиш повторно проверяются прогноз,
  текущий префикс, поле и настройки. Сохраняются продолжение слова, пробелы,
  отмена и оценка уверенности. Ручной выбор, исключения и защиты ввода
  остаются приоритетнее вероятностного решения.
- В Windows и Linux обновлены описания контекстного помощника, базовой модели
  слов и раннего переключения. Чтение настоящего поля по-прежнему включается
  отдельно; соседний текст не добавляется в диагностику `prefix_decision`.
- Добавлены публичный корпус, разделение по семействам префиксов, воспроизводимое
  обучение и сквозная проверка точного текста. Сборки проверяют целостность и
  результаты новой модели; отклонённые context-v2 и boundary-v1 остаются выключены.

На отложенном синтетическом тесте ранняя модель исправила до конца слова
94.70% / 95.62% нужных случаев в двух словарных профилях. Остались две ложные
замены опечаток в каждом профиле — это одни и те же ситуации, не четыре
независимые ошибки. Это не оценка всех реальных приложений и намерений.
В сквозном сравнении сохраняются прежние ложные вмешательства завершённой
модели в 10 из 160 отрицательных примеров; этот выпуск их не устраняет.
Приватные логи не используются в обучении и не публикуются.

В исходниках LogCourier добавлены маркеры смены версии KeySwitch, разделение
фрагментов по версиям и выбор актуальной версии при получении логов по
умолчанию. Старые версии доступны явно. Это отдельное приложение: данный
релиз KeySwitch не обновляет установленный LogCourier и не публикует его установщик.

[Обучение, результаты и ограничения](https://github.com/olegius88/keyswitch/blob/v0.17.1/model/prefix_v1/README.md) ·
[Настройки помощника](https://github.com/olegius88/keyswitch/blob/v0.17.1/docs/context-assistant.md).

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.17.1-x64.exe` или
  `KeySwitch-0.17.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.17.1_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. `text_verified=false` в журнале не подтверждает конечный
текст стороннего приложения: после инъекции он не прочитан и не проверен.

## English

Fix the Windows build failure in 0.17.0: printing Cyrillic model-report
examples raised `UnicodeEncodeError` with cp1252 stdout. Emit lossless JSON
Unicode escapes without disabling quality gates or changing model weights.
Real CLI regression tests cover cp1252, ASCII and UTF-8 with exact report
round-tripping. Publication of 0.17.0 was blocked; this release includes its
changes below.

Trained mid-word layout correction with contextual assist enabled: for example,
`ghbd` → `прив` before the word is finished, then typing continues in Russian.
Saved preferences and default values are preserved.

- A separate local prefix-v1 model is trained on unfinished-word sequences.
  It runs in `assist` with context and early switching enabled. The shipping
  completed-word model is unchanged.
- The validated prefix range is 4–12 characters. The minimum-length setting
  can delay evaluation; a value of 3 applies only to the dictionary algorithm.
  Uncertain prefixes wait for continuation. `off`/`shadow` and disabled context
  retain the legacy early-switch algorithm.
- After key release, revalidate the prediction, current prefix, field and
  settings before replacement. Preserve continuation, spaces, undo and
  confidence. Manual intent, exclusions and input-integrity guards retain priority.
- Clarify Windows/Linux settings for the separate models and early switching.
  Actual field reading remains opt-in; `prefix_decision` diagnostics do not
  include surrounding text.
- Add a public corpus, prefix-family splits, reproducible training, exact-text
  engine replay and package promotion checks. Rejected context-v2 and boundary-v1
  candidates remain inactive.

On the held-out synthetic test, the prefix model corrected 94.70% / 95.62% of
desired cases before word completion in two dictionary profiles. Two spelling
errors still triggered false conversions in each profile: the same two cases,
not four independent errors. These are not real-world error-rate guarantees.
The engine comparison retains existing completed-word false interventions in
10 of 160 negative cases; this release does not fix them. Private logs are not
used for training or published.

LogCourier sources now mark KeySwitch version changes, separate fragments by
version and default log retrieval to the current version. Older versions remain
explicitly accessible. LogCourier is a separate application: this KeySwitch
release neither updates an installed collector nor publishes its installer.

See the [training report and limitations](https://github.com/olegius88/keyswitch/blob/v0.17.1/model/prefix_v1/README.md).

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.17.1-x64.exe` or
  `KeySwitch-0.17.1-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.17.1_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.
`text_verified=false` means final application text was not read and verified
after injection, even if input events were sent successfully.

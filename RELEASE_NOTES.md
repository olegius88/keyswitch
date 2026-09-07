# KeySwitch 0.18.0

## Русский

Обученная обработка неоднозначной пунктуации, исправление Pause и восстановление
чтения контекста после временных ошибок.

- Новый классификатор boundary-v2 выбирает границы завершённого слова и
  буквальную пунктуацию в его конце. Неоднозначная клавиша внутри слова больше
  не обрезает его преждевременно: например, `ghj,ktvf ` преобразуется в
  `проблема `. Обычная запятая после слова сохраняется. При недостаточной
  уверенности модель воздерживается от решения.
- Ручной Pause отменяет устаревшее ожидание продолжения короткого слова,
  в том числе когда замена ещё ждёт отпускания физически удерживаемой клавиши.
  Следующий ввод не остаётся связанным с прежним ожиданием.
- Включённый читатель поля восстанавливается после временного сбоя: до трёх
  повторов с задержками 5, 15 и 60 секунд, при следующем разрешённом чтении.
  Нет повторов при отсутствующей зависимости, явном отказе в доступе или
  ошибке закрытия старого провайдера. Журнал показывает состояние повторов
  без текста исключений.
- Проверки включают точный текст, пробелы, пунктуацию, отмену замены и
  порядок Enter/Tab. Пакеты проверяют соответствие принятому артефакту модели.
  Изолированные Linux GUI-тесты больше не запускают ненужный портал документов;
  системные права и настройки рабочего стола не меняются.

Boundary-v2 — отдельная небольшая модель границ слова, не LLM и не замена
классификатора языкового намерения. Веса intent, context и модели переключения
на полуслове не менялись. На заранее отделённой контрольной выборке: 1 996
верных решений, 126 отказов и 0 ошибок; это не гарантия отсутствия ошибок
в реальном вводе. Известные ошибки development-выборки и ограничения описаны
в [карточке модели](https://github.com/olegius88/keyswitch/blob/v0.18.0/model/boundary_v2/README.md).

Одиночная `z ` без надёжного контекста по-прежнему может ожидать продолжения.
`text_verified=false` не подтверждает конечный текст в приложении. Системный
сбой инициализации AT-SPI может потребовать перезапуска KeySwitch. Новая модель
не читает соседний текст; чтение поля остаётся отдельной явно включаемой опцией.

Технический журнал включается явно и по-прежнему может содержать анализируемые
слова в существующих событиях. Просматривайте его перед передачей. Приватные
логи и инструкции в выпуск не включены. LogCourier этим выпуском не обновляется.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.18.0-x64.exe` или
  `KeySwitch-0.18.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.18.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. [Описание диагностики](https://github.com/olegius88/keyswitch/blob/v0.18.0/docs/troubleshooting.md).

## English

Learned completed-token boundaries, reliable manual-wait cancellation and
bounded field-reader recovery.

- The accepted boundary-v2 ranker preserves ambiguous internal punctuation until
  continuation, a hard boundary or enabled idle timeout. It chooses the word
  span and literal trailing signs, or abstains. For example, `ghj,ktvf ` becomes
  `проблема ` without prematurely splitting at the comma key.
- Manual Pause cancels stale contextual lookahead even when the accepted command
  still awaits physical key releases.
- Transient field-reader failures receive at most three retries after 5/15/60
  seconds, on the next allowed read. Missing dependencies, explicit access denial
  and failed cleanup are not retried. Recovery diagnostics omit exception text.
- Exact-text, undo, Enter/Tab and native X11 regressions accompany sealed model
  and package checks. Isolated Linux GUI checks no longer start unnecessary
  document portals; desktop settings and system permissions stay unchanged.

Boundary-v2 is a small segmentation model, not an LLM or a replacement for
language intent. Intent, context and mid-word model weights are unchanged.
Its sealed test has 1,996 correct decisions, 126 abstentions and zero errors;
this is not a guarantee for real-world input. Known development errors and
evaluation limits are retained in the model card. An isolated `z ` without
trusted context can still wait. `text_verified=false` does not prove final
application text. A cached native AT-SPI initialization failure may require
restarting KeySwitch. Field reading remains opt-in.

Technical logs can still contain evaluated words in existing events: review
them before sharing. Private logs and local instructions are not published.
This release does not update LogCourier.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.18.0-x64.exe` or
  `KeySwitch-0.18.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.18.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

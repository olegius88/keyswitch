# KeySwitch 0.19.0

## Русский

Переобученная контекстная модель завершённых слов и слово после косой черты.

- Контекстная политика `context-v1-953375a70173` обучена на корпусе, который
  воспроизводит то, что ей действительно передаёт движок. Исправлены два
  расхождения: без чтения поля движок сообщает роль поля `unknown` для любого
  приложения, а признак «база предлагает замену» строится всей цепочкой
  решения, включая курируемое короткословное исключение. Добавлены семейства
  технических терминов и точечных файлов вне словаря, русского и английского
  жаргона, опечаток и слова после косой черты.
- Слово после буквальной `/` анализируется отдельно. Раньше `bild/c,jhrb` и
  команда чата `/c,jhrb` целиком считались кодом и до модели не доходили;
  теперь это `bild/сборки` и `/сборки`. Сама голова токена не заменяется, а при
  неоднозначной голове замены нет вовсе. Пути `/usr/local/bin`,
  `src/keyswitch/engine` и команды `/start` не затрагиваются.
- Курируемое исключение для коротких слов не отменяется вероятностной оценкой
  модели. Ожидание продолжения по-прежнему может отложить такую замену.
- Эпоха обучения выбирается по метрике поведения при нуле ложных замен на
  настроечной выборке, а не по функции потерь: прежнее правило останавливало
  обучение раньше, чем модель достигала неизменного порога замены 0.985,
  из-за чего верное решение не меняло текст.

На независимой контрольной выборке из 36 618 ситуаций и 187 непересекающихся
семейств: 16 529 нужных замен из 17 658 и 0 ложных замен. Прежний изолированный
распознаватель даёт 14 028 замен и 64 ложных. Это синтетический набор
собственного авторства, а не измерение на реальной переписке. Известные
ограничения — в [карточке модели](https://github.com/olegius88/keyswitch/blob/v0.19.0/docs/context-assistant.md).

Технические термины вне словаря остаются самой слабой категорией: если цели нет
ни в одном словаре, а имя приложения незнакомо, модель предлагает замену, но не
выполняет её. Веса intent, prefix и boundary не менялись. Порог замены не
понижался. `text_verified=false` по-прежнему не подтверждает конечный текст
в приложении. Чтение поля остаётся отдельной явно включаемой опцией.

Технический журнал включается явно и может содержать анализируемые слова.
Просматривайте его перед передачей. Приватные логи и локальные инструкции
в выпуск не включены. LogCourier этим выпуском не обновляется.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.19.0-x64.exe` или
  `KeySwitch-0.19.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.19.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. [Описание диагностики](https://github.com/olegius88/keyswitch/blob/v0.19.0/docs/troubleshooting.md).

## English

A retrained completed-word context policy and the word after a literal slash.

- Context policy `context-v1-953375a70173` is trained on a corpus that
  reproduces what the engine actually sends it. Two mismatches are fixed:
  without field reading the engine reports the `unknown` field role for every
  application, and the "baseline converts" feature is now built by the whole
  decision chain, including the curated short-word exception. New families
  cover out-of-lexicon technical terms and dotfiles, Russian and English
  jargon, misspellings and the word after a slash.
- The word after a literal `/` is analysed separately. `bild/c,jhrb` and a chat
  `/c,jhrb` were previously treated as code in full and never reached the
  model; they now become `bild/сборки` and `/сборки`. The head is never
  replaced, and an ambiguous head suppresses the conversion entirely. Paths
  such as `/usr/local/bin`, `src/keyswitch/engine` and `/start` are untouched.
- A curated short-word exception is no longer cancelled by a probabilistic
  verdict. Contextual lookahead can still delay it.
- The training epoch is selected by the development action metric under a
  zero-false-conversion budget instead of weighted log-loss, which stopped
  before the model reached the unchanged 0.985 serving threshold and left
  correct decisions without any effect on text.

On an independent holdout of 36,618 situations across 187 disjoint families:
16,529 of 17,658 required conversions and zero false conversions. The isolated
legacy recogniser scores 14,028 conversions with 64 false ones. This is an
author-written synthetic set, not a measurement on real conversations.

Out-of-lexicon technical terms remain the weakest category: with no dictionary
support and an unfamiliar application name the model suggests rather than
converts. Intent, prefix and boundary weights are unchanged. The conversion
threshold was not lowered. `text_verified=false` still does not prove the final
application text. Field reading remains opt-in.

Technical logs can contain evaluated words: review them before sharing. Private
logs and local instructions are not published. This release does not update
LogCourier.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.19.0-x64.exe` or
  `KeySwitch-0.19.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.19.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

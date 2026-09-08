# KeySwitch 0.20.0

## Русский

Модель орфотактики: решение по последовательности клавиш, а не по словарю.

- Словарные признаки бессильны для токена, которого нет ни в русском, ни в
  английском словаре. `htop`, `webjs`, `.dist`, `pm2`, набранные не в той
  раскладке, прежде оставались как есть, и их приходилось править вручную.
- Движок наблюдает не видимую строку, а последовательность физических клавиш.
  Отображение us/ru — биекция, поэтому у одной последовательности ровно два
  прочтения. Новая счётная модель `ortho-v1-bdb915e4f06f` сравнивает их
  правдоподобия и отвечает, была ли раскладка неверной, ни разу не спрашивая,
  слово ли это. Она не оценивает, хорош ли `htop` как английское слово, — она
  оценивает, хуже ли `рещз` как русская последовательность.
- Второй канал — регистр. Самые «английские» настоящие русские
  последовательности — аббревиатуры: `РСФСР`, `ЛДПР`, `РНК`. Аббревиатуру пишут
  заглавными, поэтому строчный токен этим прочтением не оправдать. Полностью
  заглавный токен и так не доходит до модели: его отсекает защита кода.
- Модель только разрешает замену там, где базовый распознаватель воздержался.
  Она никогда не запрещает замену, не отменяет отказ контекстной модели,
  уступает ожиданию продолжения и стоит ниже явных правил и исключений.

На независимой проверке из 23 137 отрицательных примеров, чьи сочетания клавиш
не встречались при обучении: 0 ложных замен и 96.7% нужных. По направлениям —
98.9% для английского в русской раскладке и 92.6% для русского в английской.
Ноль ложных замен не означает нулевого риска: верхняя граница 0.013%.
Это синтетическая проверка на разговорном корпусе, а не измерение на реальной
переписке. [Данные, результаты и границы](https://github.com/olegius88/keyswitch/blob/v0.20.0/model/ortho_v1/README.md).

Межраскладочные омографы неразрешимы принципиально: `св` и `cd`, `часу` и
`xfce` — одни и те же клавиши. Модель держит их ниже порога, то есть не портит
текст, но и не распознаёт. Токены короче трёх символов не оцениваются.
Веса intent, контекстной, префиксной моделей и модели границ не менялись.

Технический журнал включается явно и может содержать анализируемые слова.
Просматривайте его перед передачей. Приватные логи в выпуск не включены.
LogCourier этим выпуском не обновляется.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.20.0-x64.exe` или
  `KeySwitch-0.20.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.20.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. [Описание диагностики](https://github.com/olegius88/keyswitch/blob/v0.20.0/docs/troubleshooting.md).

## English

An orthotactic model: the decision follows the key sequence, not a dictionary.

- Dictionary evidence cannot help a token that neither language contains.
  `htop`, `webjs`, `.dist` and `pm2` typed in the wrong layout used to survive
  every layer untouched and had to be corrected by hand.
- The engine observes the physical key sequence, not the visible string. The
  us/ru map is a bijection, so one sequence has exactly two readings. The new
  counted model `ortho-v1-bdb915e4f06f` compares their likelihoods and answers
  whether the layout was wrong without ever asking whether the token is a word.
- A counted case channel closes the remaining escape route. The most
  English-looking genuine Russian sequences are abbreviations, and an
  abbreviation is written in capitals, so a lowercase token cannot claim to be
  one. An ALL-CAPS token never reaches the model: the code guard stops it first.
- The model only licenses a conversion where the base recogniser abstained. It
  never vetoes one, never overturns the contextual model's refusal, yields to
  contextual lookahead, and stays below explicit rules and exclusions.

On an independent holdout of 23,137 negatives whose key families were unseen
during counting: zero false conversions and 96.7% recall (98.9% for English
typed in the Russian layout, 92.6% the other way). Zero observed false
conversions is not zero risk: the upper bound is 0.013%. This is a synthetic
measurement on a conversational corpus, not a field study.

Cross-layout homographs remain unresolvable in principle: `св` and `cd`, `часу`
and `xfce` are the same keys. The model keeps them below its threshold, so it
does not corrupt them, but it cannot recognise them either. Tokens shorter than
three characters are not scored. Intent, context, prefix and boundary weights
are unchanged.

Technical logs can contain evaluated words: review them before sharing. Private
logs are not published. This release does not update LogCourier.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.20.0-x64.exe` or
  `KeySwitch-0.20.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.20.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

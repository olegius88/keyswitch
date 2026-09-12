# KeySwitch 0.21.0

## Русский

Печать модели намерения теперь привязана к тому, что инструментарий
**вычисляет**, а не к тому, как он себя называет. Плюс два отказа перед моделью
орфотактики. Веса орфотактики не менялись; модель намерения перепечатана.

### Обновление версий больше ничего не ломает само по себе

9 сентября `apt` пересобрал python3.14, не меняя версии языка. Ни одно число не
изменилось — веса, пороги и калибровки воспроизводились побайтно, — но в
`sys.version` сдвинулась дата сборки, а она входила в хеш кандидата. Выпуск
встал.

Семь строк, называвших машину сборки, вынесены из идентичности в
`model/intent_v1/build-environment.json`. В `manifest.toolchain` остались только
дайджесты кода и конфига. Гарантия не ослабла: веса и так входят в хеш
кандидата, поэтому интерпретатор, считающий иначе, по-прежнему отвергается — по
весам, а это честное основание.

Чтобы «считает иначе» можно было не только заметить, но и назвать, добавлен
`tools/environment_probe.py`. Он измеряет ровно те примитивы, от которых зависит
обучение: выражения FTRL как они написаны в трейнере, те самые функции libm,
порядок суммирования, текстовые преобразования float, **исчерпывающий** обход
всех 1 114 112 кодовых точек Unicode, цикл FNV, устойчивость сортировки,
целочисленную арифметику и поток Mersenne Twister. Зонд не участвует в
идентичности — его показания это провенанс, а его файл сертифицирован, чтобы
пробу нельзя было втихую ослабить. Сдвиг `sqrt` на один ULP двигает ровно
`float_arithmetic` и `libm`; подмена `NFC` — ровно `unicode`.

Словари Hunspell заморожены в `model/intent_v1/sources/hunspell/`. Оценка
выводит из них свои лексические выборки, поэтому обычное обновление пакета
валило выпуск, хотя обучение словарь не открывает вовсе. Замороженные байты
сегодня совпадают с системными, так что ни один дайджест корпуса не сдвинулся.

Запечатанный тест теперь расходуется по **ответу**, а не только по билету:
`seal-outcome-v21.json` фиксирует дайджест запечатанных секций безусловно —
до расчёта гейтов, до публикации и до того, как метрика попадёт в любой файл.
Повтор, давший другой ответ, останавливается, ничего не напечатав.

Пространство нарезки переведено на `keyswitch:intent-v21:physical-signature`:
инструментарий изменился, значит выборки перерезаны, и запечатанный тест —
тот, против которого этот кандидат ещё не оценивался.

### Два отказа перед моделью орфотактики

- **Слово, которое знает словарь текущей раскладки, эта модель больше не
  заменит — никогда.** `руку` — обычное русское слово, чьи клавиши пишут обычное
  английское `here`; никакая символьная модель их не различит, и попытка
  различать стоила порога, заглушавшего настоящие команды. Это гарантия по
  построению, а не по статистике: словесные модели знают о таком токене больше,
  и их отказ теперь окончателен.
- **Знак, застрявший между букв, — не слово ни в одной раскладке.** `и"ю` — это
  кавычка между двумя буквами, а не плохо написанный русский. Дефис и апостроф
  внутри слова — другое дело: движок присоединяет их к слову до всякой проверки
  на границу, поэтому `Я-то`, `из-за` и `don\'t` заменяются по-прежнему.

Оба отказа сужают то, о чём модель вообще спрашивают, и ни один не добавляет
замен. Разница в поведении небольшая и вся в сторону безопасности.

### Почему веса прежние

Для этой модели построен отдельный измерительный конвейер (`model/ortho_v2/`).
Он нашёл четыре дефекта в том, **как** модель измерялась: корпус расходился с
движком в том, какой токен вообще до модели доходит; ЙЦУКЕН ставит точку и
запятую на клавишу, которую американская раскладка пишет как `/`, отчего 16.9%
русских токенов читались обратно как слова, которых никто не набирал; словарный
отказ мерился по частотному списку, тогда как рантайм спрашивает ещё и Hunspell;
и две модели сравнивались на выборке, часть которой одна из них видела при
обучении.

Ни один кандидат, построенный на исправленном измерении, не превзошёл
установленную модель при честной калибровке. Веса не виноваты: подбор порядка и
сглаживания (25 конфигураций) даёт 99.69% против нынешних 99.65%, а при идеальном
пороге веса отделяют 99.7% нужных замен. Всё теряется на запасе над порогом, а
запас велик потому, что хвост отрицательных примеров — это мусор и текст, сам
набранный не в той раскладке.

Семь способов отделить этот класс проверены и отвергнуты, каждый с числами.
Последний из них важен: если не считать ошибкой замену там, где метке корпуса
нельзя верить, порог падает и картина выглядит отличной — а движок начинает
превращать `гифка` в `ubarf` и `Ютуб` в `Юne,`. Исключить метку не значит сделать
токен безопасным для замены.

### Известное и неисправленное

Попутно нашёлся дефект самой выпущенной модели: `флуд`, `лут` и `дюп`
преобразуются, хотя это обычные русские слова. Их нет ни в частотном списке, ни в
Hunspell, поэтому словарный отказ до них не достаёт, а поднять порог значит
потерять больше нужных замен, чем сберечь. Ручное преобразование горячей клавишей
это отменяет, а обучение запоминает отказ.

Технический журнал включается явно и может содержать анализируемые слова.
Просматривайте его перед передачей. Приватные логи в выпуск не включены.
LogCourier этим выпуском не обновляется.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.21.0-x64.exe` или
  `KeySwitch-0.21.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.21.0_amd64.deb`.
- Контрольные суммы: `SHA256SUMS`.

Установщик Windows пока не подписан сертификатом издателя. Нативный Wayland
не поддерживается. [Описание диагностики](https://github.com/olegius88/keyswitch/blob/v0.21.0/docs/troubleshooting.md).

## English

The layout-intent seal now binds to what the toolchain **computes** rather than
to what it calls itself, plus two refusals in front of the orthotactic model.
The orthotactic weights are unchanged; the intent model is re-sealed.

### A version bump no longer breaks anything on its own

On 9 September `apt` rebuilt python3.14 without changing the language version.
Not one number moved - weights, thresholds and calibration reproduced byte for
byte - but the build date inside `sys.version` did, and that string was part of
the candidate hash. The release stopped.

The seven strings naming the build machine have left the identity for
`model/intent_v1/build-environment.json`. `manifest.toolchain` now carries code
and config digests only. Nothing is given up: the candidate hash already covers
the weights, so an interpreter that computes differently is still refused - on
the weights, which is the honest ground.

So that "computes differently" can be named rather than merely noticed,
`tools/environment_probe.py` measures the primitives training actually depends
on: the FTRL update as the trainer writes it, exactly the libm functions it
calls, summation order, float text round-trips, an **exhaustive** walk of all
1,114,112 Unicode code points, the FNV mixing loop, sort stability, integer
arithmetic and the Mersenne Twister stream. The probe never votes on identity -
its readings are provenance and its file is certified so it cannot be weakened
unnoticed. A one-ULP change to `sqrt` moves exactly `float_arithmetic` and
`libm`; a changed `NFC` moves exactly `unicode`.

The Hunspell dictionaries are frozen into `model/intent_v1/sources/hunspell/`.
The evaluation derives its lexical populations from them, so an ordinary
package update failed the release although training never opens a dictionary at
all. The frozen bytes are identical to the system ones today, so no corpus
digest moved.

The sealed test is now consumed by its **answer** rather than only by its
ticket: `seal-outcome-v21.json` records the digest of the sealed sections
unconditionally - before the gates are computed, before publication, and before
a metric reaches any file. A rerun that computes a different answer stops
without printing one.

The split namespace is rotated to `keyswitch:intent-v21:physical-signature`:
the certified toolchain changed, so the splits are re-cut and the sealed test is
one this candidate has never been evaluated against.

### Two refusals in front of the orthotactic model

- **A word the dictionary of the current layout knows is never converted by this
  model again.** `руку` is an ordinary Russian word whose keys spell the ordinary
  English word `here`; no character model can separate them, and asking it to try
  cost a threshold high enough to silence real commands. This is structural
  rather than statistical: the word models know more about such a token, and
  their refusal is now final.
- **Punctuation caught between letters is not a word in either layout.** `и"ю` is
  a quotation mark between two letters, not Russian written badly. A hyphen or an
  apostrophe inside a token is a different matter — the engine joins those to the
  word before it tests for a boundary — so `Я-то`, `из-за` and `don\'t` keep
  their conversions.

Both refusals narrow what the model is asked about; neither adds a conversion.
The behavioural difference is small and entirely on the safe side.

### Why the weights are unchanged

A separate measurement pipeline was built for this model (`model/ortho_v2/`). It
found four defects in **how** the model was being measured: the corpus disagreed
with the engine about which token reaches a model at all; the Russian layout puts
the full stop and the comma on the key the US layout writes as `/`, so 16.9% of
Russian tokens were read back as words nobody typed; the dictionary refusal was
measured against the frequency list while the runtime also asks Hunspell; and the
two models were compared on a population one of them had trained on.

No candidate built on the corrected measurement beat the installed model under
honest calibration. The weights are not at fault: sweeping order and smoothing
(25 configurations) gives 99.69% against the current 99.65%, and at an ideal
threshold the weights separate 99.7% of the wanted conversions. Everything is
lost to the safety margin, and the margin is large because the negative tail is
garbage and text that was itself typed in the wrong layout.

Seven ways of separating that class were tried and rejected, each with its
numbers. The last one matters: declining to count a conversion as an error where
the corpus label cannot be trusted drops the threshold and makes everything look
excellent — and makes the engine turn `гифка` into `ubarf` and `Ютуб` into
`Юne,`. Excluding a label does not make the token safe to convert.

### Known and unfixed

Measuring the shipped model turned up a defect in it: `флуд`, `лут` and `дюп` are
converted although they are ordinary Russian. They are in neither the frequency
list nor Hunspell, so the dictionary refusal does not reach them, and raising the
threshold to cover them costs more conversions than it saves. The manual hotkey
undoes such a conversion and learning remembers the refusal.

Technical logs can contain evaluated words: review them before sharing. Private
logs are not published. This release does not update LogCourier.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.21.0-x64.exe` or
  `KeySwitch-0.21.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.21.0_amd64.deb`.
- Checksums: `SHA256SUMS`.

The Windows installer is not yet publisher-signed. Native Wayland is unsupported.

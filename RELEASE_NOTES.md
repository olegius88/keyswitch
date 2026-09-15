# KeySwitch 0.22.0

## Русский

Первый опубликованный выпуск после
[KeySwitch 0.20.0](https://github.com/olegius88/keyswitch/releases/tag/v0.20.0).
Тег 0.21.0 существует, но его сборка не была опубликована, поэтому все
изменения 0.21.0 входят в этот выпуск. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.22.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.22.0-x64.exe` — установщик для Windows 10/11 x64. Он не
  подписан сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.22.0-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Установленные модели

| Слой | Версия |
| --- | --- |
| Намерение раскладки (intent) | `intent-v1-b2a2ec8caa8d` (запечатанный тест v23) |
| Контекст завершённого слова | `context-v1-a24683995ca4` |
| Ранний префикс | `prefix-v1-2f0c54bf546b` |
| Граница слова | `boundary-v2-3b2b1af6693e` |
| Орфотактика | `ortho-v1-bdb915e4f06f` |

Контекстная модель переобучена на тех же авторских сценариях относительно
intent-модели v23: 36 618 синтетических строк, 16 539 из 17 658 желаемых замен
восстановлены, 0 ложных замен (детектор без неё восстанавливает 13 905). Это
синтетические сценарии, а не измерение качества на реальном вводе. Веса
префикса, границы и орфотактики не изменились; их замороженные свидетельства
заново проверены на движке этого выпуска: граница — 18 из 18
последовательностей точно, ни одно правильное слово не изменено; префикс —
128 из 128 желаемых восстановлений без расхождений длины.

### Известные дефекты

Эти случаи воспроизведены целыми последовательностями клавиш на моделях
именно этого выпуска и зафиксированы в тестах как ожидаемые падения
(`tests/disclosed_regressions.py`). Кандидаты новых моделей, исправляющие
часть из них, дважды не прошли независимый запечатанный тест и не
установлены.

- Раннее переключение может преждевременно перевести правильное русское
  слово, если его начало совпадает с началом английского слова в другой
  раскладке. Подтверждённый пример: `гифку` в комментарии редактора кода
  превращается в `ubare`, и проверка завершённого слова уже не возвращает
  исходный текст.
- Короткие разговорные русские слова, набранные отдельно (`гифку`, `флуд`,
  `лут`, `дюп`, `ютуб`, `зум`), могут быть заменены по паузе или по границе
  слова.
- Обход: выключить раннее переключение (`detection.early_switch`) или
  исправление по паузе (`detection.correct_on_pause`), либо добавить слово в
  исключения (`exclusions.words`). Клавиша отмены (по умолчанию `Ctrl+Alt+Z`)
  возвращает исходный текст.

Нулевые ошибки на выбранных проверках не доказывают отсутствие ошибок при
любом возможном вводе.

### Проверка выпуска

Идентичность intent-модели больше не включает строку с датой сборки Python;
сведения об окружении сохраняются отдельно в
[build-environment.json](model/intent_v1/build-environment.json), а код,
конфигурация и байты модели по-прежнему проверяются по SHA256. Hunspell-словари
оценки заморожены в репозитории. Проверка strict-отчёта отклоняет отсутствие
обязательного условия, даже если все остальные условия успешны. Вспомогательные
модели больше не переобучаются на CI: проверяются неизменность их лексических
входов, замороженные байты и повтор на текущем движке. См.
[методику проверки](docs/verification.md).

Технический журнал может содержать анализируемые слова. Просматривайте его
перед передачей. Приватные логи и переписка не входят в состав выпуска.

## English

The first published release after
[KeySwitch 0.20.0](https://github.com/olegius88/keyswitch/releases/tag/v0.20.0).
The 0.21.0 tag exists, but its build was never published, so every 0.21.0
change is part of this release. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.22.0_amd64.deb` — Ubuntu/Xubuntu, X11 session.
- `KeySwitch-Setup-0.22.0-x64.exe` — installer for Windows 10/11 x64. It is not
  signed with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.22.0-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Installed models

| Layer | Version |
| --- | --- |
| Layout intent | `intent-v1-b2a2ec8caa8d` (sealed test v23) |
| Completed-word context | `context-v1-a24683995ca4` |
| Early prefix | `prefix-v1-2f0c54bf546b` |
| Word boundary | `boundary-v2-3b2b1af6693e` |
| Orthotactics | `ortho-v1-bdb915e4f06f` |

The context model was re-fitted on the same author-written scenarios against
the v23 intent model: 36,618 synthetic rows, 16,539 of 17,658 desired
conversions restored, 0 false conversions (the detector alone restores 13,905).
These are synthetic scenarios, not a measurement of quality on real input. The
prefix, boundary and orthotactic weights are unchanged; their frozen evidence
was re-verified on this release's engine: boundary 18 of 18 sequences exact
with no correct word changed, prefix 128 of 128 desired restorations with no
length mismatch.

### Known defects

These cases are reproduced as whole keystroke sequences on the models of this
very release and recorded in the test suite as expected failures
(`tests/disclosed_regressions.py`). Candidate models that fix some of them
failed the independent sealed test twice and are not installed.

- The early switch can convert a correctly typed Russian word too early when
  its beginning coincides with the beginning of an English word in the other
  layout. Confirmed example: `гифку` in a code-editor comment becomes `ubare`,
  and the completed-word check no longer sees the original text.
- Short colloquial Russian words typed on their own (`гифку`, `флуд`, `лут`,
  `дюп`, `ютуб`, `зум`) can be replaced on pause or at a word boundary.
- Workaround: disable the early switch (`detection.early_switch`) or the
  pause correction (`detection.correct_on_pause`), or add the word to the
  exclusions (`exclusions.words`). The undo key (`Ctrl+Alt+Z` by default)
  restores the original text.

Zero observed errors on selected checks would not prove correctness for every
possible input.

### Release verification

The intent-model identity no longer includes the Python build-date string;
environment details are recorded separately in
[build-environment.json](model/intent_v1/build-environment.json), while code,
configuration and model bytes remain bound by SHA256. The Hunspell
dictionaries used by the evaluation are frozen in the repository. Strict-report
validation rejects a missing required gate even when every remaining gate
passes. The auxiliary models are no longer retrained on CI: the checks attest
their unchanged lexical inputs, their frozen bytes and a replay on the current
engine. See the [verification procedure](docs/verification.md) (in Russian).

Technical logs may contain evaluated words. Review them before sharing.
Private logs and conversations are excluded from the release.

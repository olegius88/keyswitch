# KeySwitch 0.24.0

## Русский

Выпуск меняет поведение движка там, где вы об этом просили: кавычка в Telegram, одиночные
буквы в начале сообщения, расширенное распознавание. Модели те же, что в 0.23.1. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `keyswitch_0.24.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-Setup-0.24.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.24.0-windows-x64.zip` — переносимый архив для Windows.
- `SHA256SUMS` — контрольные суммы трёх файлов; сверьте их перед установкой.

### Модели те же, кандидат остаётся кандидатом

Пара context-v1 + prefix-v1 остаётся установленной. Новая пара моделей прошла независимую
запечатанную оценку на тексте трибанков (ни одной испорченной строки против 7–28 у текущей
пары), но на второй оценке — предложения Tatoeba и индекс пакетов Ubuntu — восстановила на
две-три строки из 247 меньше текущей пары. Порог принятия не снижается, поэтому выпуск
выходит на проверенной паре; путь установки новой пары в движке и воротах уже готов.
Раскрытый случай `дюп` по-прежнему переводится.

### Telegram: кавычка сразу становится @

Кавычка, набранная в русской раскладке в начале слова, заменяется на «@» на самом
нажатии, и ник печатается следом без паузы. Нужна настоящая кавычка — нажмите ту же
клавишу ещё раз: получится «""» и раскладка вернётся. Правило живёт в новом разделе
«Особенности программ», где каждое правило — отдельный переключатель.

### Одиночная буква в начале сообщения

`z`, `b`, `f` в английской раскладке без слова перед ними становятся `я`, `и`, `а`: в
русском тексте так начинается каждое седьмое предложение, в английском одиночная буква в
начале предложения не встречается. После английского слова буква остаётся как есть. При
быстром наборе без паузы букву решает следующее слово.

### Расширенное распознавание включено

Слово больше не может превратиться в строку с пунктуацией (`рукх` не станет `her[`):
шесть русских букв живут на клавишах пунктуации, и это проверяется отдельно. После
этого исправления расширенное распознавание незнакомых слов включено по умолчанию — на
измерении оно больше ничего не портит. Описания настроек «Расширенное распознавание» и
«Ранняя смена раскладки» переписаны простыми словами; ранняя смена остаётся выключенной
по цифрам независимой оценки.

### Известные ограничения

- Изолированное слово, неизвестное словарю (`дюп`), всё ещё может быть переведено.
- Числа выше измерены на запечатанных тестовых корпусах из 311 и 282 строк естественного и
  технического текста; они не являются оценкой на произвольном вводе.

Технический журнал может содержать анализируемые слова. Просматривайте его перед
передачей. Приватные логи и переписка не входят в состав выпуска.

## English

This release changes the engine where you asked for it: the Telegram quote, lone letters
at the start of a message, the wider recognition. The models are the same as in 0.23.1. The full list is in
[CHANGELOG.md](CHANGELOG.md).

### Release files

- `keyswitch_0.24.0_amd64.deb` — Ubuntu/Xubuntu, X11 session.
- `KeySwitch-Setup-0.24.0-x64.exe` — installer for Windows 10/11 x64. It is not signed
  with a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.24.0-windows-x64.zip` — portable archive for Windows.
- `SHA256SUMS` — checksums of the three files; verify them before installing.

### Same models, the candidate stays a candidate

The context-v1 + prefix-v1 pair stays installed. The new pair passed an independent sealed
evaluation on treebank text (no corrupted rows against 7-28 for the current pair) but, on a
second one built from Tatoeba sentences and the Ubuntu package index, restored two to three
rows out of 247 fewer than the current pair. The acceptance bar is not lowered, so this
release ships on the proven pair; the path for installing a new pair in the engine and the
gates is ready. The disclosed `дюп` case is still converted.

### Telegram: the quote becomes @ at once

A quote typed in the Russian layout at the start of a word becomes `@` on the keystroke,
and the nickname follows without a pause. For a real quote press the same key again: you
get `""` and the layout returns. The rule lives in the new "Application quirks" section,
one switch per rule.

### A lone letter at the start of a message

`z`, `b`, `f` typed in the English layout with no word before them become `я`, `и`, `а`: one
Russian sentence in seven opens with such a word, no English sentence opens with a lone
letter. After an English word the letter stays. When typing continues without a pause, the
next word decides the letter.

### The wider recognition is on

A word can no longer turn into a string with punctuation (`рукх` will not become `her[`):
six Russian letters sit on punctuation keys, and that is now checked on its own. With that
fixed, the wider recognition of unknown words is on by default; on the measurement it no
longer changes anything it should not. The descriptions of the two recognition settings are
rewritten in plain words; the early switch stays off by the numbers of the independent
evaluation.

### Known limitations

- An isolated word unknown to the lexicon (`дюп`) can still be converted.
- The numbers above come from sealed test corpora of 311 and 282 rows of natural and
  technical text; they are not a measurement on arbitrary input.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.

# KeySwitch 0.35.0

## Русский

Смешанный русско-английский текст и забытое переключение раскладки исправляются заметно лучше: новая
контекстная модель училась на вопросах, которые программа задаёт ей при настоящем наборе, а знак после
исправленного слова теперь переводится вместе с ним. Полный перечень — в [CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.35.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.35.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.35.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11. Если стоит 0.28.0 или
  новее, эта версия придёт через обычное обновление системы.
- `KeySwitch-0.35.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.35.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Смешанный текст и забытое переключение

Прежде контекстная модель училась на написанных для проекта сценариях, а проверялась на предложениях,
набранных целиком в одной раскладке. Настоящий набор другой: русский текст с английскими терминами,
раскладку переключают на границе языка и иногда забывают. Теперь публичный текст — сообщения русского
Stack Overflow и предложения Tatoeba — набирается через программу так, как это делает человек, и
вопросы, которые программа при этом задаёт модели, с правильными ответами входят в обучение. Модель
видит и новые сведения: где слово стоит в строке, его регистр, точку или подчёркивание в латинском
прочтении, есть ли в токене буквы и то, как часто каждое прочтение встречается — внутри русского текста и в
тексте своего языка. Поэтому английская строка, набранная в русской раскладке целиком, больше не оставляет
первое слово («гр why»): `uh` в русском тексте не встречается, но это английское слово.

Чтобы модель не переводила имена и сленг, к данным добавлен правильно набранный русский текст UD
Russian-Taiga. На комментариях Stack Overflow, которые модель не видела (1 973 смешанных и 495 русских):
верно набранный смешанный текст портится 12 раз вместо 49; забытое переключение обратно на русский
исправляется в 95,3 % сообщений вместо 84,0 %, забытое переключение на английский — в 81,2 % вместо
52,3 %, сообщение, начатое не в той раскладке, — в 91,4 % вместо 80,8 %; верно набранные русские
сообщения по-прежнему не портятся. На больших выборках верно набранного русского текста (4 990
предложений Taiga, 1 976 сообщений Stack Overflow, 2 000 предложений Tatoeba) испорчено 1, 1 и 0 вместо
4, 2 и 0. На 5 000 предложениях субтитров десяти раскрытых наборов исправленных 24 842 вместо 24 805 из
25 000, испорченных правильных 35 вместо 55; буквы, вписанные внутрь слова, исправляются в 24 951 случае
вместо 24 920. Независимая проверка на свежих данных — 5 000 новых предложений субтитров, 9 884 сообщения
Stack Overflow с опечатками и без, 9 999 предложений Tatoeba — пройдена по всем 20 строкам: верно
набранный смешанный текст испорчен 42 раза вместо 175, забытое переключение на английский исправлено в
6 121 сообщении из 7 418 вместо 3 844, английские предложения в русской раскладке — 7 947 из 8 000 вместо
7 829, верно набранные русские сообщения и предложения не испорчены ни разу.

### Знаки после слова

Кто набирает русское слово в английской раскладке, набирает и знак после него клавишей русской
раскладки: запятая там — Shift+/, то есть `?`. Поэтому `ghbdtn?` теперь становится «привет,», а
`ltkf&` — «дела?»; раньше знак оставался как набран («привет?»). Адрес `support@mail.ru`, набранный в
русской раскладке, получает обратно `@`, а кавычка, которая открыла слово, на его конце остаётся
кавычкой: `"john"`. В обратную сторону: `hello,` в русской раскладке выходит `руддщб` (запятая — клавиша
«б»); такой хвост теперь оценивается отдельно и исправляется вместе со словом, если до него остаётся
хотя бы две буквы, а `хлеб` и `чё` остаются словами.

### Что стало хуже

Двухбуквенные русские слова, чьё латинское прочтение — английское слово, набранные в английской
раскладке после русского текста, пропускаются чаще: 38 раз вместо 32 на пробе из 52 слов (зато в пустом
поле и перед русским текстом — реже: 40 вместо 45 и 19 вместо 32). Редкие сочетания с частым английским
прочтением («шт», «уч», «фе»), верно набранные в начале поля перед русским текстом, заменяются 7 раз вместо
одного. `jr.` в начале сообщения теперь сразу становится «ок.».

### Данные обучения

Вопросы, на которых училась модель, лежат в `model/context_v1/captured/` вместе с рецептами их
повторного получения, списком вопросов Stack Overflow, авторов предложений Tatoeba и предложений Taiga:
это фрагменты текстов ru.stackoverflow.com (CC BY-SA), предложений Tatoeba (CC BY 2.0 FR) и UD
Russian-Taiga (CC BY-SA 4.0), распространяются по CC BY-SA 4.0. В пакет программы входит только таблица частот слов, выведенная из тех же текстов.

## English

Mixed Russian and English text and a forgotten layout switch are corrected much more often: the new
context model learned from the questions the program asks it during real typing, and a sign typed
after a corrected word is now converted with it. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.35.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.35.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.35.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session. With 0.28.0 or newer
  installed, this version arrives through the ordinary system update.
- `KeySwitch-0.35.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.35.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Mixed text and a forgotten switch

The context model used to learn from scenarios written for the project and was checked on sentences
typed wholly in one layout. Real typing is different: Russian text with English terms, the layout
switched at each change of language and sometimes forgotten. Now public text - Russian Stack
Overflow messages and Tatoeba sentences - is typed through the program the way a person does it, and
the questions the program asks the model on the way, with the right answers, become training data.
The model also sees where a word stands on its line, its case, a dot or underscore inside the Latin
reading, whether the token has letters at all, and how often each reading occurs - inside Russian text
and in text of its own language. So an English line typed wholly in the Russian layout no longer keeps
its first word ("гр why"): `uh` never occurs in Russian text, yet it is an English word.

So that names and slang stay, correctly typed Russian text of UD Russian-Taiga joins the data. On
Stack Overflow comments the model has not seen (1,973 mixed and 495 Russian ones): correctly typed
mixed text is changed by mistake 12 times instead of 49; a forgotten switch back to Russian is
corrected in 95.3% of messages instead of 84.0%, a forgotten switch to English in 81.2% instead of
52.3%, a message started in the wrong layout in 91.4% instead of 80.8%; correctly typed Russian
messages are still never changed. In larger samples of correctly typed Russian text (4,990 Taiga
sentences, 1,976 Stack Overflow messages, 2,000 Tatoeba sentences) 1, 1 and 0 are spoiled instead of 4,
2 and 0. On 5,000 subtitle sentences of ten disclosed sets, 24,842 instead of 24,805 of 25,000 are
restored, and 35 correct ones are spoiled instead of 55; letters typed into a word are restored in
24,951 cases instead of 24,920. An independent check on fresh data - 5,000 new subtitle sentences,
9,884 Stack Overflow messages with and without typos, 9,999 Tatoeba sentences - passed on all 20 rows:
correctly typed mixed text is changed by mistake 42 times instead of 175, a forgotten switch to English
is corrected in 6,121 of 7,418 messages instead of 3,844, English sentences typed in the Russian layout
in 7,947 of 8,000 instead of 7,829, and correctly typed Russian messages and sentences are never changed.

### Signs after a word

Typing a Russian word in the English layout, people type the sign after it with the key of the
Russian layout too: the comma there is Shift+/, that is `?`. So `ghbdtn?` now becomes "привет," and
`ltkf&` becomes "дела?"; the sign used to stay as typed ("привет?"). The address `support@mail.ru`
typed in the Russian layout gets its `@` back, while a quote that opened the word stays a quote at
its end: `"john"`. The other way round, `hello,` in the Russian layout comes out as `руддщб` (the comma
is the key of "б"); such an ending is now judged apart and converted with the word when at least two
letters are left before it, while "хлеб" and "чё" stay words.

### What got worse

Two-letter Russian words whose Latin reading is an English word, typed in the English layout after
Russian text, are missed more often: 38 times instead of 32 in a probe of 52 words (though less often
alone in an empty field and before Russian text: 40 instead of 45 and 19 instead of 32). Rare pairs
whose Latin reading is a common English word ("шт", "уч", "фе"), typed as intended at the start of a
field before Russian text, are changed 7 times instead of once. `jr.` at the start of a message now
becomes "ок." at once.

### Training data

The questions the model learned from are in `model/context_v1/captured/`, with the recipes that
re-create them, the list of Stack Overflow questions, the authors of the Tatoeba sentences and the
Taiga sentences: they are fragments of ru.stackoverflow.com posts (CC BY-SA), of Tatoeba sentences
(CC BY 2.0 FR) and of UD Russian-Taiga (CC BY-SA 4.0), distributed under CC BY-SA 4.0. The program package contains only a table of word frequencies derived
from the same texts.

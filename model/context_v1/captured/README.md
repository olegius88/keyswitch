# Вопросы движка на набранном публичном тексте

Контекстная модель context-v1 с 0.35.0 учится не только на сценариях проекта
([`../scenarios.json`](../scenarios.json)), но и на вопросах, которые движок KeySwitch сам задаёт ей,
когда через него набирают публичный текст так, как это делает человек: каждый токен — клавишами
задуманной раскладки, на границе языка раскладка выбирается вручную, иногда переключение забыто или
сообщение начато не в той раскладке, иногда в слове опечатка, иногда буквы вставляют внутрь слова после
щелчка. Правильный ответ известен из замысла: `convert`, если спрошенный токен набран не в той
раскладке, иначе `keep`. Английские предложения Tatoeba набираются и целиком в русской раскладке — в
пустом поле, под строкой кода или сообщения и под русским предложением: так модель видит английскую строку,
у которой переключение забыто с первого слова. Правильно набранный русский текст UD Russian-Taiga (соцсети, художественные
тексты, новости — с именами, сленгом и редкими словами) даёт только ответы `keep`: в обучающих текстах
Stack Overflow незнакомое кириллическое слово чаще всего оказывалось английским термином в чужой
раскладке, и без Taiga модель переводила имена и сленг. Эти ответы не входят в баланс действий
(`"balance": false` в манифесте): иначе они снижали бы вес каждого `keep` и сдвигали всю модель к замене —
английских слов и технических токенов в том числе. Как это устроено и что это дало — в
[описании контекстного помощника](../../../docs/context-assistant.md), раздел «Смешанный текст и
вопросы движка».

## Файлы

`manifest.json` перечисляет тринадцать файлов: SHA-256 каждого, число вопросов (`rows`) и разных вопросов
(`unique`), вес в обучении, источник, лицензию и рецепт — какие сообщения, сколько, с каким зерном и в
каких режимах набраны. Каждая строка файла `*.jsonl.xz` — массив JSON в порядке `columns` манифеста:
сначала сколько раз вопрос был задан, затем сведения, которые движок передаёт модели (`ContextEvidence`:
токен, его прочтение в другой раскладке, словарные флаги, текст поля до и после слова), в конце —
правильный ответ. Имени приложения в вопросах нет: модель его не читает.

`tools/train_context_model.py` проверяет SHA-256 каждого файла по манифесту, а отчёт модели
([`../report.json`](../report.json), поле `captured_sha256`) закрепляет сам манифест.

Чьи тексты в файлах:

- `so-questions.txt` — вопросы ru.stackoverflow.com, чьи сообщения или строки кода есть в файлах; страница `https://ru.stackoverflow.com/questions/<id>` называет авторов вопроса, ответов и
  комментариев;
- `tatoeba-authors.tsv.xz` — предложения Tatoeba с авторами: id предложения и имя автора (`\N` — у
  предложения нет владельца); страница предложения — `https://tatoeba.org/sentences/show/<id>`;
- `taiga-sentences.txt` — предложения UD Russian-Taiga по `sent_id`.

## Как пересоздать

Источники (суммы — те, что проверены при загрузке 29.09.2026):

- Hugging Face `IlyaGusev/ru_stackoverflow`, коммит `3bfd79e598eaab49b8b5ec7397d5c12b0f22416a`, файл
  `ru_stackoverflow.jsonl.zst`, SHA-256
  `e3f6eda2b3df7de1d3cc4a45ce2e69bb1e439b7736cd8d81e911129bd512dab9`;
- Tatoeba, `https://downloads.tatoeba.org/exports/per_language/{rus,eng}/{rus,eng}_sentences.tsv.bz2`
  (выгрузка от 26.09.2026): `rus_sentences.tsv.bz2` SHA-256
  `8c80c8e8337da106a8f3a1e1c96a6a100a6ab5429f01e8735c978c0697d6f75f`, `eng_sentences.tsv.bz2` SHA-256
  `f0568ae34496f8a03758ce6821b68da04b1fe67a9300de8d26d50494ae5aceab`; для авторов — там же
  `{rus,eng}_sentences_detailed.tsv.bz2` той же выгрузки: SHA-256
  `4270bbb5e918693bb8a2d96f0b7bcf62b5b3847e4a85d66362ff3e6213adc5d5` и
  `353d48de7905952cf6f1500f6a3158516ecf9e10cd844ba051982cfa4a11c111`. Tatoeba выгрузки обновляет;
  суммы другой выгрузки будут другими, и файлы не совпадут;
- UD Russian-Taiga, <https://github.com/UniversalDependencies/UD_Russian-Taiga>, тег r2.18, коммит
  `d1e48cd29c1f19b6aad1aec361d9f724db764f66`, файлы `ru_taiga-ud-*.conllu`.

```bash
# Сообщения обоих источников и строки кода, по частям train/dev/test (нити вопросов, id предложений).
KEYSWITCH_MODEL_PATH=model/intent_v1/sources python3 tools/mixed_typing.py messages \
    --so ru_stackoverflow.jsonl.zst --tatoeba <каталог с rus_/eng_sentences.tsv.bz2> \
    --taiga <каталог с ru_taiga-ud-*.conllu> --out messages
# Вопросы задавала контекстная модель 0.33.3 (та же в 0.34.0).
git show v0.34.0:src/keyswitch/resources/models/context_policy_v1.json > context-0.33.3.json
KEYSWITCH_MODEL_PATH=model/intent_v1/sources python3 tools/mixed_typing.py capture \
    --manifest model/context_v1/captured/manifest.json --messages messages --model context-0.33.3.json --verify
# Таблица частот, которую читает схема признаков 7.
KEYSWITCH_MODEL_PATH=model/intent_v1/sources python3 tools/mixed_typing.py terms \
    --messages messages --out context-term-frequency.json
# Списки вопросов и авторов рядом с манифестом.
python3 tools/mixed_typing.py sources --manifest model/context_v1/captured/manifest.json \
    --messages messages --tatoeba <каталог с rus_/eng_sentences_detailed.tsv.bz2>
```

С `--verify` сбор сравнивает содержимое каждого файла с сохранённым, без него — перезаписывает файлы.
Набираются только обучающие части; части dev и test служат проверке (`tools/mixed_typing.py evaluate`).

## Лицензии

- Тексты участников ru.stackoverflow.com (сообщения из вопросов, ответов и комментариев и строки кода —
  в `so-*.jsonl.xz`, а строки кода над предложениями — и в `tatoeba-*`, `taiga-*.jsonl.xz`) Stack Exchange
  распространяет по CC BY-SA 2.5, 3.0 или 4.0 в зависимости от даты публикации
  (<https://stackoverflow.com/help/licensing>); набор `IlyaGusev/ru_stackoverflow` указывает CC BY-SA 2.5
  и хранит авторов текстов, где они известны.
- Предложения Tatoeba (<https://tatoeba.org>) — CC BY 2.0 FR
  (<https://creativecommons.org/licenses/by/2.0/fr/>), часть из них — также CC0 1.0; авторы — в
  `tatoeba-authors.tsv.xz`.
- UD Russian-Taiga — CC BY-SA 4.0 (`LICENSE.txt` репозитория); источники предложений описаны в его
  README, предложения — в `taiga-sentences.txt`.
- Файлы каталога — переработка этих текстов: тексты разбиты на сообщения и набраны через движок, в файлах
  — текст поля вокруг слова, о котором задан вопрос (не больше 512 знаков до него и 128 после — при
  правке внутри слова). Они распространяются по CC BY-SA 4.0
  (<https://creativecommons.org/licenses/by-sa/4.0/>); предложения Tatoeba в них остаются под CC BY 2.0 FR.
- Таблица частот `src/keyswitch/resources/models/context-term-frequency.json` — счёт слов тех же
  обучающих частей: латинских внутри сообщений Stack Overflow, кириллических вне словаря внутри русского
  текста Stack Overflow и Tatoeba, слов английских предложений Tatoeba и всех кириллических слов русского
  текста Stack Overflow, Tatoeba и Taiga (слово попадает в таблицу, если встретилось не меньше пяти раз);
  в пакет программы она входит, сами тексты — нет.

Прочтение лицензий юристом не проверено.

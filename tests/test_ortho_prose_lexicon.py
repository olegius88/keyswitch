"""The public-domain prose forms the orthotactic source channel counts (tools/ortho_prose_lexicon.py)."""
from __future__ import annotations

import bz2
import gzip
import hashlib
import io
import json
import runpy
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import ortho_prose_lexicon as prose  # noqa: E402

from keyswitch.constants.ortho import (  # noqa: E402
    MEDIAWIKI_ARTICLE_NAMESPACE,
    MEDIAWIKI_CATEGORY_NAMESPACE,
    ORTHO_PROSE_CATEGORY_DEPTH,
    ORTHO_PROSE_LAST_DEATH_YEAR,
    ORTHO_PROSE_LEXICON_SCHEMA_VERSION,
    ORTHO_PROSE_WORDS_SCALE,
    ORTHO_V1_MAXIMUM_LEXICON_WEIGHT,
    WIKISOURCE_AUTHOR_NAMESPACE,
)

AUTHOR = "Иван_Тестов"
LATE = "Пётр_Поздний"
FOREIGN = "Джон_Переводной"
EXCLUDED = prose.EXCLUDED_AUTHORS[0]
KEPT_TEXT = ("{{Отексте\n| АВТОР = Иван Тестов\n| НАЗВАНИЕ = Рассказ\n}}\n"
             "Куафер пришёл. Старый куафер сказал: «гыр-гыр, ну-ну».\nОн — куафер! Тут Москва и кот.")
DEEP_TEXT = "{{Отексте\n| АВТОР = [[Автор:Иван Тестов|Иван Тестов]]\n}}\nПовесть о том, как куафер уехал."
# The words of the two kept pages that carry a letter, and the forms they give with their counts.
KEPT_WORDS = ["Куафер", "пришёл", "Старый", "куафер", "сказал", "гыр-гыр", "ну-ну", "Он", "куафер", "Тут", "Москва", "и",
              "кот", "Повесть", "о", "том", "как", "куафер", "уехал"]
KEPT_FORMS = {form: KEPT_WORDS.count(form) for form in ("куафер", "пришёл", "сказал", "гыр-гыр", "ну-ну", "уехал")}


def weight(count: int) -> int:
    return prose.prose_weight(count, len(KEPT_WORDS))


class Dump:
    """A tiny dump of the four files the tool reads, with an author of each kind and a page of each kind."""

    def __init__(self, directory: Path) -> None:
        self.directory = directory
        self.pages: list[tuple[int, str, bool]] = []
        self.targets: dict[tuple[int, str], int] = {}
        self.links: list[tuple[int, str, int]] = []
        self.texts: dict[int, str] = {}

    def page(self, namespace: int, title: str, redirect: bool = False, text: str | None = None) -> int:
        page = len(self.pages) + 1
        self.pages.append((namespace, title, redirect))
        if text is not None:
            self.texts[page] = text
        return page

    def target(self, title: str, namespace: int = MEDIAWIKI_CATEGORY_NAMESPACE) -> int:
        return self.targets.setdefault((namespace, title), len(self.targets) + 1)

    def link(self, page: int, category: str, kind: str = "page", namespace: int = MEDIAWIKI_CATEGORY_NAMESPACE) -> None:
        self.links.append((page, kind, self.target(category, namespace)))

    def build(self) -> None:
        for title, died, russian in ((AUTHOR, ORTHO_PROSE_LAST_DEATH_YEAR, True), (LATE, ORTHO_PROSE_LAST_DEATH_YEAR + 1, True),
                                     (EXCLUDED, ORTHO_PROSE_LAST_DEATH_YEAR, True), (FOREIGN, ORTHO_PROSE_LAST_DEATH_YEAR, False)):
            author = self.page(WIKISOURCE_AUTHOR_NAMESPACE, title)
            self.link(author, f"Умершие_в_{died}_году")
            if russian:
                self.link(author, prose.RUSSIAN_WRITERS)
            prose_category = self.page(MEDIAWIKI_CATEGORY_NAMESPACE, f"Проза_{title}")
            self.link(prose_category, prose.PROSE_CATEGORY, "subcat")
            self.link(prose_category, title, "subcat")
            self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, f"Рассказ_({title})", text=KEPT_TEXT), f"Проза_{title}")
        redirect = self.page(WIKISOURCE_AUTHOR_NAMESPACE, f"{AUTHOR}_старый", redirect=True)
        self.link(redirect, "Умершие_в_1800_году")
        self.link(redirect, prose.RUSSIAN_WRITERS)
        own = f"Проза_{AUTHOR}"
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, f"Рассказ_({AUTHOR}){prose.PRE_REFORM_PAGE}", text=KEPT_TEXT), own)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Перевод", text="{{Отексте\n| ПЕРЕВОДЧИК = Кто-то\n}}\nтекст"), own)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Старый", text="{{Отексте\n}}\nонъ пришелъ въ домѣ"), own)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Чужой", text="{{Отексте\n| АВТОР = Другой Автор\n}}\nтекст"), own)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Перенаправление", redirect=True, text="#перенаправление"), own)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Шаблонная ссылка", text=KEPT_TEXT), own, namespace=WIKISOURCE_AUTHOR_NAMESPACE)
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Файл в категории", text=KEPT_TEXT), own, kind="file")
        parent = own
        for depth in range(ORTHO_PROSE_CATEGORY_DEPTH + 1):
            child = f"Глубина_{depth}"
            self.link(self.page(MEDIAWIKI_CATEGORY_NAMESPACE, child), parent, "subcat")
            parent = child
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Повесть_(Тестов)", text=DEEP_TEXT), "Глубина_0")
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Слишком_глубоко", text=KEPT_TEXT), parent)
        self.link(self.page(MEDIAWIKI_CATEGORY_NAMESPACE, "Глубина_0"), own, "subcat")        # met twice, followed once
        self.link(self.page(MEDIAWIKI_ARTICLE_NAMESPACE, "Статья в чужом пространстве"), "Проза_по_авторам", "subcat")
        self.write()

    def write(self) -> None:
        def table(name: str, columns: list[str], rows: list[str]) -> bytes:
            declared = ",\n".join(f"  `{column}` int(8) NOT NULL" for column in columns)
            return (f"CREATE TABLE `{name}` (\n{declared},\n  PRIMARY KEY (`{columns[0]}`)\n) ENGINE=InnoDB;\n"
                    f"INSERT INTO `{name}` VALUES {','.join(rows)};\n").encode()

        def quoted(value: str) -> str:
            return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"

        pages = [f"({page},{namespace},{quoted(title)},{int(redirect)},0.5)"
                 for page, (namespace, title, redirect) in enumerate(self.pages, start=1)]
        targets = [f"({target},{namespace},{quoted(title)})" for (namespace, title), target in self.targets.items()]
        links = [f"({page},{quoted('ключ')},'2026-08-01 00:00:00','',{quoted(kind)},1,{target})" for page, kind, target in self.links]
        files = {
            "page.sql.gz": table("page", ["page_id", "page_namespace", "page_title", "page_is_redirect", "page_random"], pages),
            "linktarget.sql.gz": table("linktarget", ["lt_id", "lt_namespace", "lt_title"], targets),
            "categorylinks.sql.gz": table("categorylinks", ["cl_from", "cl_sortkey", "cl_timestamp", "cl_sortkey_prefix",
                                                            "cl_type", "cl_collation_id", "cl_target_id"], links),
        }
        for name, content in files.items():
            (self.directory / f"{prose.DUMP}-{name}").write_bytes(gzip.compress(b"-- a dump\n" + content, mtime=0))
        entries = "".join(
            f"<page><title>{escape(self.pages[page - 1][1])}</title><ns>{self.pages[page - 1][0]}</ns><id>{page}</id>"
            f"<revision><id>{page}</id><timestamp>2026-08-01T00:00:00Z</timestamp><text>{escape(text)}</text></revision></page>"
            for page, text in sorted(self.texts.items()))
        xml = f'<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/"><siteinfo/>{entries}</mediawiki>'
        (self.directory / f"{prose.DUMP}-pages-articles.xml.bz2").write_bytes(bz2.compress(xml.encode()))

    def pins(self) -> dict[str, tuple[str, str]]:
        return {name: ("sha1", hashlib.sha256((self.directory / f"{prose.DUMP}-{name}").read_bytes()).hexdigest())
                for name in prose.FILES}


class ProseLexiconTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dump = Dump(self.root)
        self.dump.build()
        for name, value in (("FILES", self.dump.pins()), ("FORMS", self.root / "forms.json.gz"),
                            ("RECEIPT", self.root / "receipt.json")):
            patcher = patch.object(prose, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def run_main(self, *arguments: str) -> dict[str, object]:
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(prose.main(list(arguments)), 0)
        return dict(json.loads(output.getvalue()))

    def test_sql_rows_are_read_by_the_columns_of_their_table(self) -> None:
        line = b"INSERT INTO `t` VALUES (1,'a\\'b\\\\c\\n',NULL,-1,0.5),(0,'',NULL,1,'x');\n"
        path = self.root / "t.sql.gz"
        path.write_bytes(gzip.compress(b"CREATE TABLE `t` (\n  `a` int,\n  `b` text,\n  `c` int,\n  `d` int,\n  `e` float,\n"
                                       b"  KEY `a` (`a`)\n) ENGINE=InnoDB;\nINSERT INTO `other` VALUES (7);\n" + line))
        rows = list(prose.sql_rows(path, "t"))
        self.assertEqual(rows[0], {"a": 1, "b": b"a'b\\c\n", "c": None, "d": -1, "e": b"0.5"})
        self.assertEqual(rows[1], {"a": 0, "b": b"", "c": None, "d": 1, "e": b"x"})

    def test_the_rule_selects_the_russian_prose_of_authors_dead_long_enough(self) -> None:
        pages, authors, dropped = prose.select(self.root)
        self.assertEqual(sorted(pages), ["Повесть (Тестов)", f"Рассказ ({AUTHOR.replace('_', ' ')})"])
        self.assertEqual(authors, {AUTHOR.replace("_", " "): ORTHO_PROSE_LAST_DEATH_YEAR})
        self.assertEqual(dropped, {"another author": 1, "pre-reform spelling": 1, "translation": 1})
        story = pages[f"Рассказ ({AUTHOR.replace('_', ' ')})"]
        self.assertEqual((story["author"], story["category"], story["content"]), (AUTHOR.replace("_", " "), f"Проза {AUTHOR.replace('_', ' ')}", KEPT_TEXT))
        self.assertEqual(pages["Повесть (Тестов)"]["timestamp"], "2026-08-01T00:00:00Z")

    def test_forms_are_the_lowercase_words_long_enough_weighed_by_their_count(self) -> None:
        pages, _authors, _dropped = prose.select(self.root)
        table, words = prose.forms(pages)
        self.assertEqual(words, len(KEPT_WORDS))
        self.assertEqual(table, {form: weight(count) for form, count in KEPT_FORMS.items()})
        every = min(ORTHO_V1_MAXIMUM_LEXICON_WEIGHT, ORTHO_PROSE_WORDS_SCALE.bit_length())      # a form that is every word
        self.assertEqual(prose.prose_weight(len(KEPT_WORDS), len(KEPT_WORDS)), every)
        self.assertEqual(prose.prose_weight(0, len(KEPT_WORDS)), 1)

    def test_a_dump_file_that_is_not_the_pinned_one_is_refused(self) -> None:
        pins = dict(prose.FILES)
        name = next(iter(pins))
        pins[name] = (pins[name][0], "another digest")
        with patch.object(prose, "FILES", pins):
            with self.assertRaises(ValueError):
                prose.derive(self.root)

    def test_the_frozen_file_is_byte_stable_and_loads_as_weights(self) -> None:
        table, summary = prose.derive(self.root)
        self.assertEqual(summary["pages"], len(prose.select(self.root)[0]))
        self.assertEqual(summary["left_out"], {"another author": 1, "pre-reform spelling": 1, "translation": 1})
        digest = prose.write(table, self.root / "a.json.gz")
        self.assertEqual(digest, prose.write(table, self.root / "b.json.gz"))
        self.assertEqual(prose.load(self.root / "a.json.gz"), table)
        broken = self.root / "broken.json.gz"
        broken.write_bytes(gzip.compress(json.dumps({"schema_version": 0, "forms": {}}).encode()))
        with self.assertRaises(ValueError):
            prose.load(broken)

    def test_freeze_writes_the_forms_and_a_receipt_that_pins_the_dump_and_the_forms(self) -> None:
        recorded = self.run_main("--freeze", str(self.root))
        self.assertEqual(recorded["schema_version"], ORTHO_PROSE_LEXICON_SCHEMA_VERSION)
        self.assertEqual(recorded["forms"], len(KEPT_FORMS))
        self.assertEqual(recorded["forms_sha256"], hashlib.sha256(prose.FORMS.read_bytes()).hexdigest())
        self.assertEqual(recorded["authors"], {AUTHOR.replace("_", " "): ORTHO_PROSE_LAST_DEATH_YEAR})
        source = dict(recorded["source"])  # type: ignore[call-overload]
        self.assertEqual([entry["sha256"] for entry in source["files"]], [sha for _sha1, sha in prose.FILES.values()])
        self.assertEqual(source["license"], prose.LICENSE)
        with self.assertRaises(ValueError):                     # a frozen lexicon is never overwritten
            prose.main(["--freeze", str(self.root)])
        self.assertEqual(self.run_main(), recorded)              # a plain run checks and prints the receipt
        self.assertEqual(self.run_main("--source", str(self.root)), recorded)   # and replays from the dump
        self.assertFalse(prose.FORMS.with_name(".prose-lexicon-check.json.gz").exists())

    def test_a_changed_file_or_source_is_refused(self) -> None:
        self.run_main("--freeze", str(self.root))
        table, summary = prose.derive(self.root)
        fewer = dict(list(table.items())[:-1])
        with patch.object(prose, "derive", return_value=(fewer, summary)):
            with self.assertRaises(ValueError):
                prose.main(["--source", str(self.root)])
        self.assertFalse(prose.FORMS.with_name(".prose-lexicon-check.json.gz").exists())
        prose.write(fewer, prose.FORMS)
        with self.assertRaises(ValueError):
            prose.main([])

    def test_the_command_line_entry_point_runs_main(self) -> None:
        with patch.object(sys, "argv", ["ortho_prose_lexicon.py", "--help"]), redirect_stdout(io.StringIO()) as output:
            with self.assertRaises(SystemExit) as stopped:
                runpy.run_path(TOOLS_PATH + "/ortho_prose_lexicon.py", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)
        self.assertIn("--freeze", output.getvalue())


if __name__ == "__main__":
    unittest.main()

Answers of the last Python build (0.66.2) on made-up files and on public words: the dictionaries the program ships with and a few
phrases. The tests compare the Rust code with them on every run; no real typing goes in here.

The rows of a token of two to five letters in capitals alone or after Russian text are answered by the Rust code that added
the upper-case head (10.11.2026), not by the Python: the head's features, and the lone-word, short-message and known-reply
heads, which no longer answer such a token, changed them on purpose (132 rows of the five `predict` and `decide` files,
written by these tests run once with every differing row written back). Since the head answers only a pair its counts
settle (15.11.2026), the 42 of those rows whose pair the counts leave alive (`НЕ`, `ЧТО`, `BP` for `ИЗ`) hold the Python's
answer again, written back the same way. Every other row is still the Python's answer. A prediction is compared only while
the installed pair is the one that answered it.

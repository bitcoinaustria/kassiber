"""The probes' SQL dumps mean the same values on every SQLite (tests/rust_port/portable_dump.py)."""
import math
import sqlite3
from contextlib import closing

from tests.rust_port.portable_dump import iterdump, portable


def test_bare_infinity_words_become_numbers_and_quoted_text_does_not():
    statement = (
        """INSERT INTO "Inf"("Inf_col") VALUES(Inf,-Inf,'Inf','it''s Inf',"""
        """X'496E66',1.5,Infinity,NaNInf);"""
    )
    assert portable(statement) == (
        """INSERT INTO "Inf"("Inf_col") VALUES(9.0e+999,-9.0e+999,'Inf','it''s Inf',"""
        """X'496E66',1.5,Infinity,NaNInf);"""
    )
    assert portable("INSERT INTO t VALUES(1);") == "INSERT INTO t VALUES(1);"


def test_an_old_sqlite_dump_restores_the_same_values():
    # SQLite before 3.46 renders an infinite REAL as the bare word Inf.
    def old_quote(value):
        if isinstance(value, float) and math.isinf(value):
            return "Inf" if value > 0 else "-Inf"
        if isinstance(value, str):
            return "'" + value.replace("'", "''") + "'"
        return "NULL" if value is None else repr(value)

    rows = [(float("inf"), "Inf"), (float("-inf"), "-Inf"), (1.5, "it's")]
    with closing(sqlite3.connect(":memory:")) as source:
        source.create_function("quote", 1, old_quote)
        source.execute("CREATE TABLE t(x REAL, y TEXT)")
        source.executemany("INSERT INTO t VALUES(?, ?)", rows)
        dumped = "\n".join(iterdump(source))
    with closing(sqlite3.connect(":memory:")) as target:
        target.executescript(dumped)
        assert target.execute("SELECT x, y FROM t ORDER BY rowid").fetchall() == rows

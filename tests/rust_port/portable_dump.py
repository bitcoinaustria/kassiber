"""SQL dumps the Rust probes can replay whatever SQLite wrote them.

The stdlib dumper renders every value through SQLite's ``quote()``. Up to
SQLite 3.45 (CI's runners ship 3.45.1, Debian 12 ships 3.40) ``quote()``
writes an infinite REAL as the bare word ``Inf``, which no SQLite parses back
as a number; newer releases write ``9.0e+999``. The probes link a current
SQLite, so a dump taken on an older one failed to install ("SQL fixture
installation failed"). ``iterdump`` rewrites those bare words, and only those:
string literals and quoted identifiers pass through untouched, so the text
denotes exactly the stored values on every SQLite.
"""

from __future__ import annotations

from collections.abc import Iterator
from sqlite3.dump import _iterdump

INFINITY = "9.0e+999"


def iterdump(conn) -> Iterator[str]:
    """``Connection.iterdump()``, also for connections that lack it (sqlcipher3)."""
    for statement in _iterdump(conn):
        yield portable(statement)


def portable(statement: str) -> str:
    """``statement`` with every bare ``Inf`` token written as ``9.0e+999``."""
    if "Inf" not in statement:
        return statement
    out: list[str] = []
    index, length = 0, len(statement)
    while index < length:
        char = statement[index]
        if char in "'\"":
            # A literal or identifier runs to the next unpaired quote.
            end = index + 1
            while end < length:
                if statement[end] == char:
                    if end + 1 < length and statement[end + 1] == char:
                        end += 2
                        continue
                    break
                end += 1
            out.append(statement[index : end + 1])
            index = end + 1
            continue
        if (
            statement.startswith("Inf", index)
            and not _word(statement[index - 1 : index])
            and not _word(statement[index + 3 : index + 4])
        ):
            out.append(INFINITY)
            index += 3
            continue
        out.append(char)
        index += 1
    return "".join(out)


def _word(char: str) -> bool:
    return bool(char) and (char.isalnum() or char == "_")

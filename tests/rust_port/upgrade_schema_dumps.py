"""Bring frozen whole-book dumps up to the current store manifest.

Oracle vectors record Python books as ``{table: {"columns": [...], "rows":
[...]}}``, positionally as ``{table: [[value, ...], ...]}``, by name as
``{table: [{column: value}, ...]}``, or as ``iterdump()`` ``INSERT`` seeds. When the schema
gains a nullable or defaulted column, or a new table, the Rust tests rebuild
their books from the current manifest and no longer match the frozen dumps. Upgrading follows SQLite's own rule for an added
column: every existing row reads the column's declared default. It is the state
Python's ``ensure_schema_compat`` leaves when it adds the column to an old book,
so the frozen expectations themselves are unchanged. A dump that holds every
table also gains the new tables, empty.

Run it only on vectors whose tests replay them into a book built from the
current manifest; vectors that carry their own schema DDL keep their layout.
Only the columns listed in ``ADDED`` (and, in an empty table,
``ADDED_WHEN_EMPTY``) are filled in; any other difference refuses. A column
whose migration backfills anything but its default, or a dropped or reordered
column, must regenerate its vectors with their generators instead.

    python -m tests.rust_port.upgrade_schema_dumps [--check] FILE...
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
import sqlite3
import sys

from tests.rust_port.book_schema import MANIFEST

# Columns added since the frozen vectors were cut whose existing rows read the
# declared default (main #675 to #708; see docs/reference/database-compatibility.md).
ADDED = {
    "transactions": ("pricing_review_accepted_price",),
    "ai_providers": ("agent_access",),
    # Rows linked before positions were recorded keep NULL (the same-time
    # source order; database-compatibility.md#same-time-source-order).
    "import_batch_transactions": ("source_position",),
}
# Columns whose migration backfills existing rows (8f numbers import batches
# by `(imported_at, rowid)`): only an empty table's dump gains them exactly.
ADDED_WHEN_EMPTY = {
    "import_batches": ("import_sequence",),
}


def manifest_tables():
    conn = sqlite3.connect(":memory:")
    tables = {}
    for obj in json.loads(MANIFEST.read_text())["objects"]:
        if obj[0] != "table":
            continue
        conn.execute(obj[3])
        info = conn.execute(f'PRAGMA table_info("{obj[1]}")').fetchall()
        tables[obj[1]] = dict(
            columns=[row[1] for row in info],
            defaults={row[1]: literal(row[4]) for row in info},
            rowid="WITHOUT ROWID" not in obj[3].upper(),
        )
    conn.close()
    return tables


def literal(default):
    if default is None:
        return None
    return sqlite3.connect(":memory:").execute("SELECT " + default).fetchone()[0]


def is_table_dump(value):
    return (isinstance(value, dict) and set(value) == {"columns", "rows"}
            and isinstance(value["columns"], list) and isinstance(value["rows"], list))


def upgrade_table(name, dump, schema, changes, path):
    columns = dump["columns"]
    prefix = ["_rowid_"] if columns[:1] == ["_rowid_"] else []
    target = prefix + schema["columns"]
    if columns == target:
        return
    missing = [c for c in target if c not in columns]
    allowed = set(ADDED.get(name, ())) | (set(ADDED_WHEN_EMPTY.get(name, ())) if not dump["rows"] else set())
    if [c for c in target if c in columns] != columns or not set(missing) <= allowed:
        raise SystemExit(f"{path}: {name} columns differ from the manifest beyond ADDED")
    rows = []
    for row in dump["rows"]:
        values = dict(zip(columns, row, strict=True))
        rows.append([values[c] if c in values else schema["defaults"][c] for c in target])
    dump["columns"] = target
    dump["rows"] = rows
    changes.add(f"{name}+{','.join(missing)}")


def upgrade_positional(name, rows, schema, changes, path):
    columns = schema["columns"]
    old = [c for c in columns if c not in ADDED.get(name, ())]
    for index, row in enumerate(rows):
        if len(row) == len(columns):
            continue
        if len(row) != len(old) or len(old) == len(columns):
            raise SystemExit(f"{path}: {name} row has {len(row)} values for {len(columns)} columns")
        values = dict(zip(old, row, strict=True))
        rows[index] = [values[c] if c in values else schema["defaults"][c] for c in columns]
        changes.add(f"{name}+{','.join(c for c in columns if c not in old)}")


def upgrade_named(name, rows, schema, changes, path):
    """Rows as ``{column: value}`` dicts in column order."""
    columns = schema["columns"]
    def key(row):
        return json.dumps(row, sort_keys=True)
    ordered = rows == sorted(rows, key=key)
    for index, row in enumerate(rows):
        missing = [c for c in columns if c not in row]
        if not missing:
            continue
        if not set(row) <= set(columns) or not set(missing) <= set(ADDED.get(name, ())):
            raise SystemExit(f"{path}: {name} row keys differ from the manifest beyond ADDED")
        if list(row) == [c for c in columns if c in row]:
            order = columns
        elif list(row) == sorted(row):
            order = sorted(columns)
        else:
            raise SystemExit(f"{path}: {name} row keys follow neither column nor sorted order")
        rows[index] = {c: row[c] if c in row else schema["defaults"][c] for c in order}
        changes.add(f"{name}+{','.join(missing)}")
    if ordered:
        rows.sort(key=key)


def is_positional_book(value, schema):
    return (isinstance(value, dict) and value and set(value) <= set(schema)
            and len(value) > len(schema) * 9 // 10
            and all(isinstance(rows, list) and (all(isinstance(r, list) for r in rows)
                                                or all(isinstance(r, dict) for r in rows))
                    for rows in value.values()))


def is_overrides(value, schema):
    return (isinstance(value, dict) and set(value) <= set(schema)
            and all(isinstance(rows, list) and all(isinstance(r, list) for r in rows)
                    for rows in value.values()))


def upgrade_packed(value, schema, changes, path):
    """``{base: whole-book dump, cases: [{side: {table: rows}}]}``: each case's
    rows replace the base table's rows and use its columns."""
    base = value["base"]
    before = {name: list(base[name]["columns"]) for name in ADDED if is_table_dump(base.get(name))}
    upgrade(base, schema, changes, path)
    for case in value["cases"]:
        for key, side in case.items():
            if not is_overrides(side, schema):
                upgrade(side, schema, changes, path)
                continue
            for name, rows in side.items():
                if name not in before or not rows:
                    continue
                old, new = before[name], base[name]["columns"]
                if old == new:
                    continue
                for index, row in enumerate(rows):
                    values = dict(zip(old, row, strict=True))
                    rows[index] = [values[c] if c in values else schema[name]["defaults"][c] for c in new]
    for key, item in value.items():
        if key not in ("base", "cases"):
            upgrade(item, schema, changes, path)


SQL = sqlite3.connect(":memory:")


def upgrade_insert(statement, schema, changes, path):
    """Widen an ``iterdump()`` ``INSERT INTO "t" VALUES(...)`` for an added column.

    Values are re-rendered with ``quote()``, as ``iterdump()`` writes them, and
    the untouched values must reproduce the original statement exactly."""
    # One statement, or several joined into a script; only INSERTs change.
    parts = re.split(r"(?<=;)\n", statement)
    return "\n".join(widen(part, schema, changes, path) if part.startswith('INSERT INTO "')
                     else part for part in parts)


def widen(statement, schema, changes, path):
    end = ";" if statement.endswith(";") else ""
    head, _, rest = statement[:len(statement) - len(end)].partition(" VALUES(")
    name = head[len('INSERT INTO "'):-1]
    if name not in ADDED:
        return statement
    if not rest.endswith(")"):
        raise SystemExit(f"{path}: cannot parse {statement[:80]!r}")
    try:
        width = len(SQL.execute(f"VALUES({rest}").description)
    except sqlite3.Error as error:
        raise SystemExit(f"{path}: cannot evaluate {statement[:80]!r}: {error}")
    columns = schema[name]["columns"]
    old = [c for c in columns if c not in ADDED[name]]
    if width == len(columns):
        return statement
    if width != len(old):
        raise SystemExit(f"{path}: {name} insert has {width} values for {len(columns)} columns")
    quoted = SQL.execute(
        "SELECT " + ",".join(f"quote(column{i + 1})" for i in range(width)) + f" FROM (VALUES({rest})"
    ).fetchone()
    if f'{head} VALUES({",".join(quoted)}){end}' != statement:
        raise SystemExit(f"{path}: {name} insert does not round-trip through quote()")
    values = dict(zip(old, quoted, strict=True))
    for column in columns:
        if column not in values:
            values[column] = SQL.execute("SELECT quote(?)", (schema[name]["defaults"][column],)).fetchone()[0]
    changes.add(f"{name}+{','.join(c for c in columns if c not in old)}")
    return f'{head} VALUES({",".join(values[c] for c in columns)}){end}'


def upgrade(value, schema, changes, path):
    if isinstance(value, list):
        for index, item in enumerate(value):
            if isinstance(item, str) and 'INSERT INTO "' in item[:13]:
                value[index] = upgrade_insert(item, schema, changes, path)
            else:
                upgrade(item, schema, changes, path)
        return
    if not isinstance(value, dict):
        return
    for key, item in value.items():
        if isinstance(item, str) and ('INSERT INTO "' in item[:13] or '\nINSERT INTO "' in item):
            value[key] = upgrade_insert(item, schema, changes, path)
    if isinstance(value.get("base"), dict) and isinstance(value.get("cases"), list) \
            and any(is_table_dump(v) for v in value["base"].values()):
        upgrade_packed(value, schema, changes, path)
        return
    if is_positional_book(value, schema):
        for name, rows in value.items():
            if rows and isinstance(rows[0], dict):
                upgrade_named(name, rows, schema[name], changes, path)
            else:
                upgrade_positional(name, rows, schema[name], changes, path)
        absent = [t for t in schema if t not in value]
        for name in absent:
            value[name] = []
            changes.add(f"+{name}")
        if absent and list(value)[:-len(absent)] == sorted(list(value)[:-len(absent)]):
            ordered = sorted(value.items())
            value.clear()
            value.update(ordered)
        return
    if set(value) <= set(schema) and all(isinstance(rows, list) and rows and all(isinstance(r, dict) for r in rows)
                                         for name, rows in value.items() if name in ADDED):
        # Partial named rows (the tables a case changed) name their columns.
        for name, rows in value.items():
            if name in ADDED and rows:
                upgrade_named(name, rows, schema[name], changes, path)
    partial = [k for k, v in value.items() if k in ADDED and isinstance(v, list) and v
               and all(isinstance(r, list) for r in v) and set(value) <= set(schema)]
    if partial:
        # Packed rows that borrow another dump's columns: their width alone
        # cannot tell an added column from a _rowid_ prefix.
        raise SystemExit(f"{path}: partial positional rows for {partial}; regenerate instead")
    dumps = {k: v for k, v in value.items() if k in schema and is_table_dump(v)}
    for name, dump in dumps.items():
        upgrade_table(name, dump, schema[name], changes, path)
    # A whole-book dump names every table but the ones added since it was cut.
    absent = [t for t in schema if t not in value]
    if dumps and len(dumps) == len(value) and len(absent) < len(schema) // 10:
        rowid = any(d["columns"][:1] == ["_rowid_"] for d in dumps.values())
        for name in absent:
            columns = (["_rowid_"] if rowid and schema[name]["rowid"] else []) + schema[name]["columns"]
            value[name] = dict(columns=columns, rows=[])
            changes.add(f"+{name}")
        if absent and list(value) != sorted(value, key=lambda k: k):
            ordered = sorted(value.items())
            if [k for k, _ in ordered if k not in absent] == [k for k in value if k not in absent]:
                value.clear()
                value.update(ordered)
    for key, item in value.items():
        if key not in dumps:
            upgrade(item, schema, changes, path)


# The layouts the vector generators write; a file must reproduce byte for byte.
FORMATS = tuple(
    (lambda data, o=dict(ensure_ascii=ascii_only, sort_keys=sort_keys, **layout): json.dumps(data, **o) + "\n")
    for ascii_only in (True, False)
    for sort_keys in (False, True)
    for layout in (dict(separators=(",", ":")), dict(indent=2), dict(indent=1), {})
) + (
    # One compact case per line (gen_gain_vectors.py).
    lambda data: "[\n" + ",\n".join(
        json.dumps(item, ensure_ascii=True, separators=(",", ":")) for item in data) + "\n]\n",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="fail if any file needs upgrading")
    parser.add_argument("files", nargs="+", type=Path)
    args = parser.parse_args()
    schema = manifest_tables()
    stale = False
    for path in args.files:
        text = path.read_text()
        data = json.loads(text)
        layout = next((f for f in FORMATS if f(data) == text), None)
        changes = set()
        upgrade(data, schema, changes, path)
        if not changes:
            print(f"{path}: current")
            continue
        if layout is None:
            raise SystemExit(f"{path}: no known JSON layout reproduces the file byte for byte")
        stale = True
        print(f"{path}: {' '.join(sorted(changes))}")
        if not args.check:
            path.write_text(layout(data))
    if args.check and stale:
        sys.exit(1)


if __name__ == "__main__":
    main()

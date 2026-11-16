#!/usr/bin/env python3
"""Checks or re-resolves parity/fixtures.json against a demo book (tooling only, stdlib).

    python3 -I tools/demo-fixtures.py check   <kassiber.sqlite3>
    python3 -I tools/demo-fixtures.py resolve <kassiber.sqlite3>

`check` runs every fixture's `select` and fails when it does not return exactly one row equal to
the stored id and fields. `resolve` rewrites the stored ids and fields from the selects (the step
after replacing the demo book). The book is opened read-only (`mode=ro`); pass a plaintext copy.
Scripts are read by Rune itself, so their ids stay literal (the check below covers them).
It also flags every UUID-shaped string left in parity scenes, scripts and Rust scene files that is
neither a fixture nor present in the book, so a stale id cannot hide outside the fixture file.
"""
import json
import os
import re
import sqlite3
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FIX = os.path.join(ROOT, "parity", "fixtures.json")
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


def rows(db, sql):
    cur = db.execute(sql)
    names = [d[0] for d in cur.description]
    return [dict(zip(names, r)) for r in cur.fetchall()]


def stale_refs(db, fixture_ids):
    """UUIDs in scene inputs that are neither fixtures nor rows of the book."""
    tables = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type = 'table'")]
    id_tables = [t for t in tables if any(c[1] == "id" for c in db.execute(f'PRAGMA table_info("{t}")'))]

    def known(u):
        return u in fixture_ids or any(db.execute(f'SELECT 1 FROM "{t}" WHERE id = ? LIMIT 1', (u,)).fetchone() for t in id_tables)

    files = [os.path.join(ROOT, "parity", "scenes.json")]
    for d, ext in (("parity/scenes.d", ".json"), ("scripts", ".json")):
        p = os.path.join(ROOT, d)
        files += [os.path.join(p, f) for f in sorted(os.listdir(p)) if f.endswith(ext)]
    for dirpath, _, names in os.walk(os.path.join(ROOT, "src")):
        files += [os.path.join(dirpath, f) for f in names if f.endswith(".rs") and ("scene" in f or f in ("main.rs", "mod.rs"))]
    out = []
    for f in files:
        with open(f, encoding="utf-8") as fh:
            text = fh.read()
        if f.endswith(".rs"):
            # Only the scene tables: the code under `#[cfg(test)]` carries synthetic ids by design.
            text = re.split(r"#\[cfg\(test\)\]\s*mod tests", text)[0]
        # `txd-revert-<event id>` names an edit event an earlier script creates at runtime.
        text = re.sub(r"txd-revert-" + UUID.pattern, "", text)
        for u in sorted(set(UUID.findall(text))):
            if not known(u):
                out.append((os.path.relpath(f, ROOT), u))
    return out


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("check", "resolve"):
        print(__doc__)
        return 2
    mode, path = sys.argv[1], sys.argv[2]
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    with open(FIX, encoding="utf-8") as fh:
        cfg = json.load(fh)
    bad = 0
    for name, fx in cfg["fixtures"].items():
        got = rows(db, fx["select"])
        if len(got) != 1:
            print(f"FAIL {name}: select returned {len(got)} rows")
            bad += 1
            continue
        for k, v in got[0].items():
            v = str(v)
            if fx.get(k) != v:
                if mode == "resolve":
                    print(f"set  {name}.{k}: {fx.get(k)} -> {v}")
                    fx[k] = v
                else:
                    print(f"FAIL {name}.{k}: stored {fx.get(k)}, book {v}")
                    bad += 1
    if mode == "resolve":
        with open(FIX, "w", encoding="utf-8") as f:
            json.dump(cfg, f, indent=1, ensure_ascii=False)
            f.write("\n")
    ids = {fx["id"] for fx in cfg["fixtures"].values()}
    for f, u in stale_refs(db, ids):
        print(f"STALE {f}: {u}")
        bad += 1
    print("ok" if not bad else f"{bad} problem(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

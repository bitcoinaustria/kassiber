"""The current book schema's tables, read from the store manifest.

Both runtimes admit exactly this manifest, so whole-book comparisons of a book
current code created check their table list against it instead of a count.
Frozen seeds (``privacy_hygiene_seed.sql``) keep the layout they were cut from.
"""
import json
from pathlib import Path
import sqlite3

MANIFEST = Path(__file__).resolve().parents[2] / "crates/store/fixtures/current-schema.json"
TABLES = tuple(sorted(
    obj[1] for obj in json.loads(MANIFEST.read_text())["objects"] if obj[0] == "table"))

# Books built from the frozen privacy seed keep the layout it was cut from.
SEED = MANIFEST.parents[3] / "crates/reports/tests/data/privacy_hygiene_seed.sql"


def _seed_tables() -> tuple[str, ...]:
    conn = sqlite3.connect(":memory:")
    try:
        conn.executescript(SEED.read_text())
        return tuple(row[0] for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"))
    finally:
        conn.close()


SEED_TABLES = _seed_tables()

"""The current book schema's tables, read from the store manifest.

Both runtimes admit exactly this manifest, so whole-book comparisons of a book
current code created check their table list against it instead of a count.
Frozen seeds (``privacy_hygiene_seed.sql``) keep the layout they were cut from.
"""
import json
from pathlib import Path

MANIFEST = Path(__file__).resolve().parents[2] / "crates/store/fixtures/current-schema.json"
TABLES = tuple(sorted(
    obj[1] for obj in json.loads(MANIFEST.read_text())["objects"] if obj[0] == "table"))

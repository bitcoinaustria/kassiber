"""Export semantic migration predicates/templates; never execute in product."""
from __future__ import annotations
import argparse
import ast
import hashlib
import inspect
import json
from pathlib import Path
import tempfile

from kassiber import db
from kassiber.core import acquisition_review

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/semantics.json"
MIGRATIONS = (
    (db.JOURNAL_SEMANTICS_ADOPTION_MIGRATION, db._migrate_journal_semantics_adoption),
    (db.WAGES_ACQUISITION_SEMANTICS_MIGRATION, db._migrate_wages_acquisition_semantics),
    (db.RP2_POOL_BASIS_SEMANTICS_MIGRATION, db._migrate_rp2_pool_basis_semantics),
    (db.CUSTODY_HOLD_SEMANTICS_MIGRATION, db._migrate_custody_hold_semantics),
    (db.TAX_ENGINE_SEMANTICS_MIGRATION, db._migrate_tax_engine_semantics),
    (db.AT_OUTFLOW_REGIME_SEMANTICS_MIGRATION, db._migrate_at_outflow_regime_semantics),
    (acquisition_review.VALUATION_MIGRATION, acquisition_review.migrate_valuation_support),
    (db.PARTIAL_PAYOUT_SEMANTICS_MIGRATION, db._migrate_partial_payout_semantics),
)


def payload():
    with tempfile.TemporaryDirectory(prefix="kassiber-r3-semantics-") as root:
        conn = db.open_db(Path(root) / "data")
        audits = {row["migration_name"]: row["impact_json"] for row in conn.execute("SELECT * FROM schema_migration_audits")}
        conn.close()
    results = []
    for name, function in MIGRATIONS:
        source = inspect.getsource(function)
        tree = ast.parse(source)
        predicates = [node.value.value for node in ast.walk(tree) if isinstance(node, ast.Assign)
                      and isinstance(node.value, ast.Constant) and isinstance(node.value.value, str)
                      and any(isinstance(t, ast.Name) and t.id == "affected" for t in node.targets)]
        results.append(dict(name=name, affected=predicates[0] if predicates else None,
                            impact=json.loads(audits[name]),
                            source_sha256=hashlib.sha256(source.encode()).hexdigest()))
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(payload(), indent=2, ensure_ascii=True) + "\n"
    if args.check:
        assert DESTINATION.read_text() == encoded, "Python semantic migrations changed; review Rust port"
    else:
        DESTINATION.write_text(encoded)


if __name__ == "__main__":
    main()

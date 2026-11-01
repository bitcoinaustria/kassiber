import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "crates/wallets/scripts"))
SPEC = importlib.util.spec_from_file_location("analysis_source_repro", ROOT / "crates/wallets/scripts/generate_analysis_sources.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def reproduce():
    sql = REFERENCE.stored(raw_json=REFERENCE.raw(vout=[{"value": 1, "scriptpubkey": "aa  bb"}]))
    case = {"sql": sql}
    rejected = REFERENCE.evaluate(case)
    case["sql"] += "ALTER TABLE chain_analysis_datasets RENAME TO unavailable_chain_analysis_datasets;"
    accepted = REFERENCE.evaluate(case)
    assert rejected == {"error": "validation"}
    assert any(fact.get("script") == "aa  bb" for fact in accepted["output_facts"].values())
    return {"with_dataset_table": rejected, "without_dataset_table": "accepted"}


if __name__ == "__main__":
    import json
    print(json.dumps(reproduce(), sort_keys=True))

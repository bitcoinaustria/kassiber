import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "crates/wallets/scripts"))
from generate_analysis_sources import database, dataset, evaluate, stored


def reproduce():
    result = {}
    for name, value in [("non_finite_manifest", float("nan")), ("surrogate_manifest", "\ud800")]:
        case = {"sql": stored() + dataset(manifest={"unused": value})}
        conn = database(case["sql"])
        try:
            evaluate(case, conn)
        except (ValueError, UnicodeError) as error:
            result[name] = type(error).__name__
        else:
            result[name] = "accepted"
        finally:
            conn.close()
    return result


if __name__ == "__main__":
    print(json.dumps(reproduce(), sort_keys=True))

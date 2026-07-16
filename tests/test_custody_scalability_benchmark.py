import json
import subprocess
import sys
from pathlib import Path


def test_custody_scalability_smoke_preserves_structural_invariants():
    script = (
        Path(__file__).resolve().parents[1]
        / "scripts"
        / "benchmark-custody-scalability.py"
    )
    completed = subprocess.run(
        [sys.executable, str(script), "--smoke"],
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr or completed.stdout
    events = [json.loads(line) for line in completed.stdout.splitlines()]
    results = {
        event["stage"]: event
        for event in events
        if event["event"] == "benchmark_result"
    }
    assert set(results) == {"atomic", "lineage", "gaps"}
    assert all(result["ok"] for result in results.values())
    assert results["atomic"]["metrics"]["invariants"][
        "constant_decision_traversals"
    ]
    assert results["lineage"]["metrics"]["invariants"][
        "ordered_page_avoids_temp_sort"
    ]
    assert results["lineage"]["metrics"]["invariants"][
        "transaction_lookup_uses_multi_index_or"
    ]
    assert results["gaps"]["metrics"]["relevant_outbounds"] > 87
    assert results["gaps"]["metrics"]["invariants"][
        "structured_candidate_survives_large_book"
    ]
    assert events[-1] == {
        "event": "benchmark_complete",
        "schema_version": 1,
        "ok": True,
    }

"""Disposable repro: graph transfer-group annotations follow PYTHONHASHSEED.

`_annotate_graph` collects a row's annotation group ids into a set and appends
one `linked_transfer_group` annotation per id to each owned destination, so
with two or more groups their order depends on string hashing. Rust sorts the
ids. Python product code is unchanged. Run with TMPDIR under the worktree's
target/; it starts child interpreters with different hash seeds.
"""
import os
import subprocess
import sys

CHILD = r"""
from kassiber.core.transaction_graph import _annotate_graph
class Index:
    def lookup_outpoint(self, outpoint):
        return [type("M", (), dict(wallet_id="w2", wallet_label="Two", chain="bitcoin",
                                   network="main", branch_label="receive"))()]
    def lookup_script(self, script):
        return []
graph = {"inputs": [], "outputs": [{"id": "out-0", "outpoint": "aa:0", "_script": None,
                                    "ownership": "unknown", "role": "output", "annotations": []}]}
row = {"id": "t", "wallet_id": "w1", "direction": "outbound", "raw_json": "{}"}
groups = [{"groupId": f"custody-decision:{name}"} for name in "abcdefgh"]
_annotate_graph(graph, row, Index(), {"by_row": {"t": groups}}, raw={})
print([a.get("groupId") for a in graph["outputs"][0]["annotations"] if a["code"] == "linked_transfer_group"])
"""


def main():
    orders = set()
    for seed in ("1", "2", "3", "4"):
        env = dict(os.environ, PYTHONHASHSEED=seed)
        out = subprocess.run([sys.executable, "-c", CHILD], env=env, capture_output=True, text=True, check=True)
        orders.add(out.stdout.strip())
    if len(orders) > 1:
        print(f"Python reference bug reproduced: {len(orders)} annotation orders across hash seeds")
    else:
        raise SystemExit("expected the annotation order to vary with PYTHONHASHSEED")


if __name__ == "__main__":
    main()

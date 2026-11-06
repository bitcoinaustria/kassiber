"""Check: graph transfer-group annotations no longer follow PYTHONHASHSEED.

`_annotate_graph` collected a row's annotation group ids into a set and
appended one `linked_transfer_group` annotation per id to each owned
destination, so with two or more groups their order depended on string
hashing. It now sorts the ids, as Rust does. Run with TMPDIR under the
worktree's target/; it starts child interpreters with different hash seeds.
Exit 0 means one sorted order under every seed.
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
        env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        env["PYTHONHASHSEED"] = seed
        out = subprocess.run([sys.executable, "-c", CHILD], env=env, capture_output=True, text=True, check=True)
        orders.add(out.stdout.strip())
    expected = str([f"custody-decision:{name}" for name in "abcdefgh"])
    if orders != {expected}:
        raise SystemExit(f"annotation order still varies or is unsorted: {sorted(orders)}")
    print("fixed: one sorted annotation order across hash seeds")


if __name__ == "__main__":
    main()

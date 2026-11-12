"""Backend kind aliases, natively (round 2, C9).

Python routes a backend by `normalize_backend_kind`: `mempool` and
`liquid-esplora` are Esplora. Stored backends may carry either kind. The
hybrid runner must serve such a sync natively, and Python's CLI and the
native binary must store the same bytes; the raw kind stays on the output
inventory rows, the normalized one on the outcome.
"""
import copy
import json
import subprocess
import sys

import pytest

from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.rust_liquid_chain import esplora as liquid_esplora
from tests.rust_sync_chain import serve
from tests.rust_sync_parity import connect, dump_bytes
from tests.test_rust_lwk_execution import VECTORS
from tests.test_rust_lwk_sync_service import wallet_config
from tests.test_rust_wallet_publication import _fixture
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_wallet_sync_cli import HYBRID, RUST_BIN, _args, _books, _environment

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")


def _round(tmp_path, chain, kind, url, roots, books, index=0):
    """Python's CLI, then the hybrid runner (which must serve it natively),
    on copies of one book from the same chain state."""
    before = chain and copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    env = _environment(tmp_path, kind, url, index)
    reference = subprocess.run([sys.executable, "-m", "tests.rust_sync_cli_reference", *_args(roots[0])],
                               capture_output=True, env=env, timeout=90)
    if chain:
        with chain.lock:
            chain.__dict__.update(before)
    actual = subprocess.run([sys.executable, str(HYBRID), *_args(roots[1])],
                            capture_output=True, env=env, timeout=90)
    assert actual.stderr == reference.stderr == b"", (reference.stderr, actual.stderr)
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    assert actual.returncode == reference.returncode == 0, (expected, result)
    expected["data"], result["data"] = fixed_elapsed_ms(expected["data"]), fixed_elapsed_ms(result["data"])
    assert result == expected
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    log = [json.loads(line) for line in (tmp_path / "served.jsonl").read_text().splitlines()]
    assert log == [{"command": "wallets.sync", "served_by": "rust"}] * (index + 1)
    return expected["data"][0]


@pytest.mark.parametrize("fixture", ["receive-change", "reorg"])
def test_mempool_kind_descriptor_matches_python(tmp_path, fixture):
    config, chain, rounds = _fixture(fixture)
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            row = _round(tmp_path, chain, "mempool", url, roots, books, index)
    assert (row["status"], row["backend_kind"]) == ("synced", "esplora"), row
    conn = connect(books[1])
    kinds = {r[0] for r in conn.execute("SELECT backend_kind FROM wallet_utxo_refreshes")}
    conn.close()
    assert kinds == {"mempool"}


def test_mempool_kind_address_list_matches_python(tmp_path):
    config, chain, rounds = _fixture("receive-change")
    plan = load_descriptor_plan(config)
    addresses = [t.address for t in derive_descriptor_targets(plan, start=0, end=4) if t.branch_index == 0]
    roots, books = _books(tmp_path, {"addresses": addresses, "chain": "bitcoin", "network": plan.network,
                                     "backend": config.get("backend")})
    with serve(chain, "esplora") as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            row = _round(tmp_path, chain, "mempool", url, roots, books, index)
    assert row["status"] == "synced", row


def test_liquid_esplora_kind_matches_python(tmp_path):
    roots, books = _books(tmp_path, wallet_config())
    state = {"phase": VECTORS["phases"]["compat_initial"]}
    with liquid_esplora(state) as url:
        _round(tmp_path, None, "liquid-esplora", url, roots, books)
        state["phase"] = VECTORS["phases"]["compat_more"]
        row = _round(tmp_path, None, "liquid-esplora", url, roots, books, 1)
    assert (row["status"], row["observer_route"]) == ("synced", "compatibility"), row

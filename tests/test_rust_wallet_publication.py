"""Full-book Bitcoin publication parity: Python and Rust apply the same prepared fetch.

Python's unchanged preparation produces each round's fetch against a loopback
chain; both implementations then publish it atomically under identical
deterministic id/clock sequences, and every table must match exactly.
"""
from contextlib import ExitStack
import json
import os
from pathlib import Path
import sqlite3

import pytest

from tests.rust_sync_chain import FOREIGN, Chain, serve
from tests.rust_sync_parity import (
    connect, copy_book, create_book, deserialize_fetch, dump, dump_bytes, fixed_elapsed_ms, python_apply, python_prefetch,
    runtime, rust_apply, serialize_fetch, seed_reviewed_state,
)
from tests.rust_fixture_binary import fixture_binary
from tests.rust_sync_env import child_env as _env
from tests.test_rust_wallet_discovery import _config
from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.egress_guard import guarded_pythonpath

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the Rust publication fixture build")
STAGES = ["begin", "observer_persistence", "retractions", "transaction_insertion", "output_inventory",
          "derivation_coverage", "freshness_checkpoint", "commit"]


def _binary():
    return fixture_binary("publication_fixture")


def _wallet(name, gap):
    config = _config(name, gap)
    config["backend"] = "fixture"
    plan = load_descriptor_plan(config)
    targets = list(derive_descriptor_targets(plan, start=0, end=8))
    by_branch = {}
    for target in targets:
        by_branch.setdefault(target.branch_index, []).append(target.script_pubkey)
    return config, by_branch


def _fixture(name):
    """(config, chain, rounds): `rounds` mutate the chain before each sync."""
    chain = Chain()
    if name == "multi-script":
        config, scripts = _wallet("multi-script-p2tr+p2pkh+p2wpkh+p2sh-p2wpkh", 3)
        branches = sorted(scripts)
        funded = [chain.fund(scripts[branch][0], 30_000 + index) for index, branch in enumerate(branches[::2])]
        chain.spend(funded[:2], [(scripts[branches[1]][1], 59_000)], height=14)
        return config, chain, [None]
    if name == "fixed":
        config, scripts = _wallet("wpkh-fixed", 3)
        script = next(iter(scripts.values()))[0]
        first = chain.fund(script, 40_000)
        chain.fund(script, 41_000)
        chain.spend([first], [(FOREIGN, 39_000)], height=15)
        return config, chain, [None]
    config, scripts = _wallet("wpkh", 1 if name == "gap-one" else 3)
    receive, change = scripts[0], scripts.get(1, scripts[0])
    rounds = [None]
    if name == "receive-change" or name == "checkpoint":
        parent = chain.fund(receive[1], 100_000)
        chain.spend([parent], [(FOREIGN, 50_000), (change[1], 48_000)], height=12)
        if name == "checkpoint":
            rounds = [None, None, lambda: chain.fund(receive[2], 7_000, height=16)]
    elif name == "gap-one":
        chain.fund(receive[0], 10_000)
        chain.fund(receive[1], 11_000)
        chain.fund(receive[3], 12_000)
    elif name in {"reorg", "retraction"}:
        parent = chain.fund(receive[0], 20_000)

        def mutate():
            chain.reorg(parent_height)
            if name == "reorg":
                chain.confirm(parent[0], parent_height + 1)
            else:
                chain.drop(parent[0])
        parent_height = chain.heights[parent[0]]
        rounds = [None, mutate]
    elif name == "mempool":
        txid = chain.spend([chain.fund(FOREIGN, 30_000)], [(receive[0], 29_000)])
        rounds = [None, lambda: chain.confirm(txid, 17)]
    elif name == "coinbase":
        chain.coinbase(receive[0], 625_000_000, height=13)
    elif name != "empty":
        raise AssertionError(name)
    return config, chain, rounds


def _sequence(name):
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    receive = scripts[0]
    source = chain.fund(FOREIGN, 25_000)
    mempool = name in {"mempool-drop-return", "replacement", "force-full", "force-without-state"}
    txid = chain.spend([source], [(receive[0], 24_000)], height=None if mempool else 14)
    state = {"force": None, "sql": None}

    def second():
        if name == "reanchor":
            chain.reorg(14)
        elif name == "reorg-drop":
            chain.reorg(14)
            chain.drop(txid)
        elif name == "preflight-race":
            chain.fork = 1
            chain.race_on_header = True
        elif name in {"mempool-drop-return", "force-full", "force-without-state"}:
            chain.drop(txid)
            if name != "mempool-drop-return":
                state["force"] = True
            if name == "force-without-state":
                state["sql"] = "UPDATE chain_observer_instances SET state_json='{\"schema_version\":1}'"
        elif name == "replacement":
            chain.drop(txid)
            state["replacement"] = chain.spend([source], [(receive[0], 23_500)])
        elif name == "height-change":
            chain.reorg(14)
            chain.confirm(txid, 15)
        else:
            raise AssertionError(name)

    def third():
        state["force"] = None
        state["sql"] = None
        if name in {"reorg-drop", "mempool-drop-return", "force-full", "force-without-state"}:
            chain.unconfirm(txid)
        if name == "replacement":
            chain.confirm(state["replacement"], 18)

    return config, chain, [None, second, third], state


def _round(tmp_path, protocol, url, python_book, rust_book, *, force_full=False, sql=None, index=0, reviewed=False,
           clear_authority=False):
    rt = runtime(protocol, url)
    if reviewed and index:
        for book in (python_book, rust_book):
            conn = connect(book)
            seed_reviewed_state(conn, clear_authority=clear_authority and index == 1)
            conn.close()
    for book in (python_book, rust_book):
        if sql:
            conn = connect(book)
            conn.executescript(sql)
            conn.commit()
            conn.close()
    conn = connect(python_book)
    try:
        fetch, checkpoints = python_prefetch(conn, rt, force_full=force_full)
        data = serialize_fetch(fetch)
        captured = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
        if "error" in data:
            # Preparation refused before any write; nothing to publish.
            return data, None
        expected = python_apply(conn, rt, deserialize_fetch(data), checkpoints, force_full=force_full,
                                prefix=f"r{index}", now=index * 1000)
    finally:
        conn.close()
    actual = rust_apply(_binary(), rust_book, data, captured, _env(tmp_path), prefix=f"r{index}", now=index * 1000)
    assert ("error" in actual) == ("error" in expected), (expected, actual)
    if "error" in expected:
        assert actual["error"]["code"] == expected["error"]["code"], (expected, actual)
    else:
        assert fixed_elapsed_ms(actual["result"]) == json.loads(json.dumps(fixed_elapsed_ms(expected["result"]), sort_keys=True))
    assert dump_bytes(rust_book) == dump_bytes(python_book)
    return data, expected


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one",
                                     "reorg", "retraction", "mempool", "coinbase"])
def test_publication_fixture_books_match_python(tmp_path, protocol, fixture):
    config, chain, rounds = _fixture(fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        published = 0
        for index, mutate in enumerate(rounds):
            if mutate is not None:
                mutate()
            data, expected = _round(tmp_path, protocol, url, python_book, rust_book, index=index)
            published += expected is not None and "result" in expected
            if index == 0 and fixture != "empty":
                assert dump(python_book)["transactions"], "a non-empty fixture must publish transactions"
        assert published == len(rounds)
    final = dump(python_book)
    if fixture == "retraction":
        assert not final["transactions"], "the dropped transaction must be retracted"
    assert final["freshness_source_states"]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return",
                                      "replacement", "height-change", "force-full", "force-without-state"])
def test_publication_sequence_books_match_python_across_restarts(tmp_path, protocol, sequence):
    config, chain, rounds, state = _sequence(sequence)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    outcomes = []
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate is not None:
                mutate()
            _, expected = _round(tmp_path, protocol, url, python_book, rust_book,
                                 force_full=bool(state["force"]), sql=state["sql"], index=index)
            outcomes.append(expected)
    assert len(outcomes) == 3
    assert all(outcome is not None and "result" in outcome for outcome in outcomes), outcomes


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fixture", ["checkpoint", "reorg", "retraction", "mempool", "authority"])
def test_publication_preserves_saved_reports_and_current_journals(tmp_path, protocol, fixture):
    config, chain, rounds = _fixture("checkpoint" if fixture == "authority" else fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate is not None:
                mutate()
            _, expected = _round(tmp_path, protocol, url, python_book, rust_book, index=index, reviewed=True,
                                 clear_authority=fixture == "authority")
            assert expected is not None and "result" in expected
            if fixture == "authority" and index == 1:
                result = expected["result"][0]
                assert result["imported"] == 0 and not result["updated_records"]
                # Electrum republishes the unchanged record; an Esplora cache
                # hit supplies no records and therefore restores no authority.
                assert result["journal_invalidated"] == bool(result["records_fetched"])
    final = dump(rust_book)
    assert final["filed_report_chain_dependencies"]
    if fixture in {"retraction", "mempool", "reorg"}:
        assert final["filed_report_chain_impacts"], "changed saved evidence must append a report impact"
    elif fixture == "authority" and protocol == "esplora":
        # Its cache hit leaves the removed closed authority absent. Reports
        # must expose that change even though no transaction was fetched.
        assert final["filed_report_chain_impacts"]


def _written_tables(before, after):
    return sorted(table for table in set(before) | set(after) if before.get(table) != after.get(table))


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_publication_rolls_back_every_stage_and_table_write(tmp_path, protocol):
    config, chain, rounds = _fixture("receive-change")
    base = tmp_path / "base.sqlite"
    create_book(base, config)
    with serve(chain, protocol) as url:
        rt = runtime(protocol, url)
        conn = connect(base)
        fetch, checkpoints = python_prefetch(conn, rt)
        data = serialize_fetch(fetch)
        captured = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
        conn.close()
    before = dump(base)
    success = copy_book(base, tmp_path / "success.sqlite")
    assert "result" in rust_apply(_binary(), success, data, captured, _env(tmp_path))
    written = _written_tables(before, dump(success))
    assert {"transactions", "chain_observation_provenance", "freshness_source_states", "wallets",
            "wallet_utxos"} <= set(written)
    injected = 0
    for stage in STAGES:
        book = copy_book(base, tmp_path / f"stage-{stage}.sqlite")
        result = rust_apply(_binary(), book, data, captured, _env(tmp_path), fail_stage=stage)
        assert result["error"]["code"] == "injected_failure"
        assert dump(book) == before, stage
        injected += 1
        if stage in {"begin", "commit"}:
            continue
        python = copy_book(base, tmp_path / f"python-{stage}.sqlite")
        conn = connect(python)
        outcome = python_apply(conn, rt, deserialize_fetch(data), checkpoints, fail_stage=stage)
        conn.close()
        assert "error" in outcome
        assert dump(python) == before, stage
    for table in written:
        for operation in ("INSERT", "UPDATE", "DELETE"):
            book = copy_book(base, tmp_path / f"{table}-{operation}.sqlite")
            conn = sqlite3.connect(book)
            conn.execute(f'CREATE TRIGGER injected_{operation.lower()} BEFORE {operation} ON "{table}" '
                         f"BEGIN SELECT RAISE(ABORT, 'injected write failure'); END")
            conn.commit()
            conn.close()
            trapped = dump(book)
            result = rust_apply(_binary(), book, data, captured, _env(tmp_path))
            if "error" in result:
                injected += 1
                assert dump(book) == trapped, (table, operation)
            else:
                # This table/operation pair is not part of this publication.
                assert dump(book) == dump(success), (table, operation)
    assert injected >= len(STAGES) + len(written)


def test_publication_refuses_unsupported_shapes_without_writing(tmp_path):
    config, chain, _ = _fixture("receive-change")
    base = tmp_path / "base.sqlite"
    create_book(base, config)
    with serve(chain, "electrum") as url:
        conn = connect(base)
        fetch, _ = python_prefetch(conn, runtime("electrum", url))
        data = serialize_fetch(fetch)
        captured = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
        conn.close()
    cases = {
        # A better owner of the prepared targets appeared after preparation.
        "new-owner": ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                      "SELECT 'b','w','p','Alpha','descriptor',config_json,'fixed' FROM wallets WHERE id='a'",
                      "source_overlap_retry"),
        "stale-network": ("INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,"
                          "domains_json,acknowledgements_json,revision,inventory_digest,created_at) "
                          "VALUES('p','env','regtest',NULL,'[]','[]',1,'d','fixed')", None),
    }
    for name, (sql, code) in cases.items():
        book = copy_book(base, tmp_path / f"{name}.sqlite")
        conn = connect(book)
        conn.executescript(sql)
        conn.commit()
        conn.close()
        before = dump(book)
        conn = connect(book)
        current = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
        conn.close()
        result = rust_apply(_binary(), book, data, current, _env(tmp_path))
        assert "error" in result, name
        if code:
            assert result["error"]["code"] == code, (name, result)
        assert dump(book) == before, name
    # A changed configuration is a stale preparation even when still supported.
    book = copy_book(base, tmp_path / "fence.sqlite")
    result = rust_apply(_binary(), book, data, captured.replace('"gap_limit": 3', '"gap_limit": 4'), _env(tmp_path))
    assert result["error"]["code"] == "stale_context"
    # A descriptor wallet with BTCPay provenance is no longer refused: 7a95da76c
    # enriches it natively. Rust publishes and its book matches Python's
    # publication of the same prepared fetch on the same book, byte for byte.
    python_book = copy_book(base, tmp_path / "enrichment-python.sqlite")
    rust_book = copy_book(base, tmp_path / "enrichment-rust.sqlite")
    enrichment = "UPDATE wallets SET config_json=json_set(config_json,'$.btcpay_provenance',json('[]'))"
    with serve(chain, "electrum") as url:
        _, expected = _round(tmp_path, "electrum", url, python_book, rust_book, sql=enrichment)
    assert "result" in expected, expected
    assert dump(rust_book)["transactions"], "the enriched wallet must publish its transactions"


def test_wallet_publication_vectors_still_match_python(tmp_path):
    """The offline Rust replay consumes Python's current preparation and books."""
    import subprocess
    import sys
    script = Path(__file__).resolve().parents[1] / "crates/wallets/scripts/generate_wallet_publication_vectors.py"
    result = subprocess.run([sys.executable, str(script), "--check"], capture_output=True, timeout=900,
                            env={**_env(tmp_path), "PYTHONPATH": guarded_pythonpath()})
    assert result.returncode == 0, result.stderr.decode()

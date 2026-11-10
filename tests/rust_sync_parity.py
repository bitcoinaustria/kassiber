"""Helpers for full-book Bitcoin wallet-sync parity between Python and Rust.

A prepared fetch crosses the boundary as JSON (the contract the Rust
coordinator consumes). Python applies it through the unchanged handler under
deterministic `uuid4`/`now_iso` seams; Rust applies the identical JSON with
the same sequences. Books are plaintext disposable SQLite files.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict, replace
from decimal import Decimal
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "crates/wallets/scripts"))
from publication_books import new_book, snapshot, timestamp  # noqa: E402

from kassiber.cli import handlers  # noqa: E402
from kassiber.core import (  # noqa: E402
    filed_report_chain,
    freshness,
    imports,
    output_inventory,
    ownership_policy_epochs,
    sync as core_sync,
)
from kassiber.core.chain_observer import provenance, store  # noqa: E402
from kassiber.core.chain_observer.bdk import BdkObserver  # noqa: E402
from kassiber.core.chain_observer.contract import PreparedObserverUpdate  # noqa: E402
from kassiber.core.chain_observer.identity import ObserverIdentity  # noqa: E402
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION  # noqa: E402

CLOCK_MODULES = (imports, provenance, filed_report_chain, output_inventory, ownership_policy_epochs, store,
                 freshness, handlers)

# A sync's `elapsed_ms` is its real wall time on both sides (Python's
# time.monotonic span), in envelopes and in the outcome a freshness job
# stores, so exact comparisons check it and then fix it.
ELAPSED_MS_FIXED = 0
_ELAPSED_MS_BYTES = re.compile(rb'"elapsed_ms": ?([^,}\]\s]*)')
_ELAPSED_MS_TEXT = re.compile(r'"elapsed_ms": ?([^,}\]\s]*)')


def _checked_elapsed_ms(value):
    if type(value) is not int or value < 0:
        raise AssertionError(f"elapsed_ms must be a non-negative integer, not {value!r}")
    return ELAPSED_MS_FIXED


def fixed_elapsed_ms(value):
    """`value` with every `elapsed_ms` key checked to be a non-negative int
    and replaced by ELAPSED_MS_FIXED; nothing else changes."""
    if isinstance(value, dict):
        return {key: _checked_elapsed_ms(item) if key == "elapsed_ms" else fixed_elapsed_ms(item)
                for key, item in value.items()}
    if isinstance(value, list):
        return [fixed_elapsed_ms(item) for item in value]
    return value


def _fixed_raw(match):
    try:
        value = json.loads(match.group(1))
    except ValueError:
        value = match.group(1)
    _checked_elapsed_ms(value)
    head = match.group(0)[: match.start(1) - match.start(0)]
    return head + (str(ELAPSED_MS_FIXED).encode() if isinstance(head, bytes) else str(ELAPSED_MS_FIXED))


def fixed_elapsed_ms_bytes(output: bytes) -> bytes:
    """`fixed_elapsed_ms` for raw JSON output compared byte for byte."""
    return _ELAPSED_MS_BYTES.sub(_fixed_raw, output)


def fixed_elapsed_ms_text(text: str) -> str:
    """`fixed_elapsed_ms` inside stored JSON text (a column value); every
    other character is left as stored."""
    return _ELAPSED_MS_TEXT.sub(_fixed_raw, text)


def create_book(path: Path, config: dict, *, label="Cold", extra_sql="") -> None:
    memory = new_book()
    disk = sqlite3.connect(path)
    memory.backup(disk)
    disk.close()
    conn = connect(path)
    conn.executescript("""
        INSERT INTO workspaces(id,label,created_at) VALUES('w','ws','fixed');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','book','EUR','fixed');
    """)
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                 "VALUES('a','w','p',?,'descriptor',?,'fixed')", (label, json.dumps(config)))
    if extra_sql:
        conn.executescript(extra_sql)
    conn.commit()
    conn.close()


def connect(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def runtime(protocol: str, url: str) -> dict:
    return {"backends": {"fixture": {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2, "source": "test"}},
            "default_backend": "fixture", "env_file": "test.env"}


def _plain(value):
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, dict):
        return {str(k): _plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_plain(v) for v in value]
    return value


def serialize_fetch(fetch) -> dict:
    """JSON contract for one prepared `WalletBackendFetch`."""
    if isinstance(fetch, BaseException):
        return {"error": {"code": getattr(fetch, "code", type(fetch).__name__), "message": str(fetch)}}
    state = fetch.sync_state
    plan = getattr(state, "descriptor_plan", None) if state is not None else None
    return _plain({
        "backend": dict(fetch.backend),
        "kind": fetch.kind,
        "force_full": fetch.force_full,
        "authoritative_chain_observer": fetch.authoritative_chain_observer,
        "book_environment_id": fetch.book_environment_id,
        "skip_outcome": dict(fetch.skip_outcome) if fetch.skip_outcome is not None else None,
        "sync_state": None if state is None else {
            "chain": state.chain, "network": state.network, "target_count": len(state.targets),
            "gap_limit": getattr(plan, "gap_limit", None), "descriptor_kind": getattr(plan, "kind", None),
            "policy_asset_id": state.policy_asset_id, "checkpoint": dict(state.checkpoint or {}),
            # Publication re-checks the targets' ownership against the book.
            "targets": [{key: target.get(key) for key in ("script_pubkey", "branch_index", "address_index", "address")}
                        for target in state.targets],
        },
        "normalized_records": [dict(r) for r in fetch.normalized_records],
        "adapter_meta": dict(fetch.adapter_meta),
        "observer_updates": [
            {"identity": {**asdict(u.identity), "branch_keys": list(u.identity.branch_keys)},
             "expected_epoch": u.expected_epoch, "update": dict(u.update)}
            for u in fetch.observer_updates
        ],
        "elapsed_ms": 0,
    })


class _ReplayObserver:
    """The BDK observer's `apply` is a pure repackaging of the prepared update."""

    apply = BdkObserver.apply

    def discard(self):
        pass


def deserialize_fetch(data: dict):
    if "error" in data:
        raise AssertionError("cannot replay a failed preparation")
    state = data["sync_state"]
    sync_state = None if state is None else core_sync.WalletSyncState(
        chain=state["chain"], network=state["network"],
        descriptor_plan=SimpleNamespace(kind=state["descriptor_kind"], gap_limit=state["gap_limit"]),
        policy_asset_id=state["policy_asset_id"],
        targets=[dict(t) for t in state["targets"]] if "targets" in state else [{}] * state["target_count"],
        tracked_scripts={},
        history_cache={}, checkpoint=dict(state["checkpoint"]))
    updates = tuple(
        PreparedObserverUpdate(identity=ObserverIdentity(**{**u["identity"], "branch_keys": tuple(u["identity"]["branch_keys"])}),
                               update=u["update"], prior_state=None, expected_epoch=u["expected_epoch"],
                               _observer=_ReplayObserver())
        for u in data["observer_updates"])
    return core_sync.WalletBackendFetch(
        backend=data["backend"], sync_state=sync_state, normalized_records=tuple(data["normalized_records"]),
        adapter_meta=dict(data["adapter_meta"]), kind=data["kind"], started=time.monotonic(),
        force_full=data["force_full"], skip_outcome=data["skip_outcome"], observer_updates=updates,
        authoritative_chain_observer=data["authoritative_chain_observer"],
        book_environment_id=data["book_environment_id"])


def stored_checkpoints(conn, wallet_id="a"):
    wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet_id,)).fetchone()
    return handlers._stored_wallet_freshness_checkpoints(conn, "p", [wallet])


def seed_reviewed_state(conn, *, clear_authority=False):
    """A current journal and saved-report dependencies must share sync's rollback.

    Use actual closed observations from the preceding round. Reusing this seed
    on a later round preserves the original saved dependency and its inbox.
    """
    conn.execute("UPDATE profiles SET last_processed_at='reviewed', "
                 "last_processed_input_version=journal_input_version, "
                 "last_processed_tx_count=(SELECT COUNT(*) FROM transactions "
                 "WHERE profile_id='p' AND excluded=0) WHERE id='p'")
    # Canonical freshness also needs the count and the calculation semantics;
    # stamp after the markers because the clearing trigger fires on them.
    conn.execute("UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
                 (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
    conn.execute("INSERT OR IGNORE INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,"
                 "period_start_year,period_end_year,content_sha256,created_at) "
                 "VALUES('saved','w','p','summary.pdf','filed',2023,2023,?,'fixed')", ("0" * 64,))
    for row in conn.execute("SELECT id,external_id FROM transactions WHERE wallet_id='a' AND excluded=0").fetchall():
        dependency = {"snapshot_id": "saved", "profile_id": "p", "transaction_id": row[0], "wallet_id": "a",
                      "chain": "bitcoin", "network": "main", "txid": row[1]}
        observed = filed_report_chain._current(conn, dependency)
        conn.execute("INSERT OR IGNORE INTO filed_report_chain_dependencies(snapshot_id,profile_id,transaction_id,"
                     "wallet_id,chain,network,txid,observation_json) VALUES(?,?,?,?,?,?,?,?)",
                     (*dependency.values(), filed_report_chain._json(observed)))
    if clear_authority:
        # A legacy row has no closed provenance yet. Restoring authority on an
        # otherwise unchanged row must still invalidate a current journal.
        conn.execute("DELETE FROM chain_observation_provenance")
    conn.commit()


def python_prefetch(conn, runtime_config, *, force_full=False, wallet_id="a"):
    """Exactly the preparation `handlers.sync_wallet` performs for one wallet."""
    profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet_id,)).fetchone()
    checkpoints = stored_checkpoints(conn, wallet_id)
    if force_full:
        checkpoints = {key: kept for key, value in checkpoints.items()
                       if (kept := core_sync.full_scan_checkpoint(value))}
    hooks = handlers._wallet_sync_hooks(commit=False)
    prefetched = handlers._prefetch_chain_wallets(conn, runtime_config, profile, [wallet], hooks,
                                                  freshness_checkpoints=checkpoints, force_full=force_full)
    return prefetched[wallet_id], checkpoints


@contextmanager
def deterministic_sources(prefix="id", now=0):
    counter = {"id": 0, "now": now}

    def next_id():
        counter["id"] += 1
        return f"{prefix}-{counter['id']:04d}"

    def next_now():
        counter["now"] += 1
        return timestamp(counter["now"])

    savepoint = SimpleNamespace(uuid4=lambda: SimpleNamespace(hex="fixture"))
    patches = [patch("uuid.uuid4", side_effect=next_id), patch.object(handlers, "uuid", savepoint)]
    patches += [patch.object(module, "now_iso", side_effect=next_now) for module in CLOCK_MODULES]
    for item in patches:
        item.start()
    try:
        yield counter
    finally:
        for item in reversed(patches):
            item.stop()


def python_apply(conn, runtime_config, fetch, checkpoints, *, force_full=False, wallet_id="a", fail_stage=None,
                 prefix="id", now=0):
    profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet_id,)).fetchone()
    hooks = handlers._wallet_sync_hooks(commit=False)
    if fail_stage is not None:
        def fail(stage):
            if stage == fail_stage:
                raise RuntimeError(stage)
        hooks = replace(hooks, after_apply_stage=fail)
    with deterministic_sources(prefix, now):
        try:
            return {"result": _plain(handlers._apply_wallet_sync_atomically(
                conn, runtime_config, profile, wallet, hooks, freshness_checkpoints=checkpoints,
                force_full=force_full, prefetched={wallet_id: fetch}))}
        except Exception as exc:  # The comparison records Python's refusal.
            return {"error": {"code": getattr(exc, "code", type(exc).__name__), "message": str(exc)}}


def rust_apply(binary: Path, book: Path, fetch: dict, captured_config: str, env, *, wallet_id="a", fail_stage=None,
               prefix="id", now=0):
    payload = {"book": str(book), "profile_id": "p", "wallet_id": wallet_id, "captured_config": captured_config,
               "fetch": fetch, "fail_stage": fail_stage, "prefix": prefix, "now": now}
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True, timeout=60,
                            env=env)
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    return json.loads(result.stdout)


def _fixed_stored_elapsed_ms(value):
    if isinstance(value, str):
        return fixed_elapsed_ms_text(value) if '"elapsed_ms"' in value else value
    if isinstance(value, dict):
        return {key: _fixed_stored_elapsed_ms(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_fixed_stored_elapsed_ms(item) for item in value]
    return value


def dump(path: Path) -> dict:
    """Every table's rows, with a stored outcome's wall-clock `elapsed_ms`
    checked and fixed (freshness_jobs.result_json)."""
    conn = connect(path)
    try:
        return _fixed_stored_elapsed_ms(json.loads(json.dumps(snapshot(conn), sort_keys=True)))
    finally:
        conn.close()


def dump_bytes(path: Path) -> bytes:
    """Canonical logical rows; preserve numeric types when comparing parity."""
    return (json.dumps(dump(path), sort_keys=True, ensure_ascii=False, allow_nan=False,
                       separators=(",", ":")) + "\n").encode("utf-8")


def copy_book(source: Path, target: Path) -> Path:
    shutil.copyfile(source, target)
    return target

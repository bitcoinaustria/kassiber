"""A BDK refresh re-emits only transactions whose record could have changed."""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from types import SimpleNamespace
from unittest import TestCase, main, mock

from kassiber.core.chain_observer.bdk import (
    RECORD_FINGERPRINT_VERSION,
    BdkObserver,
    bdk_branches_for_identity,
)
from kassiber.core.chain_observer.contract import ChainFacts, ObserverPrepareRequest
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.provenance import persist_chain_observation_provenance
from kassiber.core.chain_observer.store import CoveragePoint, StoredObserverState
from kassiber.core.imports import ImportCoordinatorHooks, insert_wallet_records
from kassiber.core.sync_backends import _current_observed_txids
from kassiber.db import open_db
from kassiber.time_utils import now_iso
from tests.test_bdk_observer import _descriptor_wallet

PARENT = "11" * 32
SPEND = "22" * 32
FUNDER = "33" * 32
OWNED_RECEIVE = "0014" + "aa" * 20
OWNED_CHANGE = "0014" + "bb" * 20
PAYEE = "0014" + "cc" * 20
FOREIGN = "0014" + "dd" * 20


class _Amount:
    def __init__(self, sats):
        self._sats = sats

    def to_sat(self):
        return self._sats


class _Script:
    def __init__(self, hex_script):
        self._bytes = bytes.fromhex(hex_script)

    def to_bytes(self):
        return self._bytes


def _txout(script, sats):
    return SimpleNamespace(value=_Amount(sats), script_pubkey=_Script(script))


class _Tx:
    def __init__(self, txid, inputs, outputs):
        self._txid = txid
        self._inputs = [
            SimpleNamespace(previous_output=SimpleNamespace(txid=prev, vout=vout), witness=[])
            for prev, vout in inputs
        ]
        self._outputs = [_txout(script, sats) for script, sats in outputs]

    def compute_txid(self):
        return self._txid

    def input(self):
        return self._inputs

    def output(self):
        return self._outputs


def _confirmed(height, block_hash):
    return SimpleNamespace(
        is_confirmed=lambda: True,
        confirmation_block_time=SimpleNamespace(
            block_id=SimpleNamespace(height=height, hash=block_hash),
            confirmation_time=1_700_000_000 + height,
        ),
    )


def _mempool():
    return SimpleNamespace(is_confirmed=lambda: False)


class _Wallet:
    def __init__(self, entries, tip=120):
        self.entries = entries
        self.tip = tip
        self.tx_details = mock.Mock(side_effect=lambda txid: SimpleNamespace(fee=_Amount(1_000)))

    def transactions(self):
        return [SimpleNamespace(transaction=tx, chain_position=position) for tx, position in self.entries]

    def latest_checkpoint(self):
        return SimpleNamespace(height=self.tip, hash="ee" * 32)

    def list_output(self):
        return []

    def list_unspent(self):
        return []


def _observer():
    wallet, plan = _descriptor_wallet()
    identity = identities_for_wallet(wallet, observer_kind="bdk")[0]
    return BdkObserver(
        identity=identity,
        backend={"name": "fulcrum", "kind": "electrum", "url": "ssl://example.invalid:50002"},
        branches=bdk_branches_for_identity(plan, identity),
        gap_limit=20,
    )


def _history(*, spend_position=None, extra=()):
    funder = _Tx(FUNDER, [], [(FOREIGN, 200_000)])
    parent = _Tx(PARENT, [(FUNDER, 0)], [(OWNED_RECEIVE, 100_000), (FOREIGN, 99_000)])
    spend = _Tx(SPEND, [(PARENT, 0)], [(PAYEE, 60_000), (OWNED_CHANGE, 39_000)])
    return [
        (parent, _confirmed(100, "a1" * 32)),
        (spend, spend_position or _confirmed(101, "a2" * 32)),
        *extra,
    ], [funder, parent, spend]


class RecordReuseTest(TestCase):
    def _facts(self, observer, entries, graph_txs, owned, **kwargs):
        wallet = _Wallet(entries)
        observer._persistence = SimpleNamespace(
            aggregate=SimpleNamespace(
                tx_graph_changeset=lambda: SimpleNamespace(txs=graph_txs, first_seen={}, txouts={})
            )
        )

        def target(_wallet, script):
            script_hex = script.to_bytes().hex()
            return {"script_pubkey": script_hex} if script_hex in owned else None

        coverage = CoveragePoint(branch_key="receive", scanned_to=0, highest_used=None, details={})
        with mock.patch.object(observer, "_target", side_effect=target), mock.patch.object(
            observer, "_coverage_point", return_value=coverage
        ):
            facts = observer._facts(wallet, None, **kwargs)
        return facts, wallet, dict(observer._record_fingerprints)

    def test_an_unchanged_confirmed_history_emits_no_records_and_skips_fee_lookups(self):
        observer = _observer()
        entries, graph = _history()
        owned = {OWNED_RECEIVE, OWNED_CHANGE}
        first, first_wallet, fingerprints = self._facts(observer, entries, graph, owned)
        self.assertEqual({record["txid"] for record in first.transaction_records}, {PARENT, SPEND})
        self.assertEqual(first_wallet.tx_details.call_count, 2)

        again, wallet, refreshed = self._facts(
            observer, entries, graph, owned,
            reusable_txids=frozenset({PARENT, SPEND}), prior_fingerprints=fingerprints,
        )

        self.assertEqual(again.transaction_records, ())
        self.assertEqual(wallet.tx_details.call_count, 0)
        self.assertEqual(observer._reused_records, 2)
        self.assertEqual(refreshed, fingerprints)
        # Retractions and outputs never depend on which records were emitted.
        self.assertEqual(again.freshness_checkpoint["canonical_txids"], sorted([PARENT, SPEND]))

    def test_records_carry_only_their_own_owned_scripts(self):
        observer = _observer()
        entries, graph = _history()
        facts, _wallet, _ = self._facts(observer, entries, graph, {OWNED_RECEIVE, OWNED_CHANGE})
        by_txid = {record["txid"]: json.loads(record["raw_json"]) for record in facts.transaction_records}
        self.assertEqual(by_txid[PARENT]["observer_owned_scripts"], [OWNED_RECEIVE])
        self.assertEqual(by_txid[SPEND]["observer_owned_scripts"], sorted([OWNED_RECEIVE, OWNED_CHANGE]))

    def test_a_row_that_no_longer_matches_its_proof_is_emitted_again(self):
        observer = _observer()
        entries, graph = _history()
        owned = {OWNED_RECEIVE, OWNED_CHANGE}
        _, _, fingerprints = self._facts(observer, entries, graph, owned)

        again, _, _ = self._facts(
            observer, entries, graph, owned,
            reusable_txids=frozenset({PARENT}), prior_fingerprints=fingerprints,
        )

        self.assertEqual([record["txid"] for record in again.transaction_records], [SPEND])

    def test_a_moved_block_is_emitted_again(self):
        observer = _observer()
        entries, graph = _history()
        owned = {OWNED_RECEIVE, OWNED_CHANGE}
        _, _, fingerprints = self._facts(observer, entries, graph, owned)
        reorged, _ = _history(spend_position=_confirmed(101, "f2" * 32))

        again, _, _ = self._facts(
            observer, reorged, graph, owned,
            reusable_txids=frozenset({PARENT, SPEND}), prior_fingerprints=fingerprints,
        )

        self.assertEqual([record["txid"] for record in again.transaction_records], [SPEND])

    def test_a_newly_recognised_owned_script_re_emits_the_transaction_it_appears_in(self):
        observer = _observer()
        entries, graph = _history()
        _, _, fingerprints = self._facts(observer, entries, graph, {OWNED_RECEIVE, OWNED_CHANGE})

        again, _, _ = self._facts(
            observer, entries, graph, {OWNED_RECEIVE, OWNED_CHANGE, PAYEE},
            reusable_txids=frozenset({PARENT, SPEND}), prior_fingerprints=fingerprints,
        )

        self.assertEqual([record["txid"] for record in again.transaction_records], [SPEND])

    def test_a_mempool_transaction_is_always_emitted(self):
        observer = _observer()
        entries, graph = _history(spend_position=_mempool())
        owned = {OWNED_RECEIVE, OWNED_CHANGE}
        _, _, fingerprints = self._facts(observer, entries, graph, owned)

        again, _, _ = self._facts(
            observer, entries, graph, owned,
            reusable_txids=frozenset({PARENT, SPEND}), prior_fingerprints=fingerprints,
        )

        self.assertEqual([record["txid"] for record in again.transaction_records], [SPEND])


class PrepareReuseGateTest(TestCase):
    def _prepare(self, *, payload, force_full=False, supplier=None, remote_matches=True):
        observer = _observer()
        observer._persistence = mock.Mock()
        observer._persistence.payload.return_value = {"schema_version": 1}
        native_wallet = mock.Mock()
        native_wallet.latest_checkpoint.return_value = SimpleNamespace(height=10, hash="ab" * 32)
        prior = StoredObserverState(identity=observer.identity, payload=payload, coverage=())
        facts = ChainFacts(freshness_checkpoint={"canonical_txids": [PARENT]})
        observer._record_fingerprints = {PARENT: "f" * 32}
        observer._reused_records = 1
        request = ObserverPrepareRequest(
            backend_name="fulcrum",
            backend_kind="electrum",
            force_full=force_full,
            options={} if supplier is None else {"current_record_txids": supplier},
        )
        with mock.patch.object(
            observer, "_wallet_from_state", return_value=(native_wallet, mock.Mock())
        ), mock.patch.object(observer, "_client"), mock.patch.object(
            observer, "_remote_block_hash", return_value=("ab" * 32 if remote_matches else "cd" * 32)
        ), mock.patch.object(observer, "_sync_revealed_horizon"), mock.patch.object(
            observer, "_full_scan"
        ), mock.patch.object(observer, "_reveal_scan_horizon"), mock.patch.object(
            observer, "_facts", return_value=facts
        ) as collect:
            prepared = observer.prepare(request, prior)
        return collect.call_args.kwargs, prepared

    def _payload(self, version=RECORD_FINGERPRINT_VERSION):
        return {
            "schema_version": 1,
            "bdk_changeset": {},
            "canonical_txids": [PARENT],
            "record_fingerprint_version": version,
            "record_fingerprints": {PARENT: "e" * 32},
        }

    def test_an_incremental_refresh_reuses_rows_that_still_match(self):
        supplier = mock.Mock(return_value={PARENT})
        kwargs, prepared = self._prepare(payload=self._payload(), supplier=supplier)
        self.assertEqual(kwargs["reusable_txids"], frozenset({PARENT}))
        self.assertEqual(kwargs["prior_fingerprints"], {PARENT: "e" * 32})
        self.assertEqual(prepared["state"]["record_fingerprints"], {PARENT: "f" * 32})
        self.assertEqual(prepared["state"]["record_fingerprint_version"], RECORD_FINGERPRINT_VERSION)
        self.assertEqual(prepared["facts"]["reused_records"], 1)

    def test_another_release_force_full_reorg_or_no_supplier_reuse_nothing(self):
        cases = {
            "release": dict(payload=self._payload(version="0:old")),
            "force_full": dict(payload=self._payload(), force_full=True),
            "reorg": dict(payload=self._payload(), remote_matches=False),
            "multi_family": dict(payload=self._payload(), supplier=None),
        }
        for name, options in cases.items():
            with self.subTest(name):
                supplier = mock.Mock(return_value={PARENT})
                options.setdefault("supplier", supplier)
                kwargs, _ = self._prepare(**options)
                self.assertEqual(kwargs["reusable_txids"], frozenset())
                supplier.assert_not_called()


class CurrentObservedTxidsTest(TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="kassiber-bdk-reuse-")
        self.addCleanup(temp.cleanup)
        self.conn = open_db(Path(temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)", (timestamp,))
        self.conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
                                 tax_long_term_days, gains_algorithm, created_at)
            VALUES('profile', 'ws', 'Profile', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
            VALUES('wallet', 'ws', 'profile', 'Cold', 'descriptor', '{}', ?)
            """,
            (timestamp,),
        )
        self.profile = self.conn.execute("SELECT * FROM profiles").fetchone()
        self.wallet = self.conn.execute("SELECT * FROM wallets").fetchone()

    def _observe(self, txid, *observer_ids):
        record = {
            "txid": txid,
            "occurred_at": "2026-01-01T00:00:00Z",
            "confirmed_at": "2026-01-01T00:00:00Z",
            "direction": "inbound",
            "asset": "BTC",
            "amount": "0.001",
            "fee": "0",
            "kind": "deposit",
            "raw_json": json.dumps({"txid": txid, "observer": "bdk", "vin": [], "vout": []}),
        }
        outcome = insert_wallet_records(
            self.conn, self.profile, self.wallet, [record], "backend:fulcrum",
            ImportCoordinatorHooks(ensure_tag_row=mock.Mock(), invalidate_journals=mock.Mock()),
            commit=False, authoritative_chain_observer=True,
        )
        persist_chain_observation_provenance(
            self.conn, self.profile, self.wallet,
            application_revision="apply", chain="bitcoin", network="main",
            entries=[{"external_id": txid, "asset": "BTC", "direction": "inbound",
                      "observer_ids": list(observer_ids), "observer_kinds": ["bdk"]}],
            resolved_records=outcome["_observer_resolved_records"],
        )

    def test_only_unmodified_rows_of_this_observer_are_reusable(self):
        edited, foreign, shared = "44" * 32, "55" * 32, "66" * 32
        self._observe(PARENT, "bdk:one")
        self._observe(edited, "bdk:one")
        self._observe(foreign, "bdk:two")
        # A former script-family union stays stale after the other family is gone.
        self._observe(shared, "bdk:one", "bdk:two")
        self.conn.execute(
            "UPDATE transactions SET amount = amount + 1 WHERE external_id = ?", (edited,)
        )

        self.assertEqual(_current_observed_txids(self.conn, "wallet", "bdk:one"), {PARENT})


if __name__ == "__main__":
    main()

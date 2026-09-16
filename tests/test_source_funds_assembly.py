import json
import sqlite3
import unittest

from kassiber.core.source_funds_assembly import build_owned_outpoint_index


class SourceFundsOwnedOutpointIndexTests(unittest.TestCase):
    def test_missing_legacy_network_uses_chain_default(self):
        txid = "10" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, asset TEXT,
                txid TEXT, vout INTEGER, amount INTEGER, branch_label TEXT,
                spent_by TEXT
            )
            """
        )
        conn.execute(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', 'wallet', 'bitcoin', 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (txid,),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(list(index), [("bitcoin", "main", txid, 0)])

    def test_identical_outpoints_on_different_networks_stay_separate(self):
        txid = "20" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'bitcoin', ?, 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (("main-wallet", "main", txid), ("regtest-wallet", "regtest", txid)),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(
            set(index),
            {("bitcoin", "main", txid, 0), ("bitcoin", "regtest", txid, 0)},
        )

    def test_liquid_requires_consensus_asset_identity(self):
        txid = "21" * 32
        asset_id = "ab" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT, raw_json TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'liquid', 'liquidv1', 'LBTC', ?, ?, 100,
                'receive', NULL, ?
            )
            """,
            (
                ("known", txid, 0, json.dumps({"asset_id": asset_id})),
                ("unknown", txid, 1, "{}"),
            ),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(set(index), {("liquid", "liquidv1", txid, 0)})
        self.assertEqual(index[("liquid", "liquidv1", txid, 0)]["asset_identity"], asset_id)


def _script(tag: str) -> str:
    return "0014" + (tag * 40)[:40]


def _authoritative(row: dict) -> dict:
    """Attach the closed observation commitment an authoritative apply writes."""
    from kassiber.core.chain_observer.provenance import (
        AUTHORITY_VERSION,
        canonical_graph_hash,
        canonical_observed_quantity_hash,
    )

    row = dict(row)
    row["observation_authority_version"] = AUTHORITY_VERSION
    row["observation_graph_hash"] = canonical_graph_hash(row["raw_json"])
    row["observation_quantity_hash"] = canonical_observed_quantity_hash(row)
    return row


def _parent_row(txid: str, amount_msat: int, *, wallet: str = "w", authoritative: bool = True) -> dict:
    raw = {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "observer_owned_scripts": [_script("a")],
        "vin": [],
        "vout": [{"n": 0, "scriptpubkey": _script("a"), "value": amount_msat // 1000}],
    }
    row = {
        "id": f"tx-{txid[:8]}",
        "wallet_id": wallet,
        "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
        "external_id": txid,
        "external_id_kind": "txid",
        "direction": "inbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": 0,
        "amount_includes_fee": 0,
        "occurred_at": "2026-01-01T00:00:00Z",
        "raw_json": json.dumps(raw, sort_keys=True),
    }
    return _authoritative(row) if authoritative else row


def _spend_row(txid: str, parents, amount_msat: int, fee_msat: int, *, wallet: str = "w",
               authoritative: bool = True, owned_scripts=None) -> dict:
    raw = {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "observer_owned_scripts": owned_scripts if owned_scripts is not None else [_script("a")],
        "vin": [
            {
                "txid": parent_txid,
                "vout": 0,
                "prevout": {"scriptpubkey": _script("a"), "value": value_msat // 1000},
            }
            for parent_txid, value_msat in parents
        ],
        "vout": [{"n": 0, "scriptpubkey": _script("f"), "value": amount_msat // 1000}],
    }
    row = {
        "id": f"tx-{txid[:8]}",
        "wallet_id": wallet,
        "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
        "external_id": txid,
        "external_id_kind": "txid",
        "direction": "outbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": fee_msat,
        "amount_includes_fee": 0,
        "occurred_at": "2026-02-01T00:00:00Z",
        "raw_json": json.dumps(raw, sort_keys=True),
    }
    return _authoritative(row) if authoritative else row


def _index(parents, *, wallet: str = "w"):
    return {
        ("bitcoin", "main", txid, 0): {
            "wallet_id": wallet, "amount_msat": value, "branch_label": "receive",
            "spent_by": "", "asset": "BTC", "asset_identity": "BTC", "ambiguous": False,
        }
        for txid, value in parents
    }


class ParentSpendDerivationTests(unittest.TestCase):
    """The shape a real consolidation spend has: inputs sum to amount + fee."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        self.parents = [("a1" * 32, 200_000_000), ("b2" * 32, 300_000_000), ("c3" * 32, 500_000_000)]
        self.amount = 999_900_000
        self.fee = 100_000
        self.spend = _spend_row("d4" * 32, self.parents, self.amount, self.fee)
        self.rows = [_parent_row(t, v) for t, v in self.parents] + [self.spend]

    def test_owned_inputs_become_funding_edges_that_exactly_cover_the_spend(self):
        pairs = self.derive(self.rows, _index(self.parents), skip_row=lambda row: False)

        self.assertEqual(len(pairs), 3)
        # Gross upstream demand is every owned input, including the fee.
        self.assertEqual(sum(p["from_allocation_msat"] for p in pairs), 1_000_000_000)
        # Allocations cover the target exactly, so the report cannot raise
        # ambiguous_allocation for a spend whose lineage is fully observed.
        self.assertEqual(sum(p["allocation_msat"] for p in pairs), self.amount)
        self.assertTrue(all(p["to_row"] is self.spend for p in pairs))

    def test_a_foreign_input_emits_nothing_rather_than_a_partial_history(self):
        """Partial cover would raise ambiguous_allocation, which has no fix-it path."""
        spend = _spend_row("d4" * 32, self.parents, self.amount, self.fee)
        raw = json.loads(spend["raw_json"])
        # One input pays from a script this wallet never watched.
        raw["vin"][1]["prevout"]["scriptpubkey"] = _script("f")
        spend["raw_json"] = json.dumps(raw, sort_keys=True)
        rows = [_parent_row(t, v) for t, v in self.parents] + [_authoritative(spend)]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])

    def test_lineage_resolves_without_any_utxo_inventory(self):
        """Inventory holds the CURRENT unspent set, so a wallet imported after
        these outputs were spent has no row for any of them. The spend's own
        attested scripts must be enough."""
        pairs = self.derive(self.rows, {}, skip_row=lambda row: False)

        self.assertEqual(len(pairs), 3)
        self.assertEqual(sum(p["allocation_msat"] for p in pairs), self.amount)

    def test_inventory_disagreeing_about_ownership_stops_the_spend(self):
        """Corroboration may veto; it may never be overruled."""
        conflicting = dict(_index(self.parents))
        key = ("bitcoin", "main", self.parents[0][0], 0)
        conflicting[key] = {**conflicting[key], "wallet_id": "other"}

        self.assertEqual(self.derive(self.rows, conflicting, skip_row=lambda row: False), [])

    def test_a_row_without_observation_authority_founds_no_lineage(self):
        """A hand-written vin array must never author provenance."""
        rows = [_parent_row(t, v) for t, v in self.parents] + [
            _spend_row("d4" * 32, self.parents, self.amount, self.fee, authoritative=False)
        ]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])

    def test_a_parent_without_observation_authority_founds_no_lineage(self):
        rows = [_parent_row(self.parents[0][0], self.parents[0][1], authoritative=False)] + [
            _parent_row(t, v) for t, v in self.parents[1:]
        ] + [self.spend]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])

    def test_inputs_owned_by_another_wallet_stay_a_custody_question(self):
        foreign = dict(_index(self.parents))
        foreign[("bitcoin", "main", self.parents[0][0], 0)] = {
            **foreign[("bitcoin", "main", self.parents[0][0], 0)], "wallet_id": "other",
        }

        self.assertEqual(self.derive(self.rows, foreign, skip_row=lambda row: False), [])

    def test_a_privacy_boundary_row_is_never_traversed(self):
        self.assertEqual(
            self.derive(self.rows, _index(self.parents), skip_row=lambda row: True), [],
        )

    def test_an_ambiguous_owned_outpoint_blocks_the_whole_spend(self):
        ambiguous = dict(_index(self.parents))
        ambiguous[("bitcoin", "main", self.parents[1][0], 0)] = {
            **ambiguous[("bitcoin", "main", self.parents[1][0], 0)], "ambiguous": True,
        }

        self.assertEqual(self.derive(self.rows, ambiguous, skip_row=lambda row: False), [])

    def test_a_truncated_graph_cannot_prove_complete_ownership(self):
        """Fewer parsed inputs than declared means the evidence is incomplete."""
        spend = _spend_row("d4" * 32, self.parents, self.amount, self.fee)
        raw = json.loads(spend["raw_json"])
        raw["vin"].append({"txid": "ee" * 32, "vout": 0})  # declared, unparseable prevout
        spend["raw_json"] = json.dumps(raw, sort_keys=True)
        spend = _authoritative(spend)
        rows = [_parent_row(t, v) for t, v in self.parents] + [spend]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])


class ChangePassthroughTests(unittest.TestCase):
    """A change output is never its own row, so lineage must pass through it.

    The observer stores one NET row per wallet per transaction: a same-wallet
    consolidation becomes an outbound `fee` row and change is netted out of a
    withdrawal's amount. Spending change therefore lands on an outbound parent.
    """

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        self.deposit_txid = "e1" * 32
        self.hop_txid = "e2" * 32
        self.spend_txid = "e3" * 32
        # Deposit 1,000,000 -> pay 400,000 out, keep 599,000 change, 1,000 fee.
        self.deposit = _parent_row(self.deposit_txid, 1_000_000_000)
        hop_raw = {
            "txid": self.hop_txid, "chain": "bitcoin", "network": "main",
            "observer_owned_scripts": [_script("a")],
            "vin": [{
                "txid": self.deposit_txid, "vout": 0,
                "prevout": {"scriptpubkey": _script("a"), "value": 1_000_000},
            }],
            "vout": [
                {"n": 0, "scriptpubkey": _script("f"), "value": 400_000},
                {"n": 1, "scriptpubkey": _script("a"), "value": 599_000},
            ],
        }
        self.hop = _authoritative({
            "id": "hop", "wallet_id": "w",
            "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
            "external_id": self.hop_txid, "external_id_kind": "txid",
            "direction": "outbound", "asset": "BTC",
            "amount": 400_000_000, "fee": 1_000_000, "amount_includes_fee": 0,
            "occurred_at": "2026-01-15T00:00:00Z",
            "raw_json": json.dumps(hop_raw, sort_keys=True),
        })
        # Now spend that change entirely.
        self.spend = _spend_row(
            self.spend_txid, [(self.hop_txid, 599_000_000)], 598_000_000, 1_000_000,
        )
        spend_raw = json.loads(self.spend["raw_json"])
        spend_raw["vin"][0]["vout"] = 1
        self.spend["raw_json"] = json.dumps(spend_raw, sort_keys=True)
        self.spend = _authoritative(self.spend)
        self.rows = [self.deposit, self.hop, self.spend]

    def test_spending_change_reaches_the_deposit_behind_it(self):
        pairs = self.derive(self.rows, {}, skip_row=lambda row: False)
        funding = [p for p in pairs if p["to_row"]["id"] == self.spend["id"]]

        self.assertEqual(len(funding), 1)
        # The edge lands on the deposit, not on the intermediate payment.
        self.assertEqual(funding[0]["from_row"]["id"], self.deposit["id"])
        self.assertEqual(funding[0]["allocation_msat"], 598_000_000)

    def test_an_unresolvable_hop_emits_nothing(self):
        """Drop the deposit: the change can no longer be traced to a root."""
        pairs = self.derive([self.hop, self.spend], {}, skip_row=lambda row: False)

        self.assertEqual([p for p in pairs if p["to_row"]["id"] == self.spend["id"]], [])

    def test_a_privacy_hop_is_not_passed_through(self):
        pairs = self.derive(
            self.rows, {}, skip_row=lambda row: row["id"] == "hop",
        )

        self.assertEqual([p for p in pairs if p["to_row"]["id"] == self.spend["id"]], [])

    def test_an_ownership_contradiction_one_hop_deep_still_vetoes(self):
        """The veto has to apply at every hop, or it is not a veto.

        Astra's finding: inventory was consulted only on the spend's own
        inputs, so a deposit that inventory says belongs to another wallet was
        still adopted through the intermediate payment.
        """
        foreign = {
            ("bitcoin", "main", self.deposit_txid, 0): {
                "wallet_id": "someone-else", "amount_msat": 1_000_000_000,
                "branch_label": "receive", "spent_by": "", "asset": "BTC",
                "asset_identity": "BTC", "ambiguous": False,
            },
        }

        pairs = self.derive(self.rows, foreign, skip_row=lambda row: False)

        self.assertEqual([p for p in pairs if p["to_row"]["id"] == self.spend["id"]], [])

    def test_an_ambiguous_outpoint_one_hop_deep_still_vetoes(self):
        ambiguous = {
            ("bitcoin", "main", self.deposit_txid, 0): {
                "wallet_id": "w", "amount_msat": 1_000_000_000,
                "branch_label": "receive", "spent_by": "", "asset": "BTC",
                "asset_identity": "BTC", "ambiguous": True,
            },
        }

        pairs = self.derive(self.rows, ambiguous, skip_row=lambda row: False)

        self.assertEqual([p for p in pairs if p["to_row"]["id"] == self.spend["id"]], [])

    def test_a_passthrough_edge_discloses_its_hops_and_attribution(self):
        pairs = self.derive(self.rows, {}, skip_row=lambda row: False)
        funding = [p for p in pairs if p["to_row"]["id"] == self.spend["id"]]

        self.assertEqual(len(funding), 1)
        explanation = funding[0]["explanation"]
        # It must not claim the deposit created this spend's inputs; it did not.
        self.assertNotIn("include outputs created by", explanation)
        self.assertIn("intermediate transactions", explanation)
        self.assertIn("proportion", explanation)


class FanInBoundsTests(unittest.TestCase):
    """A wallet that splits and recombines must not cost combinatorial work."""

    def _book(self, hops, width):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        script = _script("a")

        def out_row(rid, txid, vin, n_out, occurred, amount):
            raw = {
                "txid": txid, "chain": "bitcoin", "network": "main",
                "observer_owned_scripts": [script], "vin": vin,
                "vout": [
                    {"n": i, "scriptpubkey": script, "value": 1_000_000}
                    for i in range(n_out)
                ],
            }
            return _authoritative({
                "id": rid, "wallet_id": "w",
                "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
                "external_id": txid, "external_id_kind": "txid",
                "direction": "outbound", "asset": "BTC", "amount": amount,
                "fee": 1_000_000, "amount_includes_fee": 0, "occurred_at": occurred,
                "raw_json": json.dumps(raw, sort_keys=True),
            })

        # Note: 00..00 is the coinbase sentinel and is refused, so start at 01.
        deposits = [(f"{i + 1:02x}" * 32, 1_000_000) for i in range(width)]
        rows = [_parent_row(txid, sats * 1000) for txid, sats in deposits]
        prev = [(txid, 0) for txid, _sats in deposits]
        for hop in range(hops):
            txid = f"{(0xa0 + hop):02x}" * 32
            vin = [
                {"txid": t, "vout": v, "prevout": {"scriptpubkey": script, "value": 1_000_000}}
                for t, v in prev
            ]
            rows.append(out_row(f"hop{hop}", txid, vin, width, f"2026-01-{hop + 2:02d}T00:00:00Z", 0))
            prev = [(txid, i) for i in range(width)]
        spend_value = width * 1_000_000 * 1000 - 1_000_000
        rows.append(out_row(
            "final", "ff" * 32,
            [{"txid": t, "vout": v, "prevout": {"scriptpubkey": script, "value": 1_000_000}}
             for t, v in prev],
            1, "2026-02-01T00:00:00Z", spend_value,
        ))
        return rows, spend_value

    def test_repeated_ancestors_resolve_once_and_stay_exact(self):
        rows, spend_value = self._book(hops=4, width=10)

        pairs = self.derive(rows, {}, skip_row=lambda row: False)
        funding = [p for p in pairs if p["to_row"]["id"] == "final"]

        # One edge per deposit, not one per path that reaches it.
        self.assertEqual(len(funding), 10)
        self.assertEqual(len({p["from_row"]["id"] for p in funding}), 10)
        self.assertEqual(sum(p["allocation_msat"] for p in funding), spend_value)

    def test_a_graph_beyond_the_work_budget_fails_closed(self):
        from kassiber.core import source_funds_assembly

        rows, _spend_value = self._book(hops=4, width=10)
        original = source_funds_assembly._MAX_ANCESTOR_RESOLUTIONS_PER_TARGET
        source_funds_assembly._MAX_ANCESTOR_RESOLUTIONS_PER_TARGET = 2
        try:
            pairs = self.derive(rows, {}, skip_row=lambda row: False)
        finally:
            source_funds_assembly._MAX_ANCESTOR_RESOLUTIONS_PER_TARGET = original

        # Exhausting the budget emits nothing rather than a partial history.
        self.assertEqual([p for p in pairs if p["to_row"]["id"] == "final"], [])


class PassthroughWeightTests(unittest.TestCase):
    """Value carried through a hop is split by what each input contributed."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        script = _script("a")
        self.big, self.small = "b1" * 32, "b2" * 32
        hop_txid, spend_txid = "b3" * 32, "b4" * 32
        # 750,000 and 250,000 in; one 999,000 change output out.
        hop_raw = {
            "txid": hop_txid, "chain": "bitcoin", "network": "main",
            "observer_owned_scripts": [script],
            "vin": [
                {"txid": self.big, "vout": 0,
                 "prevout": {"scriptpubkey": script, "value": 750_000}},
                {"txid": self.small, "vout": 0,
                 "prevout": {"scriptpubkey": script, "value": 250_000}},
            ],
            "vout": [{"n": 0, "scriptpubkey": script, "value": 999_000}],
        }
        hop = _authoritative({
            "id": "hop", "wallet_id": "w",
            "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
            "external_id": hop_txid, "external_id_kind": "txid",
            "direction": "outbound", "asset": "BTC", "amount": 0,
            "fee": 1_000_000, "amount_includes_fee": 0,
            "occurred_at": "2026-01-15T00:00:00Z",
            "raw_json": json.dumps(hop_raw, sort_keys=True),
        })
        spend = _spend_row(spend_txid, [(hop_txid, 999_000_000)], 998_000_000, 1_000_000)
        self.rows = [
            _parent_row(self.big, 750_000_000),
            _parent_row(self.small, 250_000_000),
            hop,
            spend,
        ]

    def test_each_ancestor_carries_its_share_and_the_total_is_exact(self):
        pairs = self.derive(self.rows, {}, skip_row=lambda row: False)
        funding = {p["from_row"]["external_id"]: p["allocation_msat"] for p in pairs
                   if p["to_row"]["id"] == "tx-b4b4b4b4"}

        self.assertEqual(set(funding), {self.big, self.small})
        # 3:1 in, so 3:1 out -- and the shares still sum to the spend exactly.
        self.assertEqual(sum(funding.values()), 998_000_000)
        self.assertGreater(funding[self.big], funding[self.small] * 2)


class TraversalBoundsTests(unittest.TestCase):
    """Depth and cycles are bounded, and both fail closed."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs

    def _chain(self, length):
        script = _script("a")
        deposit = "c0" * 32
        rows = [_parent_row(deposit, 1_000_000_000)]
        prev, value = deposit, 1_000_000
        for hop in range(length):
            txid = f"{(0xc1 + hop):02x}" * 32
            value -= 100
            raw = {
                "txid": txid, "chain": "bitcoin", "network": "main",
                "observer_owned_scripts": [script],
                "vin": [{"txid": prev, "vout": 0,
                         "prevout": {"scriptpubkey": script, "value": value + 100}}],
                "vout": [{"n": 0, "scriptpubkey": script, "value": value}],
            }
            rows.append(_authoritative({
                "id": f"hop{hop}", "wallet_id": "w",
                "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
                "external_id": txid, "external_id_kind": "txid",
                "direction": "outbound", "asset": "BTC", "amount": 0,
                "fee": 100_000, "amount_includes_fee": 0,
                "occurred_at": f"2026-01-{hop + 2:02d}T00:00:00Z",
                "raw_json": json.dumps(raw, sort_keys=True),
            }))
            prev = txid
        spend = _spend_row("cf" * 32, [(prev, value * 1000)], (value - 100) * 1000, 100_000)
        rows.append(spend)
        return rows

    def test_a_long_change_chain_still_resolves_to_its_deposit(self):
        """Length is not a reason to refuse history the book can prove.

        Chain length is deliberately uncapped: a depth limit made this
        order-dependent once resolution was memoised, and refusing a long but
        fully observed chain reports a misleading "no root source".
        """
        for length in (3, 12):
            with self.subTest(length=length):
                pairs = self.derive(self._chain(length), {}, skip_row=lambda row: False)

                funding = [p for p in pairs if p["to_row"]["id"] == "tx-cfcfcfcf"]
                self.assertEqual(len(funding), 1)
                self.assertEqual(funding[0]["from_row"]["id"], "tx-c0c0c0c0")

    def test_a_cycle_cannot_be_traversed(self):
        rows = self._chain(3)
        # Point the first hop back at a descendant of itself.
        hop0 = next(row for row in rows if row["id"] == "hop0")
        raw = json.loads(hop0["raw_json"])
        raw["vin"][0]["txid"] = "c3" * 32
        hop0["raw_json"] = json.dumps(raw, sort_keys=True)
        rows = [_authoritative(hop0) if row["id"] == "hop0" else row for row in rows]

        pairs = self.derive(rows, {}, skip_row=lambda row: False)

        self.assertEqual([p for p in pairs if p["to_row"]["id"] == "tx-cfcfcfcf"], [])


class WalletIsolationTests(unittest.TestCase):
    """One batched transaction pays several wallets; each gets only its own ancestry."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        sa, sb, ext = _script("a"), _script("b"), _script("f")
        pa, pb, batch, spend_a, spend_b = "a1" * 32, "b1" * 32, "cc" * 32, "da" * 32, "db" * 32

        def row(rid, wallet, txid, direction, amount, vin, vout, scripts):
            raw = {"txid": txid, "chain": "bitcoin", "network": "main",
                   "observer_owned_scripts": scripts, "vin": vin, "vout": vout}
            return _authoritative({
                "id": rid, "wallet_id": wallet,
                "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
                "external_id": txid, "external_id_kind": "txid", "direction": direction,
                "asset": "BTC", "amount": amount, "fee": 1000 if direction == "outbound" else 0,
                "amount_includes_fee": 0, "occurred_at": "2026-01-01T00:00:00Z",
                "raw_json": json.dumps(raw, sort_keys=True),
            })

        batch_vout = [{"n": 0, "scriptpubkey": sa, "value": 499_000},
                      {"n": 1, "scriptpubkey": sb, "value": 699_000}]
        self.rows = [
            row("parent-A", "A", pa, "inbound", 500_000_000, [], [{"n": 0, "scriptpubkey": sa, "value": 500_000}], [sa]),
            row("parent-B", "B", pb, "inbound", 700_000_000, [], [{"n": 0, "scriptpubkey": sb, "value": 700_000}], [sb]),
            # Each wallet stores its own NET leg of the shared batch transaction.
            row("batch-A", "A", batch, "inbound", 499_000_000,
                [{"txid": pa, "vout": 0, "prevout": {"scriptpubkey": sa, "value": 500_000}}], batch_vout, [sa]),
            row("batch-B", "B", batch, "inbound", 699_000_000,
                [{"txid": pb, "vout": 0, "prevout": {"scriptpubkey": sb, "value": 700_000}}], batch_vout, [sb]),
            row("spend-A", "A", spend_a, "outbound", 498_000_000,
                [{"txid": batch, "vout": 0, "prevout": {"scriptpubkey": sa, "value": 499_000}}],
                [{"n": 0, "scriptpubkey": ext, "value": 498_000}], [sa]),
            row("spend-B", "B", spend_b, "outbound", 698_000_000,
                [{"txid": batch, "vout": 1, "prevout": {"scriptpubkey": sb, "value": 699_000}}],
                [{"n": 0, "scriptpubkey": ext, "value": 698_000}], [sb]),
        ]

    def _edges(self, rows):
        return {(p["from_row"]["id"], p["to_row"]["id"]) for p in
                self.derive(rows, {}, skip_row=lambda row: False)}

    def test_each_wallet_resolves_only_its_own_parent_in_either_order(self):
        """Astra's P1: a txid-only memo handed wallet B the ancestry cached for A."""
        expected = {("batch-A", "spend-A"), ("batch-B", "spend-B")}
        self.assertEqual(self._edges(self.rows), expected)
        self.assertEqual(self._edges(list(reversed(self.rows))), expected)

    def test_no_edge_ever_crosses_a_wallet(self):
        for rows in (self.rows, list(reversed(self.rows))):
            for src, dst in self._edges(rows):
                a = next(r for r in rows if r["id"] == src)["wallet_id"]
                b = next(r for r in rows if r["id"] == dst)["wallet_id"]
                self.assertEqual(a, b, f"cross-wallet edge {src} -> {dst}")


class OrderAndScaleTests(unittest.TestCase):
    """Row order must not change the answer, and size must not crash it."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs

    def _long_chain(self, hops):
        script = _script("a")
        deposit = "01" * 32
        rows = [_parent_row(deposit, 10_000_000_000)]
        prev, value = deposit, 10_000_000
        for i in range(hops):
            txid = f"{i + 2:064x}"
            value -= 1
            raw = {"txid": txid, "chain": "bitcoin", "network": "main",
                   "observer_owned_scripts": [script],
                   "vin": [{"txid": prev, "vout": 0, "prevout": {"scriptpubkey": script, "value": value + 1}}],
                   "vout": [{"n": 0, "scriptpubkey": script, "value": value}]}
            rows.append(_authoritative({
                "id": f"h{i}", "wallet_id": "w",
                "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
                "external_id": txid, "external_id_kind": "txid", "direction": "outbound",
                "asset": "BTC", "amount": 0, "fee": 1000, "amount_includes_fee": 0,
                # Same-block timestamps: chronology cannot be used to order the walk.
                "occurred_at": "2026-01-01T00:00:00Z", "raw_json": json.dumps(raw, sort_keys=True),
            }))
            prev = txid
        rows.append(_spend_row("ff" * 32, [(prev, value * 1000)], (value - 1) * 1000, 1000))
        return rows

    def test_a_1101_hop_chain_resolves_in_both_row_orders(self):
        """Recursion overflowed on the reversed order before the budget was reached."""
        rows = self._long_chain(1101)
        for ordered in (rows, list(reversed(rows))):
            funding = [p for p in self.derive(ordered, {}, skip_row=lambda row: False)
                       if p["to_row"]["id"] == "tx-ffffffff"]
            self.assertEqual(len(funding), 1)
            self.assertEqual(funding[0]["from_row"]["id"], "tx-01010101")

    def test_unrelated_spends_cannot_exhaust_a_targets_budget(self):
        """A pass-wide budget let 5,001 unrelated pairs starve a valid target listed last."""
        rows = []
        for i in range(5001):
            parent, spend = f"{i + 10:064x}", f"{i + 20000:064x}"
            rows.append(_parent_row(parent, 1_000_000_000))
            rows.append(_spend_row(spend, [(parent, 1_000_000_000)], 999_000_000, 1_000_000))
        target_parent, target_spend = "ee" * 32, "ef" * 32
        rows += [_parent_row(target_parent, 1_000_000_000),
                 _spend_row(target_spend, [(target_parent, 1_000_000_000)], 999_000_000, 1_000_000)]

        pairs = self.derive(rows, {}, skip_row=lambda row: False)

        self.assertTrue(any(p["to_row"]["external_id"] == target_spend for p in pairs))
        # And every one of the unrelated pairs resolved too: none starved another.
        self.assertEqual(len(pairs), 5002)

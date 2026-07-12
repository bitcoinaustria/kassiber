from __future__ import annotations

import json
import hashlib
import tempfile
import unittest
from pathlib import Path

from kassiber.core import samourai as core_samourai
from kassiber.core.engines import TaxEngineLedgerInputs, build_tax_engine
from kassiber.core.output_inventory import (
    list_wallet_output_inventory,
    update_wallet_output_inventory,
)
from kassiber.core.source_funds import SourceFundsHooks, suggest_links
from kassiber.core.sync import WalletSyncState
from kassiber.core.tax_events import normalize_tax_asset_inputs
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.wallet_descriptors import load_descriptor_plan


NOW = "2026-06-05T00:00:00Z"
SAMOURAI_FIXTURE_FINGERPRINT = "5a3469b6"
SAMOURAI_FIXTURE_XPUBS = {
    "m/47'/0'/0'": "xpub6DT7zz1QcpwVA2eiCibFCYZUKGYU84h8BpLV7fPbV8gtxWocwx4TRQaLZds4xBw5g1THnWvycLBYgTXPnPLD3zKFqLEXGBxvmamio6Md7Ns",
    "m/84'/0'/0'": "xpub6DEHh42YXj7gdKbP1zxehEzQt47iW2AuUHYsqGaRxtkEGrK3bCYh2bsw1H6WUW26k9TBdQoe6gZ8ydoAP5eGAC2fDJGmFkwXcgv5feY9N7p",
    "m/84'/0'/2147483644'": "xpub6DEHh42gsPeefAtjfCSnFzxgQ5BoRfUgML1qUDGFazD4CK5u7tpQdeyu7ERMBEZnJzNvJMQFzBgukyvEgYsApLeZNCcaVhqVHjqRaHRoCfm",
    "m/84'/0'/2147483645'": "xpub6DEHh42gsPeegDmb1v3wdjj9SdJic2JKwH8FwnTCjBp4CpBmmrwPaeo9ahHZKfMBvDd68WqGjpH4guTQSrYfRomKdgKXJkpjBU5gcgC2sHY",
    "m/84'/0'/2147483646'": "xpub6DEHh42gsPeekNJnrepkqGHTzxLYdBTmYXSDoXViqMApDuyfXcHhLV7ikXGisTysVMdRTXdNbVySgXmdbWYrQ1CestiSt49WWKwqM7GDbAD",
}


def _fixture_xpub(path: str) -> str:
    return SAMOURAI_FIXTURE_XPUBS[path]


def _fixture_descriptor(path: str, branch: int) -> str:
    return (
        f"wpkh([{SAMOURAI_FIXTURE_FINGERPRINT}/{path[2:]}]"
        f"{_fixture_xpub(path)}/{branch}/*)"
    )


def _seed_book(conn):
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        ("ws-1", "Main", NOW),
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        ("profile-1", "ws-1", "Default", "EUR", "generic", 365, "FIFO", NOW),
    )
    conn.execute(
        """
        INSERT INTO accounts(
            id, workspace_id, profile_id, code, label, account_type, asset, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        ("acct-1", "ws-1", "profile-1", "treasury", "Treasury", "asset", "BTC", NOW),
    )
    conn.commit()


def _wallet_config(section: str, group_id: str = "samourai-group") -> str:
    return json.dumps(
        {
            "chain": "bitcoin",
            "network": "main",
            "samourai": {
                "role": "child",
                "group_id": group_id,
                "section": section,
                "privacy_boundary": section in {"premix", "postmix", "badbank"},
                "whirlpool": section in {"premix", "postmix", "badbank"},
                "minimum_mix_count": 1 if section == "postmix" else None,
                "mix_count_confidence": "minimum" if section == "postmix" else None,
            },
        },
        sort_keys=True,
    )


def _tax_row(
    tx_id: str,
    section: str,
    direction: str,
    *,
    external_id: str,
    amount: int = 100_000_000,
    fee: int | None = None,
    fiat_rate: int | None = None,
):
    physical_txid = hashlib.sha256(
        f"samourai-test:{external_id}".encode()
    ).hexdigest()
    return {
        "id": tx_id,
        "wallet_id": f"wallet-{section}",
        "occurred_at": "2026-01-01T00:00:00Z",
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee if fee is not None else 1_000 if direction == "outbound" else 0,
        "fiat_rate": fiat_rate,
        "fiat_value": None,
        "kind": "whirlpool",
        "description": tx_id,
        "note": None,
        "external_id": physical_txid,
        "raw_json": json.dumps(
            {
                "Tx Hash": physical_txid,
                "chain": "bitcoin",
                "network": "main",
            }
        ),
        "config_json": _wallet_config(section),
    }


class SamouraiImportTest(unittest.TestCase):
    def test_source_set_import_and_inventory_metadata_stay_watch_only(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-samourai-source-set-") as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            _seed_book(conn)
            postmix_path = "m/84'/0'/2147483646'"
            deposit_native_path = "m/84'/0'/0'"
            source_set_path = Path(tmp) / "samourai-sources.json"
            source_set_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": postmix_path,
                                "descriptor": _fixture_descriptor(postmix_path, 0),
                                "change_descriptor": _fixture_descriptor(postmix_path, 1),
                                "samourai": {
                                    "target_mix_count": 5,
                                    "pool_denomination_sat": 5_000_000,
                                    "payment_code_secret": "do-not-persist",
                                },
                            }
                        ],
                        "xpubs": [
                            {
                                "section": "deposit",
                                "script_type": "p2pkh",
                                "root_path": "m/47'/0'/0'",
                                "xpub": _fixture_xpub("m/47'/0'/0'"),
                                "fingerprint": SAMOURAI_FIXTURE_FINGERPRINT,
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            result = core_samourai.import_samourai_wallet_group(
                conn,
                "Main",
                "Default",
                label="Samourai Explicit",
                source_set_file=str(source_set_path),
                network="main",
            )
            self.assertEqual(len(result["children"]), 2)
            paynym_child = next(
                child
                for child in result["children"]
                if child["config"]["samourai"]["root_path"] == "m/47'/0'/0'"
            )
            self.assertTrue(paynym_child["config"]["samourai"]["paynym"])
            child_id = next(
                child["id"]
                for child in result["children"]
                if child["config"]["samourai"]["section"] == "postmix"
            )
            profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile-1'").fetchone()
            wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (child_id,)).fetchone()
            sync_state = WalletSyncState(
                chain="bitcoin",
                network="main",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[],
                tracked_scripts={},
                history_cache={},
            )
            update_wallet_output_inventory(
                conn,
                profile,
                wallet,
                {"name": "mempool", "kind": "esplora"},
                sync_state,
                [
                    {
                        "txid": "aa" * 32,
                        "vout": 0,
                        "amount_sats": 50_000,
                        "asset": "BTC",
                        "confirmation_status": "confirmed",
                        "block_height": 900_000,
                        "raw": {
                            "samourai": {
                                "mix_count": 3,
                                "mix_count_confidence": "exact",
                                "round_txids": ["bb" * 32],
                                "unrelated_participant_inputs": ["cc" * 32],
                            }
                        },
                    }
                ],
                seen_at="2026-06-05T12:00:00Z",
            )
            rows = list_wallet_output_inventory(conn, child_id)
            self.assertEqual(rows[0]["samourai"]["section"], "postmix")
            self.assertEqual(rows[0]["samourai"]["minimum_mix_count"], 1)
            self.assertEqual(rows[0]["samourai"]["target_mix_count"], 5)
            self.assertEqual(rows[0]["samourai"]["pool_denomination_sat"], 5_000_000)
            self.assertEqual(rows[0]["samourai"]["mix_count"], 3)
            self.assertEqual(rows[0]["samourai"]["mix_count_confidence"], "exact")
            self.assertEqual(rows[0]["samourai"]["round_txids"], ["bb" * 32])
            self.assertNotIn("payment_code_secret", rows[0]["samourai"])
            self.assertNotIn("unrelated_participant_inputs", rows[0]["samourai"])
            raw_config = conn.execute(
                "SELECT config_json FROM wallets WHERE id = ?",
                (child_id,),
            ).fetchone()["config_json"]
            self.assertNotIn("payment_code_secret", raw_config)

            bad_source_set_path = Path(tmp) / "bad-samourai-sources.json"
            bad_source_set_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/1'/2147483646'",
                                "descriptor": _fixture_descriptor(postmix_path, 0),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(AppError) as raised:
                core_samourai.import_samourai_wallet_group(
                    conn,
                    "Main",
                    "Default",
                    label="Bad Samourai Explicit",
                    source_set_file=str(bad_source_set_path),
                    network="main",
                )
            self.assertEqual(raised.exception.code, "validation")

            bad_origin_path = Path(tmp) / "bad-origin-samourai-sources.json"
            bad_origin_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/0'/2147483646'",
                                "descriptor": _fixture_descriptor(deposit_native_path, 0),
                                "change_descriptor": _fixture_descriptor(
                                    deposit_native_path,
                                    1,
                                ),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(AppError) as raised:
                core_samourai.import_samourai_wallet_group(
                    conn,
                    "Main",
                    "Default",
                    label="Bad Samourai Origin",
                    source_set_file=str(bad_origin_path),
                    network="main",
                )
            self.assertEqual(raised.exception.code, "validation")
            self.assertIn("origin", str(raised.exception).lower())

            # A Samourai source pasted with only its receive descriptor is now
            # accepted rather than rejected: Kassiber synthesizes the standard
            # sibling change chain (Samourai accounts use BIP-standard 0/1
            # chains) so postmix change — including toxic change — still lands in
            # balances and the UTXO list.
            single_branch_path = Path(tmp) / "single-branch-samourai-sources.json"
            single_branch_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/0'/2147483646'",
                                "descriptor": _fixture_descriptor(postmix_path, 0),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            single_branch_result = core_samourai.import_samourai_wallet_group(
                conn,
                "Main",
                "Default",
                label="Single Branch Samourai",
                source_set_file=str(single_branch_path),
                network="main",
            )
            single_branch_id = next(
                child["id"]
                for child in single_branch_result["children"]
                if child["config"]["samourai"]["section"] == "postmix"
            )
            stored_config = json.loads(
                conn.execute(
                    "SELECT config_json FROM wallets WHERE id = ?",
                    (single_branch_id,),
                ).fetchone()["config_json"]
            )
            # The user supplied no change descriptor, yet the persisted wallet
            # still derives both branches when its plan is built for sync.
            self.assertNotIn("change_descriptor", stored_config)
            synthesized_plan = load_descriptor_plan(stored_config)
            self.assertEqual(
                [branch.branch_label for branch in synthesized_plan.branches],
                ["receive", "change"],
            )

            duplicate_path = Path(tmp) / "duplicate-samourai-sources.json"
            duplicate_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/0'/2147483646'",
                                "descriptor": _fixture_descriptor(postmix_path, 0),
                                "change_descriptor": _fixture_descriptor(postmix_path, 1),
                            },
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/0'/2147483646'",
                                "descriptor": _fixture_descriptor(postmix_path, 0),
                                "change_descriptor": _fixture_descriptor(postmix_path, 1),
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(AppError) as raised:
                core_samourai.import_samourai_wallet_group(
                    conn,
                    "Main",
                    "Default",
                    label="Duplicate Samourai",
                    source_set_file=str(duplicate_path),
                    network="main",
                )
            self.assertEqual(raised.exception.code, "validation")
            self.assertEqual(
                raised.exception.details["duplicate_labels"],
                ["Duplicate Samourai - Postmix"],
            )

    def test_inline_source_set_imports_public_account_material(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-samourai-inline-source-set-") as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            _seed_book(conn)
            source_set = {
                "network": "main",
                "xpubs": [
                    {
                        "section": "badbank",
                        "script_type": "p2wpkh",
                        "root_path": "m/84'/0'/2147483644'",
                        "xpub": _fixture_xpub("m/84'/0'/2147483644'"),
                    },
                    {
                        "section": "premix",
                        "script_type": "p2wpkh",
                        "root_path": "m/84'/0'/2147483645'",
                        "xpub": _fixture_xpub("m/84'/0'/2147483645'"),
                    },
                ],
            }

            result = core_samourai.import_samourai_wallet_group(
                conn,
                "Main",
                "Default",
                label="Samourai Inline",
                source_set=source_set,
                network="main",
            )

            self.assertEqual(len(result["children"]), 2)
            sections = {
                child["config"]["samourai"]["section"]
                for child in result["children"]
            }
            self.assertEqual(sections, {"badbank", "premix"})
            redacted = json.dumps(result, sort_keys=True)
            self.assertNotIn("xpub", redacted)
            self.assertIn('"descriptor": "[redacted]"', redacted)

    def test_tax_events_skip_internal_whirlpool_rows_but_quarantine_external_spend(self):
        wallet_refs = {
            "wallet-deposit": {"id": "wallet-deposit", "label": "Deposit"},
            "wallet-premix": {"id": "wallet-premix", "label": "Premix"},
            "wallet-badbank": {"id": "wallet-badbank", "label": "Badbank"},
            "wallet-postmix": {"id": "wallet-postmix", "label": "Postmix"},
            "wallet-ricochet": {"id": "wallet-ricochet", "label": "Ricochet"},
        }
        tx0_rows = [
            _tax_row(
                "tx0-out",
                "deposit",
                "outbound",
                external_id="tx0",
                amount=100_000_000,
                fee=1_000,
                fiat_rate=60_000,
            ),
            _tax_row(
                "tx0-premix",
                "premix",
                "inbound",
                external_id="tx0",
                amount=80_000_000,
            ),
            _tax_row(
                "tx0-badbank",
                "badbank",
                "inbound",
                external_id="tx0",
                amount=19_999_000,
            ),
        ]
        normalized = normalize_tax_asset_inputs(
            {"id": "profile-1", "workspace_id": "ws-1"},
            "BTC",
            tx0_rows,
            wallet_refs,
            [],
        )
        self.assertEqual(normalized.events, [])
        self.assertEqual(len(normalized.transfers), 2)
        self.assertEqual(
            normalized.ordered_items,
            [
                ("transfer", "tx0-out::tx0-premix"),
                ("transfer", "tx0-out::tx0-badbank"),
            ],
        )
        self.assertEqual(normalized.transfers[0].to_wallet_label, "Premix")
        self.assertEqual(normalized.transfers[1].to_wallet_label, "Badbank")
        # The whole group is atomic in the gate. The fee lands on the first
        # canonical leg (chronological, then row id — here tx0-badbank), the
        # same ordered_pair_component + allocate_fee_msat walk Austrian regime
        # inference uses, so booking and regime_flows describe the same
        # fee-bearing leg.
        group_ids = {transfer.group_id for transfer in normalized.transfers}
        self.assertEqual(group_ids, {"samourai-internal:tx0-out"})
        by_dest = {t.to_wallet_label: t for t in normalized.transfers}
        self.assertGreater(by_dest["Badbank"].fee, 0)
        self.assertEqual(by_dest["Badbank"].spot_price, 60_000)
        self.assertEqual(
            by_dest["Badbank"].sent,
            by_dest["Badbank"].received + by_dest["Badbank"].fee,
        )
        self.assertEqual(by_dest["Premix"].fee, 0)
        self.assertEqual(normalized.quarantines, [])

        missing_price = normalize_tax_asset_inputs(
            {"id": "profile-1", "workspace_id": "ws-1"},
            "BTC",
            [
                _tax_row(
                    "mix-out",
                    "premix",
                    "outbound",
                    external_id="mix-round",
                    amount=50_000_000,
                    fee=1_000,
                ),
                _tax_row(
                    "mix-in",
                    "postmix",
                    "inbound",
                    external_id="mix-round",
                    amount=49_999_000,
                ),
            ],
            wallet_refs,
            [],
        )
        self.assertEqual(missing_price.events, [])
        self.assertEqual(missing_price.transfers, [])
        self.assertEqual(missing_price.quarantines[0]["reason"], "missing_spot_price")
        detail = json.loads(missing_price.quarantines[0]["detail_json"])
        self.assertEqual(detail["required_for"], "coinjoin_fee")

        first_mix = normalize_tax_asset_inputs(
            {"id": "profile-1", "workspace_id": "ws-1"},
            "BTC",
            [
                _tax_row(
                    "first-mix-out",
                    "premix",
                    "outbound",
                    external_id="first-mix",
                    amount=50_000_000,
                    fee=1_000,
                    fiat_rate=60_000,
                ),
                _tax_row(
                    "first-mix-in",
                    "postmix",
                    "inbound",
                    external_id="first-mix",
                    amount=49_999_000,
                ),
            ],
            wallet_refs,
            [],
        )
        self.assertEqual(first_mix.events, [])
        self.assertEqual(len(first_mix.transfers), 1)

        remix = normalize_tax_asset_inputs(
            {"id": "profile-1", "workspace_id": "ws-1"},
            "BTC",
            [
                _tax_row(
                    "remix-out",
                    "postmix",
                    "outbound",
                    external_id="remix",
                    amount=50_000_000,
                    fee=1_000,
                    fiat_rate=60_000,
                ),
                _tax_row(
                    "remix-in",
                    "postmix",
                    "inbound",
                    external_id="remix",
                    amount=49_999_000,
                ),
            ],
            wallet_refs,
            [],
        )
        self.assertEqual(remix.events, [])
        self.assertEqual(len(remix.transfers), 1)

        for section in ("postmix", "badbank", "ricochet"):
            spend = _tax_row(
                f"{section}-spend",
                section,
                "outbound",
                external_id=f"{section}-external-spend",
            )
            normalized_spend = normalize_tax_asset_inputs(
                {"id": "profile-1", "workspace_id": "ws-1"},
                "BTC",
                [spend],
                wallet_refs,
                [],
            )
            self.assertEqual(
                normalized_spend.quarantines[0]["reason"],
                "missing_spot_price",
            )

    def test_tx0_multi_output_updates_per_wallet_holdings(self):
        profile = {
            "id": "profile-1",
            "workspace_id": "ws-1",
            "label": "Default",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            "wallet-deposit": {
                "id": "wallet-deposit",
                "label": "Deposit",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-premix": {
                "id": "wallet-premix",
                "label": "Premix",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-badbank": {
                "id": "wallet-badbank",
                "label": "Badbank",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }

        def engine_row(row):
            wallet = wallet_refs[row["wallet_id"]]
            return {
                **row,
                "wallet_label": wallet["label"],
                "wallet_account_id": wallet["wallet_account_id"],
                "account_code": wallet["account_code"],
                "account_label": wallet["account_label"],
                "created_at": row["occurred_at"],
            }

        rows = [
            engine_row(
                _tax_row(
                    "deposit-acquisition",
                    "deposit",
                    "inbound",
                    external_id="deposit-acquisition",
                    amount=100_001_000,
                    fiat_rate=60_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-out",
                    "deposit",
                    "outbound",
                    external_id="tx0",
                    amount=100_000_000,
                    fee=1_000,
                    fiat_rate=60_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-premix",
                    "premix",
                    "inbound",
                    external_id="tx0",
                    amount=80_000_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-badbank",
                    "badbank",
                    "inbound",
                    external_id="tx0",
                    amount=19_999_000,
                )
            ),
        ]
        state = build_tax_engine(profile).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[],
            )
        )

        self.assertEqual(state.quarantines, [])
        holdings = {
            wallet_label: totals["quantity"]
            for (_, wallet_label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertNotIn("Deposit", holdings)
        self.assertAlmostEqual(float(holdings["Premix"]), 0.0008, places=8)
        self.assertAlmostEqual(float(holdings["Badbank"]), 0.00019999, places=8)

        entry_types = sorted(entry["entry_type"] for entry in state.entries)
        self.assertEqual(
            entry_types,
            [
                "acquisition",
                "transfer_fee",
                "transfer_in",
                "transfer_in",
                "transfer_out",
                "transfer_out",
            ],
        )

    def test_tx0_group_quarantines_atomically_when_one_leg_fails_gate(self):
        # A tx0's N MOVE legs carry a shared group_id: when the source can fund
        # the first leg but not the second, no leg may book — previously the
        # premix leg booked while the badbank receipt was silently dropped.
        profile = {
            "id": "profile-1",
            "workspace_id": "ws-1",
            "label": "Default",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            "wallet-deposit": {
                "id": "wallet-deposit",
                "label": "Deposit",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-premix": {
                "id": "wallet-premix",
                "label": "Premix",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-badbank": {
                "id": "wallet-badbank",
                "label": "Badbank",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }

        def engine_row(row):
            wallet = wallet_refs[row["wallet_id"]]
            return {
                **row,
                "wallet_label": wallet["label"],
                "wallet_account_id": wallet["wallet_account_id"],
                "account_code": wallet["account_code"],
                "account_label": wallet["account_label"],
                "created_at": row["occurred_at"],
            }

        rows = [
            engine_row(
                _tax_row(
                    "deposit-acquisition",
                    "deposit",
                    "inbound",
                    external_id="deposit-acquisition",
                    amount=85_000_000,
                    fiat_rate=60_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-out",
                    "deposit",
                    "outbound",
                    external_id="tx0",
                    amount=100_000_000,
                    fee=1_000,
                    fiat_rate=60_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-premix",
                    "premix",
                    "inbound",
                    external_id="tx0",
                    amount=80_000_000,
                )
            ),
            engine_row(
                _tax_row(
                    "tx0-badbank",
                    "badbank",
                    "inbound",
                    external_id="tx0",
                    amount=19_999_000,
                )
            ),
        ]
        state = build_tax_engine(profile).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[],
            )
        )

        reasons = sorted(q["reason"] for q in state.quarantines)
        self.assertIn("insufficient_lots", reasons)
        self.assertIn("derived_transfer_group_blocked", reasons)
        entry_types = sorted(entry["entry_type"] for entry in state.entries)
        self.assertEqual(entry_types, ["acquisition"])
        holdings = {
            wallet_label: totals["quantity"]
            for (_, wallet_label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertNotIn("Premix", holdings)
        self.assertNotIn("Badbank", holdings)
        self.assertAlmostEqual(float(holdings["Deposit"]), 0.00085, places=8)

    def test_source_funds_suggests_whirlpool_as_coinjoin_boundary(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-samourai-sof-") as tmp:
            conn = open_db(Path(tmp) / "data")
            _seed_book(conn)
            for section in ("premix", "postmix"):
                conn.execute(
                    """
                    INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        f"wallet-{section}",
                        "ws-1",
                        "profile-1",
                        "acct-1",
                        section.title(),
                        "descriptor",
                        _wallet_config(section),
                        NOW,
                    ),
                )
            mix_txid = hashlib.sha256(b"samourai-test:mix-round").hexdigest()
            for tx_id, wallet_id, direction in (
                ("premix-out", "wallet-premix", "outbound"),
                ("postmix-in", "wallet-postmix", "inbound"),
            ):
                conn.execute(
                    """
                    INSERT INTO transactions(
                        id, workspace_id, profile_id, wallet_id, external_id,
                        fingerprint, occurred_at, direction, asset, amount,
                        fiat_currency, fiat_rate, fiat_value, raw_json, created_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        tx_id,
                        "ws-1",
                        "profile-1",
                        wallet_id,
                        mix_txid,
                        f"fp-{tx_id}",
                        "2026-02-01T00:00:00Z",
                        direction,
                        "BTC",
                        50_000_000,
                        "EUR",
                        60_000,
                        30,
                        json.dumps(
                            {
                                "Tx Hash": mix_txid,
                                "chain": "bitcoin",
                                "network": "main",
                            }
                        ),
                        NOW,
                    ),
                )
            conn.commit()

            hooks = SourceFundsHooks(
                resolve_scope=lambda _conn, _ws, _profile: (
                    {"id": "ws-1", "label": "Main"},
                    {"id": "profile-1", "label": "Default"},
                ),
                resolve_transaction=lambda _conn, _profile_id, ref: conn.execute(
                    "SELECT * FROM transactions WHERE id = ? OR external_id = ?",
                    (ref, ref),
                ).fetchone()
                or (_ for _ in ()).throw(AppError("missing", code="not_found")),
                format_table=lambda _headers, rows: [str(row) for row in rows],
            )
            result = suggest_links(
                conn,
                "Main",
                "Default",
                hooks,
                target_transaction_ref="postmix-in",
            )
            links = result["links"]
            self.assertEqual(len(links), 1)
            self.assertEqual(links[0]["link_type"], "coinjoin")
            self.assertEqual(links[0]["method"], "coinjoin")
            self.assertIn("privacy boundary", links[0]["explanation"])


if __name__ == "__main__":
    unittest.main()

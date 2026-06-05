from __future__ import annotations

import base64
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC

from kassiber.core import samourai as core_samourai
from kassiber.core.output_inventory import (
    list_wallet_output_inventory,
    update_wallet_output_inventory,
)
from kassiber.core.source_funds import SourceFundsHooks, suggest_links
from kassiber.core.sync import WalletSyncState
from kassiber.core.tax_events import normalize_tax_asset_inputs
from kassiber.db import open_db
from kassiber.errors import AppError


NOW = "2026-06-05T00:00:00Z"


def _mnemonic() -> str:
    from embit import bip39

    return bip39.mnemonic_from_bytes(bytes.fromhex("000102030405060708090a0b0c0d0e0f"))


def _passphrase() -> str:
    return hashlib.sha256(b"kassiber-samourai-backup-test-key").hexdigest()[:24]


def _pad(payload: bytes) -> bytes:
    pad_len = 16 - (len(payload) % 16)
    return payload + (b"\x00" * (pad_len - 1)) + bytes([pad_len])


def _derive(passphrase: str, salt: bytes, iterations: int, algorithm, length: int) -> bytes:
    return PBKDF2HMAC(
        algorithm=algorithm,
        length=length,
        salt=salt,
        iterations=iterations,
    ).derive(passphrase.encode("utf-8"))


def _aes_encrypt(key: bytes, iv: bytes, payload: bytes) -> bytes:
    encryptor = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
    return encryptor.update(_pad(payload)) + encryptor.finalize()


def _backup_payload() -> bytes:
    from embit import bip39

    entropy = bip39.mnemonic_to_bytes(_mnemonic())
    return json.dumps({"wallet": {"seed": entropy.hex(), "testnet": False}}).encode(
        "utf-8"
    )


def _account_xpub(path: str) -> tuple[str, str]:
    from embit import bip32, bip39

    root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_mnemonic(), ""))
    return root.derive(path).to_public().to_base58(), root.my_fingerprint.hex()


def _encrypted_backup_v1() -> str:
    iv = b"\x01" * 16
    key = _derive(_passphrase(), iv, 5_000, hashes.SHA1(), 32)
    payload = base64.b64encode(iv + _aes_encrypt(key, iv, _backup_payload())).decode(
        "ascii"
    )
    return json.dumps({"version": 1, "payload": payload})


def _encrypted_backup_v2() -> str:
    salt = b"\x02" * 8
    derived = _derive(_passphrase(), salt, 15_000, hashes.SHA256(), 48)
    ciphertext = _aes_encrypt(derived[:32], derived[32:], _backup_payload())
    payload = base64.b64encode(b"Salted__" + salt + ciphertext).decode("ascii")
    return json.dumps({"version": 2, "payload": payload})


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
        "external_id": external_id,
        "config_json": _wallet_config(section),
    }


class SamouraiImportTest(unittest.TestCase):
    def test_backup_v1_and_v2_decrypt_to_recovery_words(self):
        for encrypted in (_encrypted_backup_v1(), _encrypted_backup_v2()):
            payload = core_samourai.decrypt_samourai_backup_text(
                encrypted,
                _passphrase(),
            )
            mnemonic, network = core_samourai.mnemonic_from_samourai_backup_payload(
                payload
            )
            self.assertEqual(mnemonic, _mnemonic())
            self.assertEqual(network, "main")

        with self.assertRaises(AppError) as raised:
            core_samourai.decrypt_samourai_backup_text(
                _encrypted_backup_v2(),
                _passphrase() + "wrong",
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_import_from_mnemonic_creates_redacted_group_and_all_paths(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-samourai-") as tmp:
            conn = open_db(Path(tmp) / "data")
            _seed_book(conn)
            result = core_samourai.import_samourai_wallet_group(
                conn,
                "Main",
                "Default",
                label="Samourai",
                mnemonic=_mnemonic(),
                mnemonic_passphrase="",
                network="main",
                gap_limit=80,
            )
            self.assertEqual(result["group"]["kind"], "samourai")
            self.assertEqual(len(result["children"]), 9)
            redacted = json.dumps(result, sort_keys=True)
            self.assertNotIn(_mnemonic().split()[0], redacted)
            self.assertNotIn("xpub", redacted)
            self.assertIn('"descriptor": "[redacted]"', redacted)
            root_paths = {
                child["config"]["samourai"]["root_path"]
                for child in result["children"]
            }
            recognized_roots = {
                template.path.format(coin_type=0)
                for template in core_samourai.SAMOURAI_ACCOUNT_TEMPLATES
            }
            self.assertIn("m/47'/0'/0'", recognized_roots)
            self.assertNotIn("m/47'/0'/0'", root_paths)
            self.assertIn("m/84'/0'/2147483644'", root_paths)
            self.assertIn("m/84'/0'/2147483645'", root_paths)
            self.assertIn("m/84'/0'/2147483646'", root_paths)
            self.assertIn("m/84'/0'/2147483647'", root_paths)
            postmix = next(
                child
                for child in result["children"]
                if child["config"]["samourai"]["section"] == "postmix"
            )
            self.assertEqual(postmix["gap_limit"], 80)
            self.assertEqual(postmix["config"]["samourai"]["minimum_mix_count"], 1)

    def test_source_set_import_and_inventory_metadata_stay_watch_only(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-samourai-source-set-") as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            _seed_book(conn)
            sources = core_samourai.derive_samourai_wallet_sources(
                _mnemonic(),
                "",
                network="main",
                gap_limit=40,
            )
            postmix_source = next(
                source
                for source in sources
                if source["config"]["samourai"]["section"] == "postmix"
            )
            paynym_xpub, fingerprint = _account_xpub("m/47'/0'/0'")
            source_set_path = Path(tmp) / "samourai-sources.json"
            source_set_path.write_text(
                json.dumps(
                    {
                        "network": "main",
                        "children": [
                            {
                                "section": "postmix",
                                "script_type": "p2wpkh",
                                "root_path": "m/84'/0'/2147483646'",
                                "descriptor": postmix_source["config"]["descriptor"],
                                "change_descriptor": postmix_source["config"][
                                    "change_descriptor"
                                ],
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
                                "xpub": paynym_xpub,
                                "fingerprint": fingerprint,
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
            self.assertNotIn(_mnemonic().split()[0], raw_config)

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
                                "descriptor": postmix_source["config"]["descriptor"],
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
        self.assertEqual(len(normalized.events), 1)
        self.assertEqual(normalized.events[0].amount, 0)
        self.assertGreater(normalized.events[0].fee, 0)
        self.assertEqual(normalized.events[0].spot_price, 60_000)
        self.assertEqual(normalized.transfers, [])
        self.assertEqual(normalized.ordered_items, [("event", "tx0-out")])
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
        self.assertEqual(detail["required_for"], "samourai_privacy_fee")

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
                        "mix-round",
                        f"fp-{tx_id}",
                        "2026-02-01T00:00:00Z",
                        direction,
                        "BTC",
                        50_000_000,
                        "EUR",
                        60_000,
                        30,
                        "{}",
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
            self.assertEqual(links[0]["method"], "samourai_whirlpool")
            self.assertIn("privacy boundary", links[0]["explanation"])


if __name__ == "__main__":
    unittest.main()

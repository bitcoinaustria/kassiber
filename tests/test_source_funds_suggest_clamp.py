"""Fast unit-level guard for the suggest_links write-cap ceiling.

The full CLI cap test (`test_suggest_links_caps_writes_per_call`) already
covers the runtime cap mechanism. This module pins the second half of
the contract: a caller cannot lift the ceiling by passing
``--max-suggestions`` (or the daemon equivalent) above
``SUGGESTION_WRITE_CAP``. The clamp lives in ``suggest_links`` itself
so every call site is covered with one test.
"""

from __future__ import annotations

import tempfile
import hashlib
import json
import unittest.mock
import uuid
from pathlib import Path

from kassiber.core import source_funds as sf
from kassiber.core.source_funds import SourceFundsHooks
from kassiber.errors import AppError


def _build_hooks(workspace_id: str, profile_id: str) -> SourceFundsHooks:
    workspace = {"id": workspace_id, "label": "ws"}
    profile = {"id": profile_id, "label": "Default"}

    def resolve_scope(_conn, _ws, _prof):
        return workspace, profile

    def resolve_transaction(conn, profile_id, ref):
        row = conn.execute(
            "SELECT * FROM transactions WHERE profile_id = ? AND (id = ? OR external_id = ?)",
            (profile_id, ref, ref),
        ).fetchone()
        if row is None:
            raise AppError(f"transaction '{ref}' not found", code="not_found")
        return dict(row)

    def format_table(_headers, rows):
        return [str(row) for row in rows]

    return SourceFundsHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        format_table=format_table,
    )


class SuggestLinksClampTest(unittest.TestCase):
    def setUp(self):
        from kassiber import db as kassiber_db

        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db_path = Path(self.tmp.name) / "kassiber.sqlite3"
        self.conn = kassiber_db.open_db(self.db_path)
        self.workspace_id = "ws-1"
        self.profile_id = "prof-1"
        self.account_id = "acct-1"
        now = "2026-04-01T00:00:00Z"
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
            (self.workspace_id, "ws", now),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            (self.profile_id, self.workspace_id, "Default", "EUR", "generic", now),
        )
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            (self.account_id, self.workspace_id, self.profile_id, "acct", "acct", "personal", now),
        )
        self.conn.commit()
        self.hooks = _build_hooks(self.workspace_id, self.profile_id)

    def _wallet(self, label: str) -> str:
        wallet_id = str(uuid.uuid4())
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                wallet_id,
                self.workspace_id,
                self.profile_id,
                self.account_id,
                label,
                "personal",
                "2026-04-01T00:00:00Z",
            ),
        )
        self.conn.commit()
        return wallet_id

    def _tx(
        self,
        wallet_id: str,
        external_id: str,
        direction: str,
        amount_msat: int = 100_000_000,
        asset: str = "BTC",
    ) -> str:
        tx_id = str(uuid.uuid4())
        physical_txid = hashlib.sha256(
            f"source-funds-clamp:{external_id}".encode()
        ).hexdigest()
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fiat_currency, fiat_rate, fiat_value, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                self.workspace_id,
                self.profile_id,
                wallet_id,
                physical_txid,
                f"fp-{tx_id}",
                "2026-04-01T09:00:00Z",
                direction,
                asset,
                amount_msat,
                "EUR",
                50000.0,
                float(amount_msat) / 1e11 * 50000.0,
                json.dumps({"Tx Hash": physical_txid}),
                "2026-04-01T09:00:00Z",
            ),
        )
        self.conn.commit()
        return tx_id

    def _store_moves(self, pairs: list[tuple[str, str]]) -> None:
        for ordinal, (source_id, target_id) in enumerate(pairs):
            source_wallet = self.conn.execute(
                "SELECT wallet_id FROM transactions WHERE id = ?", (source_id,)
            ).fetchone()[0]
            target_wallet = self.conn.execute(
                "SELECT wallet_id FROM transactions WHERE id = ?", (target_id,)
            ).fetchone()[0]
            decision_id = hashlib.sha256(
                f"stored-move:{source_id}:{target_id}".encode()
            ).hexdigest()
            source_hash = hashlib.sha256(
                f"source:{source_id}".encode()
            ).hexdigest()
            target_hash = hashlib.sha256(
                f"target:{target_id}".encode()
            ).hexdigest()
            self.conn.execute(
                """
                INSERT INTO journal_custody_decisions(
                    decision_id, workspace_id, profile_id,
                    source_transaction_id, target_transaction_id,
                    source_observation_hash, source_start_msat, source_end_msat,
                    target_observation_hash, target_start_msat, target_end_msat,
                    source_wallet_id, target_wallet_id,
                    source_network, target_network, source_rail, target_rail,
                    source_asset, target_asset, state, basis_state, reason,
                    atomic_group_id, occurred_at, target_occurred_at, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, 0, 100000000, ?, 0, 100000000,
                         ?, ?, 'main', 'main', 'bitcoin', 'bitcoin', 'BTC', 'BTC',
                         'internal_verified', 'eligible', 'test_stored_move', ?,
                         '2026-04-01T09:00:00Z', '2026-04-01T09:00:00Z',
                         '2026-04-01T09:00:00Z')
                """,
                (
                    decision_id,
                    self.workspace_id,
                    self.profile_id,
                    source_id,
                    target_id,
                    source_hash,
                    target_hash,
                    source_wallet,
                    target_wallet,
                    f"group-{ordinal}",
                ),
            )
        count = self.conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            (self.profile_id,),
        ).fetchone()[0]
        self.conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = '2026-04-01T09:00:00Z',
                last_processed_tx_count = ?,
                last_processed_input_version = journal_input_version
            WHERE id = ?
            """,
            (count, self.profile_id),
        )
        self.conn.commit()

    def test_oversized_max_suggestions_arg_does_not_lift_ceiling(self):
        """A caller passing max_suggestions > SUGGESTION_WRITE_CAP gets
        clamped to the ceiling: even when two same-id pairs are
        available, a patched cap of 1 still fires after the first
        suggestion regardless of the caller's request."""
        wallet_a = self._wallet("Wallet A")
        wallet_b = self._wallet("Wallet B")
        # Two same-external-id pairs would normally produce two
        # suggestions. With cap=1, only the first should land before
        # the rollback triggers.
        pairs = []
        for external_id in ("pair-one", "pair-two"):
            pairs.append(
                (
                    self._tx(wallet_a, external_id, "outbound"),
                    self._tx(wallet_b, external_id, "inbound"),
                )
            )
        self._store_moves(pairs)

        with unittest.mock.patch.object(sf, "SUGGESTION_WRITE_CAP", 1):
            with self.assertRaises(AppError) as cm:
                sf.suggest_links(
                    self.conn,
                    self.workspace_id,
                    self.profile_id,
                    self.hooks,
                    max_suggestions=999_999,
                )
        self.assertEqual(cm.exception.code, "validation")
        self.assertEqual(cm.exception.details.get("max_suggestions"), 1)
        # The transaction was rolled back; no link rows should be persisted.
        link_count = self.conn.execute(
            "SELECT COUNT(*) FROM source_funds_links WHERE profile_id = ?",
            (self.profile_id,),
        ).fetchone()[0]
        self.assertEqual(link_count, 0)

    def test_in_range_caller_value_is_honored_unchanged(self):
        """A caller passing max_suggestions <= SUGGESTION_WRITE_CAP
        should still see their tighter limit enforced."""
        wallet_a = self._wallet("Wallet A")
        wallet_b = self._wallet("Wallet B")
        pairs = []
        for external_id in ("pair-one", "pair-two", "pair-three"):
            pairs.append(
                (
                    self._tx(wallet_a, external_id, "outbound"),
                    self._tx(wallet_b, external_id, "inbound"),
                )
            )
        self._store_moves(pairs)

        with self.assertRaises(AppError) as cm:
            sf.suggest_links(
                self.conn,
                self.workspace_id,
                self.profile_id,
                self.hooks,
                max_suggestions=2,
            )
        self.assertEqual(cm.exception.code, "validation")
        self.assertEqual(cm.exception.details.get("max_suggestions"), 2)


if __name__ == "__main__":
    unittest.main()

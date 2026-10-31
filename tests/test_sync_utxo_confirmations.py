"""Unchanged-history syncs refresh counts without re-observing UTXO membership."""
import time
import json

import pytest

from kassiber.core.book_network import resolve_book_environment
from kassiber.core.sync import WalletBackendFetch, WalletSyncHooks, WalletSyncState, sync_wallet_from_backend
from kassiber.db import open_db
from tests.test_output_inventory import _seed_wallet, _insert_wallet_utxo


@pytest.mark.parametrize('protocol,chain', [('esplora', 'bitcoin'), ('electrum', 'bitcoin'), ('electrum', 'liquid')])
@pytest.mark.parametrize('tip', [809010, None, True, '809010'])
def test_unchanged_sync_updates_only_confirmations_for_its_source(tmp_path, protocol, chain, tip):
    conn = open_db(tmp_path)
    try:
        profile, wallet = _seed_wallet(conn)
        conn.execute("UPDATE wallets SET config_json=? WHERE id=?",
                     (json.dumps({'chain': chain, 'network': 'main', 'backend': 'fixture'}), wallet['id']))
        wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet['id'],)).fetchone()
        backend = dict(name='fixture', kind=protocol, url='https://unused.invalid')
        for row_id, source, spent in [('current', 'fixture', None), ('other', 'other', None), ('spent', 'fixture', '2026-01-02')]:
            _insert_wallet_utxo(conn, profile, wallet, row_id=row_id, txid=row_id,
                                backend_name=source, backend_kind=protocol, chain=chain, spent_at=spent)
        conn.commit()
        before = {row['id']: dict(row) for row in conn.execute('SELECT * FROM wallet_utxos')}
        state = WalletSyncState(chain=chain, network='main', descriptor_plan=None, policy_asset_id='',
                                targets=[], tracked_scripts={}, history_cache={})
        fetch = WalletBackendFetch(backend=backend, sync_state=state, normalized_records=[],
            adapter_meta={'utxos_skipped_unchanged': True, 'freshness_checkpoint': {f'{protocol}_tip_height': tip}},
            kind=protocol, started=time.monotonic(), force_full=False,
            book_environment_id=resolve_book_environment(conn, profile['id'])['environment_id'])
        hooks = WalletSyncHooks(import_file=lambda *a: {}, insert_records=lambda *a: {},
            resolve_backend=lambda *a: backend, resolve_sync_state=lambda *a: state,
            normalize_addresses=lambda x: x, backend_adapters={})
        outcome = sync_wallet_from_backend(conn, {}, profile, wallet, hooks, prefetched=fetch,
                                            _allow_negative_balance_rescan=False)
        after = {row['id']: dict(row) for row in conn.execute('SELECT * FROM wallet_utxos')}
        expected = before['current'].copy()
        if type(tip) is int:
            expected['confirmations'] = 11
        assert after['current'] == expected
        assert after['other'] == before['other']
        assert after['spent'] == before['spent']
        assert outcome['utxos_skipped_unchanged']
        assert not outcome['utxos_refreshed']
        # The update participates in the caller's transaction.
        conn.rollback()
        assert dict(conn.execute("SELECT * FROM wallet_utxos WHERE id='current'").fetchone()) == before['current']
    finally:
        conn.close()

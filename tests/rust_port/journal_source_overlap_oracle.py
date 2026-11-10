"""Frozen complete-dump oracle for the local journal source repair stage."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from copy import deepcopy
from dataclasses import replace
import json
import sqlite3
import uuid
from unittest.mock import patch

from kassiber.cli import handlers
from kassiber.cli.handlers import _journal_source_overlap_warning, _repair_journal_source_overlaps
from tests.rust_port.journal_prebuild_oracle import ROOT, CLOCK, refusal, seed
from tests.rust_port.journal_persistence_oracle import dump
from tests.test_source_overlap import (
    ADDR_A, ADDR_B, ADDR_C, _descriptor_config, _descriptor_target,
    _script, _seed_book, _tx, _utxo, _wallet,
)

PATH = ROOT / "crates/journals/tests/data/prebuild-sources.json"


def capture(conn, name):
    conn.commit()
    before = dump(conn)
    profile = conn.execute("SELECT * FROM profiles WHERE id='pf'").fetchone()
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("SAVEPOINT source_oracle")
    error = result = wire_fields = None
    ids = iter(uuid.UUID(int=i) for i in range(1, 1000))
    # Savepoint names consume Python UUIDs but are not persisted or projected.
    # A refusal after an audited write still drew its IDs: read them before
    # the rollback discards them.
    identity = []
    def read_ids():
        for event in conn.execute("SELECT id FROM transaction_edit_events ORDER BY rowid"):
            identity.append(event[0])
            identity.extend(r[0] for r in conn.execute("SELECT id FROM transaction_edit_fields WHERE event_id=? ORDER BY rowid", event))
    with ExitStack() as stack:
        stack.enter_context(patch("uuid.uuid4", side_effect=lambda: next(ids)))
        ticks = []
        def clock():
            tick = CLOCK if name != "changing-clock" else f"2026-10-31T12:00:{len(ticks):02}Z"
            ticks.append(tick)
            return tick
        stack.enter_context(patch.object(handlers, "_metadata_hooks", return_value=replace(handlers._metadata_hooks(), now_iso=clock)))
        for target in ("socket.getaddrinfo", "socket.socket.connect", "socket.socket.connect_ex", "socket.socket.sendto"):
            stack.enter_context(patch(target, side_effect=AssertionError("source repair attempted network I/O")))
        try:
            repair = _repair_journal_source_overlaps(conn, profile)
            warning = _journal_source_overlap_warning(conn, profile, repair)
            result = dict(repair=repair, warning=warning)
            fields = {}
            if repair is not None:
                fields["source_overlap_repair"] = repair
            if warning is not None:
                fields["source_overlap_warning"] = warning
            wire_fields = json.dumps(fields, sort_keys=False)
            read_ids()
        except Exception as exc:
            error = refusal(exc)
            read_ids()
            conn.execute("ROLLBACK TO source_oracle")
    conn.execute("RELEASE source_oracle")
    expected = dump(conn)
    conn.rollback()
    return dict(name=name, before=before, expected=expected, result=result, error=error, ids=identity, ticks=ticks, wire_fields=wire_fields)


def vectors():
    names = (
        "no-overlap", "trim-empty", "trim-partial", "duplicate", "multiple-duplicates",
        "unresolved-amount", "unresolved-fee", "unresolved-direction", "unresolved-asset",
        "unresolved-raw", "unresolved-no-external-id", "outbound", "nested-raw",
        "review-conflict", "mixed-conflict", "address-only", "inventory-overlap",
        "raw-inventory-script", "invalid-inventory-script", "different-network",
        "deprecated-inactive", "deprecated-active", "descriptor-horizon", "checkpoint-horizon",
        "inventory-horizon", "invalid-descriptor", "invalid-config", "invalid-address",
        "invalid-address-container", "invalid-deprecated", "excluded-duplicate", "other-profile",
        "trim-inventory-checkpoint", "address-map-order", "address-case", "empty-scope",
        "script-hex", "script-pubkey-object", "script-pubkey-string", "raw-inventory-whitespace",
        "invalid-outputs", "invalid-inputs", "changing-clock", "invalid-deprecated-none",
        "legacy-real-amount", "legacy-real-fee", "invalid-economic-amount", "invalid-economic-fee", "economic-text-int",
        "wide-economic-amount", "wide-economic-fee", "wide-economic-max-digits", "wide-economic-i128",
    )
    output = []
    for name in names:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        seed(conn)
        _seed_book(conn)
        config = _descriptor_config()
        target = _descriptor_target(config, 5 if name.endswith("horizon") else 0)
        addr = target.address
        chain = dict(chain="bitcoin", network="mainnet")
        if name == "no-overlap":
            addr = ADDR_C
        descriptor = config if name not in {"address-only", "inventory-overlap", "raw-inventory-script", "invalid-inventory-script", "invalid-descriptor"} else chain
        if name == "invalid-descriptor":
            descriptor = dict(chain, descriptor="bad")
        _wallet(conn, "canonical", "Descriptor", "descriptor" if name != "address-only" else "address", descriptor)
        address_config = dict(chain, addresses=[addr, ADDR_B] if name == "trim-partial" else [addr])
        if name == "different-network": address_config["network"] = "test"
        if name.startswith("deprecated-"): address_config["deprecated"] = True
        if name == "invalid-address": address_config["addresses"] = ["not-an-address"]
        if name == "invalid-address-container": address_config["addresses"] = 1
        if name == "invalid-deprecated": address_config["deprecated"] = "junk"
        if name == "invalid-deprecated-none": address_config["deprecated"] = "None"
        if name == "empty-scope": address_config.pop("network")
        if name == "address-case": address_config["addresses"] = [addr.upper(), addr]
        _wallet(conn, "duplicate", "Address", "address", address_config)
        if name == "address-map-order":
            address_config["addresses"] = {addr: 1, ADDR_B: 2}
            conn.execute("UPDATE wallets SET config_json=? WHERE id='duplicate'", (json.dumps(address_config),))
        if name == "invalid-config": conn.execute("UPDATE wallets SET config_json='[' WHERE id='duplicate'")
        if name == "address-only":
            conn.execute("UPDATE wallets SET config_json=? WHERE id='canonical'", (json.dumps(dict(chain, addresses=[addr])),))
        if name in {"inventory-overlap", "raw-inventory-script", "invalid-inventory-script", "inventory-horizon", "raw-inventory-whitespace"}:
            _utxo(conn, "canonical", addr)
            if name == "inventory-overlap":
                conn.execute("UPDATE wallet_utxos SET address=NULL,script_pubkey=?", (_script(addr),))
            if name == "raw-inventory-script":
                conn.execute("UPDATE wallet_utxos SET address=NULL,script_pubkey='invalid',raw_json=?", (json.dumps(dict(script_pubkey=_script(addr))),))
            if name == "invalid-inventory-script":
                conn.execute("UPDATE wallet_utxos SET address='invalid',script_pubkey='xyz'")
            if name == "inventory-horizon":
                conn.execute("UPDATE wallet_utxos SET address_index=5")
                _tx(conn, "unmatched", "duplicate", "cc" * 32, script_pubkey=_script(addr))
            if name == "raw-inventory-whitespace":
                conn.execute("UPDATE wallet_utxos SET script_pubkey=?", ("00 14 " + _script(addr)[4:],))
        if name == "trim-inventory-checkpoint":
            _utxo(conn, "duplicate", addr)
            _utxo(conn, "duplicate", ADDR_B, txid="bb")
            conn.execute("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,checkpoint_json,updated_at) VALUES('pf','onchain_wallet:duplicate','onchain_wallet','Address','{}',?)", (CLOCK,))
        if name in {"trim-empty", "trim-partial", "duplicate", "multiple-duplicates", "outbound", "nested-raw", "review-conflict", "mixed-conflict", "excluded-duplicate", "script-hex", "script-pubkey-object", "script-pubkey-string", "invalid-outputs", "invalid-inputs", "changing-clock", "legacy-real-amount", "legacy-real-fee", "invalid-economic-amount", "invalid-economic-fee", "economic-text-int"} or name.startswith(("unresolved-", "wide-economic-")) or name == "deprecated-active":
            direction = "outbound" if name in {"outbound", "invalid-inputs"} else "inbound"
            _tx(conn, "keep", "canonical", "AA" * 32, direction=direction, script_pubkey=_script(addr))
            _tx(conn, "exclude", "duplicate", "AA" * 32, direction=direction, script_pubkey=_script(addr))
            if name.startswith("trim-"):
                conn.execute("DELETE FROM transactions WHERE id='exclude'")
            if name == "unresolved-amount": conn.execute("UPDATE transactions SET amount=amount+1 WHERE id='exclude'")
            if name == "unresolved-fee": conn.execute("UPDATE transactions SET fee=1 WHERE id='exclude'")
            if name.startswith("legacy-real-"):
                field = "amount" if name.endswith("amount") else "fee"
                conn.execute(f"UPDATE transactions SET {field}=CASE id WHEN 'keep' THEN 1.25 ELSE 2.5 END")
            if name.startswith("invalid-economic-"):
                field = "amount" if name.endswith("amount") else "fee"
                conn.execute(f"UPDATE transactions SET {field}='invalid' WHERE id='exclude'")
            if name == "economic-text-int":
                conn.execute("UPDATE transactions SET amount=1000 WHERE id='keep'")
                conn.execute("UPDATE transactions SET amount='1_000' WHERE id='exclude'")
            numeric = {
                'wide-economic-amount': ('amount', '1_' + '0' * 4300),
                'wide-economic-fee': ('fee', '1_' + '0' * 4300),
                'wide-economic-max-digits': ('amount', '1_' + '0' * 4299),
                'wide-economic-i128': ('amount', '170_141183460469231731687303715884105727'),
            }
            if name in numeric:
                field, raw = numeric[name]
                conn.execute(f'UPDATE transactions SET {field}=?', (raw,))
            if name == "unresolved-direction":
                conn.execute("UPDATE transactions SET direction='outbound' WHERE id='exclude'")
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps(dict(vin=[dict(prevout=dict(scriptpubkey=_script(addr)))],vout=[dict(scriptpubkey=_script(addr))])),))
            if name == "unresolved-asset": conn.execute("UPDATE transactions SET asset='LBTC' WHERE id='exclude'")
            if name == "unresolved-raw": conn.execute("UPDATE transactions SET raw_json='{}' WHERE id='exclude'")
            if name == "unresolved-no-external-id": conn.execute("UPDATE transactions SET external_id=NULL WHERE id='exclude'")
            if name == "excluded-duplicate": conn.execute("UPDATE transactions SET excluded=1 WHERE id='exclude'")
            if name == "nested-raw": conn.execute("UPDATE transactions SET raw_json=json_object('tx',json(raw_json))")
            if name in {"invalid-outputs", "invalid-inputs"}:
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps({"vout" if name.endswith("outputs") else "vin": True}),))
            if name.startswith("script-"):
                entry = {"script_hex": _script(addr)} if name == "script-hex" else {"scriptPubKey": {"hex": _script(addr)} if name.endswith("object") else _script(addr)}
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps({"vout": [entry]}),))
            if name in {"multiple-duplicates", "mixed-conflict", "changing-clock"}:
                _tx(conn, "exclude-two", "duplicate", "bb" * 32, script_pubkey=_script(addr))
                _tx(conn, "keep-two", "canonical", "bb" * 32, script_pubkey=_script(addr))
            if name in {"review-conflict", "mixed-conflict"}:
                conn.execute("INSERT INTO custody_components(id,workspace_id,profile_id,lineage_id,revision,component_type,state,authored_source,created_at) VALUES('review','ws','pf','review',1,'transfer','draft','user',?)", (CLOCK,))
                conn.execute("INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,role,ordinal,rail,asset,exposure,conservation_unit,amount_msat,transaction_id,created_at) VALUES('leg','review','ws','pf','source',0,'onchain','BTC','BTC','msat',1000000000,'exclude',?)", (CLOCK,))
                conn.execute("UPDATE custody_components SET state='active' WHERE id='review'")
        if name == "checkpoint-horizon":
            conn.execute("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,checkpoint_json,updated_at) VALUES('pf','onchain_wallet:canonical','onchain_wallet','Descriptor',?,?)", ('{"highest_used":{"0":5}}',CLOCK))
        if name == "other-profile":
            conn.execute("UPDATE wallets SET profile_id='other' WHERE id='duplicate'")
        output.append(capture(conn, name))
        conn.close()
    output.extend(branch_vectors(output))
    return output


def branch_vectors(originals):
    from tests.rust_port.journal_pipeline_oracle import restore

    templates = {case['name']: case for case in originals}
    recipes = (
        ('nested-scriptpubkey-fallback', 'duplicate'),
        ('integer-outputs', 'duplicate'),
        ('float-inputs', 'outbound'),
        ('canonical-without-observed-row', 'duplicate'),
        ('all-canonical-observed-rows', 'duplicate'),
        ('comma-separated-addresses', 'trim-partial'),
        ('boolean-addresses', 'no-overlap'),
        ('float-addresses', 'no-overlap'),
        ('blank-address-entries', 'trim-empty'),
        ('deprecated-false-text', 'duplicate'),
        ('empty-wallet-label', 'trim-empty'),
        ('empty-wallet-kind', 'address-only'),
        ('unknown-chain-scope', 'address-only'),
        ('invalid-even-script', 'invalid-inventory-script'),
        ('inventory-without-scope', 'inventory-overlap'),
        ('checkpoint-boolean-integer', 'checkpoint-horizon'),
        ('checkpoint-real-integer', 'checkpoint-horizon'),
        ('checkpoint-negative-integer', 'checkpoint-horizon'),
        ('checkpoint-invalid-integer', 'checkpoint-horizon'),
        ('checkpoint-merged-inventory', 'inventory-horizon'),
        ('xpub-script-types', 'duplicate'),
        ('xpub-without-script-types', 'duplicate'),
        ('review-legacy-identity', 'review-conflict'),
        ('unknown-network-scope', 'address-only'),
        ('json-null-inventory-raw', 'invalid-inventory-script'),
    )
    output = []
    for name, template in recipes:
        before = deepcopy(templates[template]['before'])

        def records(table):
            data = before[table]
            return [dict(zip(data['columns'], row)) for row in data['rows']]

        def replace(table, rows):
            before[table]['rows'] = [
                [row[column] for column in before[table]['columns']]
                for row in rows
            ]

        wallets = records('wallets')
        duplicate = next(row for row in wallets if row['id'] == 'duplicate')
        canonical = next(row for row in wallets if row['id'] == 'canonical')
        config = json.loads(duplicate['config_json'])
        if name in {'nested-scriptpubkey-fallback', 'integer-outputs', 'float-inputs'}:
            transactions = records('transactions')
            for row in transactions:
                raw = json.loads(row['raw_json'])
                if name == 'nested-scriptpubkey-fallback':
                    raw['vout'] = [dict(scriptPubKey=dict(scriptpubkey=raw['vout'][0]['scriptpubkey']))]
                elif name == 'integer-outputs':
                    raw['vout'] = 1
                else:
                    raw['vin'] = 1.5
                row['raw_json'] = json.dumps(raw)
            replace('transactions', transactions)
        elif name == 'canonical-without-observed-row':
            third = dict(duplicate, id='third', label='Third', _rowid_=max(row['_rowid_'] for row in wallets) + 1)
            wallets.append(third)
            transactions = [row for row in records('transactions') if row['id'] == 'exclude']
            transactions.append(dict(transactions[0], id='third', wallet_id='third', fingerprint='third', _rowid_=3))
            replace('transactions', transactions)
        elif name == 'all-canonical-observed-rows':
            duplicate['kind'] = canonical['kind']
            first = config['addresses'][0]
            canonical['config_json'] = json.dumps(dict(chain='bitcoin', network='mainnet', addresses=[first]))
            config = dict(chain='bitcoin', network='mainnet', addresses=[ADDR_B])
            peer = dict(duplicate, id='peer', label='Peer', kind='address',
                        _rowid_=max(row['_rowid_'] for row in wallets) + 1)
            peer['config_json'] = json.dumps(dict(config, addresses=[first, ADDR_B]))
            wallets.append(peer)
            transactions = records('transactions')
            for row in transactions:
                if row['wallet_id'] == 'duplicate':
                    row['raw_json'] = json.dumps(dict(vout=[dict(scriptpubkey=_script(ADDR_B))]))
            replace('transactions', transactions)
        elif name == 'comma-separated-addresses':
            config['addresses'] = ', '.join(config['addresses'])
        elif name == 'boolean-addresses':
            config['addresses'] = True
        elif name == 'float-addresses':
            config['addresses'] = 1.5
        elif name == 'blank-address-entries':
            config['addresses'] = ['', '  ', *config['addresses']]
        elif name == 'deprecated-false-text':
            config['deprecated'] = 'off'
        elif name == 'empty-wallet-label':
            duplicate['label'] = ''
        elif name == 'empty-wallet-kind':
            duplicate['kind'] = ''
        elif name in {'unknown-chain-scope', 'unknown-network-scope'}:
            chain = 'unknown' if name == 'unknown-chain-scope' else 'bitcoin'
            config.update(chain=chain, network='unknown')
            other = json.loads(canonical['config_json'])
            other.update(chain=chain, network='unknown')
            canonical['config_json'] = json.dumps(other)
        elif name in {'invalid-even-script', 'inventory-without-scope', 'json-null-inventory-raw'}:
            inventory = records('wallet_utxos')
            for row in inventory:
                if name == 'invalid-even-script':
                    row['script_pubkey'] = 'zz'
                elif name == 'json-null-inventory-raw':
                    row['raw_json'] = 'null'
                else:
                    row['chain'] = ''
            replace('wallet_utxos', inventory)
        elif name.startswith('xpub-'):
            material = json.loads(canonical['config_json'])
            descriptor = material.pop('descriptor')
            material.pop('change_descriptor')
            material['xpub'] = descriptor[descriptor.index('['):descriptor.rindex('/0/*')]
            if name == 'xpub-script-types':
                material['script_types'] = ['p2wpkh']
            canonical['kind'] = 'xpub'
            canonical['config_json'] = json.dumps(material)
        elif name == 'review-legacy-identity':
            legs = records('custody_component_legs')
            legs.append(dict(legs[0], _rowid_=2, id='target-leg', role='destination',
                             ordinal=1, transaction_id=None))
            replace('custody_component_legs', legs)
            term = {column: None for column in before['custody_component_economic_terms']['columns']}
            term.update(_rowid_=1, id='term', component_id='review', workspace_id='ws',
                        profile_id='pf', ordinal=0, source_leg_id='leg', target_leg_id='target-leg',
                        term_kind='transaction_pair', legacy_source_id='legacy-review',
                        source_row_hash='0' * 64, review_kind='manual', tax_policy='carrying-value',
                        reviewed_source_amount_msat=1000000000, created_at=CLOCK)
            replace('custody_component_economic_terms', [term])
        elif name.startswith('checkpoint-'):
            highest = {
                'checkpoint-boolean-integer': True,
                'checkpoint-real-integer': 5.75,
                'checkpoint-negative-integer': '-2',
                'checkpoint-invalid-integer': None,
                'checkpoint-merged-inventory': 6,
            }[name]
            checkpoints = records('freshness_source_states')
            if not checkpoints:
                data = templates['checkpoint-horizon']['before']['freshness_source_states']
                checkpoints = [dict(zip(data['columns'], row)) for row in data['rows']]
            for row in checkpoints:
                row['checkpoint_json'] = json.dumps(dict(highest_used={'0': highest}))
            replace('freshness_source_states', checkpoints)
            if name in {'checkpoint-boolean-integer', 'checkpoint-merged-inventory'}:
                target = 3 if name == 'checkpoint-boolean-integer' else 8
                config['addresses'] = [
                    _descriptor_target(json.loads(canonical['config_json']), target).address
                ]
        else:
            raise AssertionError(name)
        duplicate['config_json'] = json.dumps(config)
        replace('wallets', wallets)
        conn = sqlite3.connect(':memory:')
        conn.row_factory = sqlite3.Row
        try:
            restore(conn, before)
            output.append(capture(conn, name))
        finally:
            conn.close()
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    data = vectors()
    base = data[0]["before"]
    packed = dict(base=base, cases=[dict(case, **{side: {table: item["rows"] for table,item in case[side].items() if item != base[table]} for side in ("before","expected")}) for case in data])
    text = json.dumps(packed, ensure_ascii=True, separators=(",", ":"), allow_nan=False) + "\n"
    if args.write: PATH.write_text(text)
    else: assert PATH.read_text() == text
    print(f"{len(data)} complete source-repair dumps match Python")


if __name__ == "__main__": main()

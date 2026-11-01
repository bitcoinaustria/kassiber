from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_durable import evaluate
from generate_analysis_sources import TX, BLOCK, OTHER, assertion, cache, inventory, label, dataset, stored, update, raw, insert
from generate_analysis_ingestion import transaction

DEST = Path(__file__).resolve().parents[1] / 'tests/data/analysis_synchronize.json'


def vectors():
    cases=[]
    sync={'operation':'synchronize'}
    rebuild={'operation':'synchronize','rebuild':True}
    def sql(text):
        return {'operation':'sql','sql':text}
    def add(name, seed='', edits=(), profile='p'):
        case={'name':name,'sql':seed,'profile':profile,'actions':[sync,*edits,sync,rebuild]}
        case['expected']=evaluate(case)
        cases.append(case)
    add('cold-warm-rebuild')
    add('physical-with-labels',stored()+inventory()+cache()+label()+dataset())
    add('other-profile',stored()+cache(),profile='other')
    add('changed-transaction',stored(),[sql(stored(raw_json=raw(vout=[{'value':4,'scriptpubkey':'0014'+'11'*20}])) )])
    add('deleted-transaction',stored(),[sql("DELETE FROM transactions WHERE id='in';")])
    add('moved-rowid',stored(),[sql("UPDATE transactions SET rowid=901 WHERE id='in';")])
    add('withdrawn-inventory',stored()+inventory(),[sql('DELETE FROM wallet_utxos;')])
    add('replaced-cache',cache(),[sql('UPDATE transaction_graph_cache SET payload_json=\'broken\';')])
    add('invalid-source-coverage',cache(payload_json='broken'),[sql('DELETE FROM transaction_graph_cache;')])
    add('invalid-empty-node-change',cache(payload_json='broken'),[sql("UPDATE transaction_graph_cache SET payload_json='null';")])
    add('wallet-configuration',stored()+inventory(),[sql("UPDATE wallets SET config_json='{\"network\":\"regtest\",\"chain_instance_id\":\"lab\"}' WHERE id='cold';")])
    add('wallet-label',stored(),[sql("UPDATE wallets SET label='Renamed' WHERE id='cold';")])
    add('source-profile-change',stored(),[sql("UPDATE transactions SET profile_id='other' WHERE id='in';")])
    add('profile-freshness',stored(),[sql("UPDATE profiles SET journal_input_version=8 WHERE id='p';")])
    add('authored-label-delete',stored()+label(),[sql("UPDATE chain_analysis_labels SET deleted=1;")])
    add('label-subject-change',stored()+label(),[sql("UPDATE chain_analysis_labels SET subject='missing';")])
    add('dataset-revoke',stored()+dataset(),[sql("UPDATE chain_analysis_datasets SET status='revoked';")])
    add('dataset-half-open-expiry',stored()+dataset(claims=[{'subject':f'tx:{TX}','valid_until':'2026-01-02T00:00:00Z'}]),[{'operation':'synchronize','as_of':'2026-01-02T00:00:00Z'}])
    add('dataset-manifest-activation',stored()+dataset(manifest={'valid_from':'2026-01-02T00:00:00Z'}),[{'operation':'synchronize','as_of':'2026-01-02T00:00:00Z'}])
    add('assertion-withdrawal',stored()+assertion(f'{BLOCK}:0'),[sql('UPDATE chain_analysis_reference_assertions SET active=0;')])
    add('assertion-occurrence-change',stored()+assertion(f'{BLOCK}:0'),[sql("UPDATE chain_analysis_reference_assertions SET occurrence_id='"+OTHER+":1',status_json='{\"block_hash\":\""+OTHER+"\"}';")])
    add('assertion-reconciling',stored()+assertion(f'{BLOCK}:0'),[sql('UPDATE chain_analysis_reference_assertions SET active=2;')])
    add('verified-tip-only',stored()+assertion(f'{BLOCK}:0'),[sql('UPDATE chain_analysis_acquisition_grants SET verified_tip_height=123;')])
    add('grant-accounting-no-change',stored()+assertion(f'{BLOCK}:0'),[sql('UPDATE chain_analysis_acquisition_grants SET requests_used=requests_used+1;')])
    add('clock-without-dirty-sources',stored(),[sql('UPDATE chain_index_clock SET revision=revision+1 WHERE id=1;')])
    add('foreign-dirty-key-watermark-only',stored(),[sql("UPDATE transactions SET note='Other' WHERE profile_id='other'; INSERT INTO chain_index_dirty VALUES('transactions',99999,99999); UPDATE chain_index_clock SET revision=99999;")])
    add('repair-dropped-node-table',stored()+label(),[sql('DROP TABLE chain_index_nodes;')])
    add('forced-version-rebuild',stored(),[sql("UPDATE chain_index_state SET version=0 WHERE profile_id='p';")])
    add('book-binding-change',stored(),[sql("INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',1,NULL,'[]','[]','fixed','fixed');")])
    child = update('transactions', {'raw_json': raw(txid=OTHER, vin=[{'txid': TX, 'vout': 0}], vout=[{'value': 9}])}, "id='out'")
    add('known-ancestor-context', stored()+child, [sql(update('transactions', {'note':'changed'}, "id='out'"))])
    physical_only = 'DELETE FROM journal_custody_decisions; DELETE FROM journal_custody_economic_relations;'
    physical_graph = physical_only + stored(raw_json=raw(vin=[{'txid':OTHER,'vout':0}])) + update('transactions', {'raw_json':raw(txid=OTHER,vin=[],vout=[{'value':12}]),'asset':'BTC'}, "id='earn'") + update('transactions', {'raw_json':raw(txid=BLOCK,vin=[{'txid':TX,'vout':0}],vout=[{'value':9}])}, "id='out'")
    add('physical-creator-updates-spender', physical_graph, [sql(stored(raw_json=raw(vin=[{'txid':OTHER,'vout':0}],vout=[{'value':8}])))])
    add('physical-context-outside-core', physical_graph, [sql(update('transactions', {'note':'changed'}, "id='out'"))])
    add('creator-output-invalidates-spender', stored()+child, [sql(stored(raw_json=raw(vout=[{'value':8}])))])
    add('withdrawn-creator-retains-prevout', stored()+child, [sql("DELETE FROM transactions WHERE id='in';")])
    add('custody-global-freshness', stored()+child, [sql("UPDATE profiles SET journal_input_version=99 WHERE id='p';")])
    add('custody-overlay-delete', stored()+child, [sql("DELETE FROM journal_custody_decisions WHERE profile_id='p';")])
    add('custody-overlay-amount', stored()+child, [sql("UPDATE journal_custody_economic_relations SET source_amount_msat=9000 WHERE profile_id='p';")])
    add('physical-retraction', stored()+child, [sql(stored(raw_json=raw(removed=True)))])
    add('tip-with-label-overlay', stored()+label()+assertion(f'{BLOCK}:0'), [sql("UPDATE chain_analysis_acquisition_grants SET verified_tip_height=125; UPDATE chain_analysis_labels SET label='Changed';")])
    add('overlay-only-without-physical-renormalization',stored()+label(),[sql("UPDATE chain_analysis_labels SET label='Changed';")])
    add('assertion-deleted-rebinds-source', stored()+assertion(f'{BLOCK}:0'), [sql('DELETE FROM chain_analysis_reference_assertions;')])
    add('ambiguous-occurrence-withdrawal', stored()+assertion(f'{BLOCK}:0',grant='one')+assertion(f'{OTHER}:1',grant='two'), [sql("DELETE FROM chain_analysis_reference_assertions WHERE grant_id='two';")])
    many = ''.join(insert('transactions', {'id':f'chunk-{i}','workspace_id':'ws','profile_id':'p','wallet_id':'cold','fingerprint':f'chunk-{i}','asset':'BTC','direction':'inbound','amount':i,'occurred_at':'fixed','created_at':'fixed'}) for i in range(301))
    add('physical-source-lookup-chunks', many, [sql("UPDATE transactions SET note='changed' WHERE id LIKE 'chunk-%';")])
    return cases


if __name__=='__main__':
    DEST.write_text(json.dumps(vectors(),sort_keys=True,separators=(',',':'))+'\n')

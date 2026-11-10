
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('sp-none','w','p','SP none','silent-payment','{"sp_descriptor":"  "}','2025'),
 ('sp-pending','w','p','SP pending','silent-payment','{"sp_descriptor":"sp(x)"}','2025'),
 ('sp-degraded','w','p','SP degraded','silent-payment','{"sp_descriptor":0}','2025'),
 ('sp-blocking','w','p','SP blocking','silent-payment','{"sp_descriptor":"sp(y)"}','2025'),
 ('sp-done','w','p','SP done','silent-payment','{"sp_descriptor":"sp(z)"}','2025'),
 ('addr-1','w','p','Address one','address','{"addresses":["bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"],"chain":"bitcoin","network":"mainnet"}','2025'),
 ('addr-2','w','p','Address two','address','{"addresses":["bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"],"chain":"bitcoin","network":"mainnet"}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at,kind,raw_json) VALUES
 ('dup-1','w','p','addr-1','dup-1','abababababababababababababababababababababababababababababababab','BTC','inbound',5000000,0,'2025-05-01T00:00:00Z','2025','deposit','{"txid":"abababababababababababababababababababababababababababababababab","vout":[{"scriptpubkey":"0014c0cebcd6c3d3ca8c75dc5ec62ebe55330ef910e2","value":5000}]}'),
 ('dup-2','w','p','addr-2','dup-2','ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB','BTC','inbound',5000000,0,'2025-05-01T00:00:00Z','2025','deposit','{"txid":"abababababababababababababababababababababababababababababababab","vout":[{"scriptpubkey":"0014c0cebcd6c3d3ca8c75dc5ec62ebe55330ef910e2","value":5000}]}');
INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,stale_reason,blocking_reports,last_error_code,checkpoint_json,updated_at) VALUES
 ('p','rates:p','rates','Rates','blocking_reports','provider_failed',1,'http_error','{}','2025'),
 ('p','onchain_wallet:cold','onchain_wallet','Cold','blocking_reports',NULL,1,NULL,'{}','2025'),
 ('p','onchain_wallet:sp-degraded','onchain_wallet','SP degraded','partially_stale','gap',0,NULL,'{"silent_payment":{"scan_complete":true,"degraded":true,"degraded_reason":"server_gap"}}','2025'),
 ('p','onchain_wallet:sp-blocking','onchain_wallet','SP blocking','blocking_reports','tip_moved',1,NULL,'not json','2025'),
 ('p','onchain_wallet:sp-done','onchain_wallet','SP done','current',NULL,0,NULL,'{"silent_payment":{"scan_complete":1}}','2025');
INSERT INTO journal_quantity_issues(issue_id,workspace_id,profile_id,issue_type,state,asset,amount_msat,occurred_at,reason,blocks_from,created_at) VALUES
 ('q1','w','p','gap','custody_suspense','BTC',1500,'2025-03-01','r','2025-03-01T00:00:00Z','2025'),
 ('q2','w','p','gap','custody_suspense','LBTC',7,'2025-02-01','r','2025-02-01T00:00:00Z','2025'),
 ('q3','w','p','gap','conflicting',NULL,NULL,NULL,'r',NULL,'2025'),
 ('q4','w','p','gap','custody_candidate','BTC',9007199254740993,'2025-04-01','r','','2025');
INSERT INTO journal_quantity_postings(posting_id,workspace_id,profile_id,transaction_id,asset,location_kind,location_id,amount_msat,state,created_at) VALUES
 ('x1','w','p','h','BTC','external','ext',100,'external_presumed','2025'),
 ('x2','w','p','h','BTC','external','ext',200,'external_presumed','2025'),
 ('x3','w','p','j','LBTC','external','ext',5,'external_presumed','2025'),
 ('x4','w','p','j','BTC','wallet','cold',5,'external_presumed','2025');
INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,source,changed_at,journal_input_version_after) VALUES
 ('e1','w','p','h','desktop','2025-06-01T00:00:00Z',3),('e2','w','p','i','ai','2025-06-02T00:00:00Z',4),
 ('e0','w','p','i','ai','2025-05-02T00:00:00Z',1);
INSERT INTO transaction_edit_fields(event_id,field) VALUES('e1','note'),('e1','fiat_rate'),('e2','taxable');
INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,expected_leg_count,expected_allocation_count,state,created_at)
 VALUES('cc','lineage','w','p',1,'transfer',2,0,'active','2025-01-01T00:00:00Z');
INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,ordinal,role,rail,asset,exposure,conservation_unit,amount_msat,transaction_id,anchor_transaction_id,created_at)
 VALUES('leg','cc','w','p',0,'source','bitcoin','BTC','asset','BTC',1000,'h','h','2025-01-01T00:00:00Z');
UPDATE profiles SET ownership_review_counts_json='{"total": 3, "by_reason": {"b": 2, "a": 1, "z": 0}}' WHERE id='p';

UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=15,journal_input_version=2,
 last_processed_input_version=2,journal_semantics_version=1 WHERE id='p';
UPDATE profiles SET required_journal_semantics_version=0 WHERE id='p';

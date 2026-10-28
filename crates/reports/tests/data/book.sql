INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','fixed');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,last_processed_at,journal_input_version,last_processed_input_version,last_processed_tx_count,created_at) VALUES('p','ws','Book','EUR','generic','2026-01-01T00:00:00Z',7,7,4,'fixed'),('other','ws','Other','USD','generic',NULL,0,0,0,'fixed');
INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','p'),('database_instance_id','0123456789abcdef0123456789abcdef');
INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('acct','ws','p','treasury','Treasury','asset','fixed');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,account_id,created_at) VALUES('cold','ws','p','Cold','custom','{}','acct','fixed'),('hot','ws','p','Hot','custom','{}','acct','fixed'),('other','ws','other','Other','custom','{}',NULL,'fixed');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,fiat_rate,fiat_rate_exact,fiat_value,fiat_value_exact,excluded,occurred_at,created_at,note,kind,taxability_override) VALUES
('in','ws','p','cold','in','BTC','inbound',123456789001,123,30000,'30000.12345678901234567890123456789',NULL,NULL,0,'2024-01-01T00:00:00Z','a',' Has note ','deposit',NULL),
('out','ws','p','cold','out','BTC','outbound',123456789,456,40000,NULL,NULL,NULL,0,'2024-12-31T23:30:00Z','b','','withdrawal',NULL),
('earn','ws','p','hot','earn','LBTC','inbound',789001,789,NULL,NULL,0.123,'0.123456789123456789',0,'2025-01-02T00:00:00Z','c',NULL,'interest',NULL),
('no-tax','ws','p','hot','no-tax','BTC','outbound',100001,0,60000,'60000.123',NULL,NULL,0,'2025-02-01T00:00:00Z','d',NULL,'withdrawal',0),
('excluded','ws','p','cold','excluded','BTC','inbound',999999,9,90000,'90000',NULL,NULL,1,'2026-02-01T00:00:00Z','e','Excluded','deposit',NULL);
INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('tag1','ws','p','one','One','fixed'),('tag2','ws','p','two','Two','fixed');
INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('in','tag1'),('in','tag2'),('excluded','tag1');
INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,entry_type,asset,quantity,fiat_value,cost_basis,proceeds,gain_loss,at_category,at_kennzahl,capital_gains_type,description,created_at) VALUES
('z','ws','p','out','cold','acct','2024-12-31T23:30:00Z','disposal','BTC',-100000001,12.34,10.00000001,12.34,2.33999999,NULL,NULL,'long_term','Split z','same'),
('a','ws','p','out','cold','acct','2024-12-31T23:30:00Z','disposal','BTC',-23456788,2.33,1.23,2.33,1.1,'neu_gain',174,'short_term','Split a','same'),
('first','ws','p','out','cold','acct','2024-12-31T23:30:00Z','disposal','BTC',-3,0,0,0,-0.0,'alt_taxfree',NULL,NULL,'Earlier creation','before'),
('income','ws','p','earn','hot','acct','2025-01-02T00:00:00Z','income','LBTC',789001,0.123,0.123,0.123,0.123,'income_general',172,'income','Income €','fixed'),
('hidden','ws','p','no-tax','hot','acct','2025-02-01T00:00:00Z','disposal','BTC',-100001,100,10,100,90,NULL,NULL,'short_term',NULL,'fixed'),
('swap','ws','p','out','cold','acct','2024-12-31T23:30:00Z','disposal','BTC',-100001,100,10,100,90,'neu_swap',NULL,'short_term',NULL,'fixed'),
('fee','ws','p','out','cold','acct','2024-12-31T23:30:00Z','fee','BTC',-456,0.01,0.01,0,0,NULL,NULL,NULL,'Fee','fixed');
INSERT INTO journal_account_holdings(id,workspace_id,profile_id,account_code,account_label,asset,quantity,cost_basis,created_at) VALUES
('az','ws','p','treasury','Treasury','BTC',111111111111,12345.67890123456,'fixed'),
('aa','ws','p','treasury','Treasury','BTC',1,0.0000001,'fixed'),
('l','ws','p','other','Other','LBTC',789001,0.1234567891234,'fixed'),
('zero','ws','p',NULL,'Fallback','BTC',0,5,'fixed'),
('negative','ws','p',NULL,'Fallback','BTC',-1,5,'fixed'),
('fallback','ws','p',NULL,'Fallback','OTHER',100000000001,9876.54321098765,'fixed');
INSERT INTO journal_wallet_holdings(id,workspace_id,profile_id,wallet_id,wallet_label,account_code,asset,quantity,cost_basis,created_at) VALUES
('wz','ws','p','cold','Cold','treasury','BTC',111111111111,12345.67890123456,'fixed'),
('wa','ws','p','cold','Cold','treasury','BTC',1,0.0000001,'fixed'),
('wh','ws','p','hot','Hot','treasury','LBTC',789001,0.1234567891234,'fixed'),
('wn','ws','p','hot','Hot','treasury','BTC',-1,7,'fixed'),
('w0','ws','p','hot','Hot','treasury','BTC',0,7,'fixed'),
('wo','ws','p','other','Other','treasury','BTC',100000000001,1,'fixed');
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('no-tax','ws','p','missing_price','fixed');
INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,target_occurred_at,created_at) VALUES
(printf('%064d',1),'ws','p','out','earn',printf('%064d',2),0,10001,printf('%064d',3),0,10001,'cold','hot','mainnet','mainnet','bitcoin','liquid','BTC','LBTC','internal_reviewed','eligible','reviewed','2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','b'),
(printf('%064d',4),'ws','p','out','in',printf('%064d',2),10001,20002,printf('%064d',3),10001,20002,'cold','cold','mainnet','mainnet','bitcoin','bitcoin','BTC','BTC','internal_reviewed','eligible','reviewed','2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','a');
INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,target_occurred_at,created_at) VALUES(printf('%064d',5),'ws','p','direct_payout','out','BTC','LBTC',10001,9999,'eligible','2024-12-31T23:30:00Z',NULL,'c');

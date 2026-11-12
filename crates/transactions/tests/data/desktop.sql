INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025'),('foreign','w','Other','EUR','at','2025');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('cold','w','p','Cold','address','{"network":"regtest","descriptor":"private-config"}','2025'),
 ('ln','w','p','Lightning','lnd','{}','2025'),
 ('exchange','w','p','Exchange','kraken','{}','2025'),
 ('liquid','w','p','Liquid','liquid','{"chain":"liquid","network":"test"}','2025'),
 ('custom','w','p','Custom','custom','bad','2025'),
 ('ambiguous','w','p','cold','custom','{}','2025'),
 ('foreign','w','foreign','Foreign','address','{}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,kind_override,fiat_rate,fiat_value,review_status,note,description,counterparty,fiat_currency) VALUES
 ('a','w','p','cold','a','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','BTC','inbound',9007199254740995,1,'2025-02-01T00:00:00Z','2025','2025-02-02','deposit','income',3,7,'completed','api_key=synthetic-private-token','Synced from private','merchant','EUR'),
 ('b','w','p','ln','b','payment-hash','BTC','inbound',100001,0,'2025-02-01T00:00:00Z','2025','2025-02-02','deposit',NULL,2,3,'completed','','','',NULL),
 ('c','w','p','exchange','c',NULL,'BTC','outbound',100001,1,'2025-02-01T00:00:00Z',NULL,'2025-02-01','withdrawal',NULL,0,0,'completed','memo','','',NULL),
 ('d','w','p','liquid','d','efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef','LBTC','outbound',0,1001,'2025-01-01T00:00:00Z',NULL,'2025-02-01','peg-in',NULL,4,NULL,'review','','','',NULL),
 ('e','w','p','custom','e','external-e','BTC','inbound',100001,0,'2025-02-01T00:00:00.000001Z',NULL,'2025-02-01','swap',NULL,NULL,NULL,NULL,'','unique-description-query','',NULL),
 ('identifier-only-record','w','p','ambiguous','f','external-f','BTC','inbound',100001,2,'2025-02-01T00:00:00.000002Z',NULL,'2025-02-01','transfer',NULL,1,NULL,'failed','word','Silent Payment sync from secret','',NULL),
 ('g','w','p','cold','g','external-g','BTC','outbound',100001,3,'2025-01-02T00:00:00Z',NULL,'2025-02-01','deposit',NULL,1,1,'review','','','',NULL),
 ('foreign','w','foreign','foreign','foreign','external','BTC','inbound',1,5,'2025-03-01',NULL,'2025','deposit',NULL,1,1,'completed','','','',NULL);
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('c','w','p','missing_fee_price','2025');
INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('z','w','p','z','Last','2025'),('a','w','p','a','First','2025');
INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('a','z'),('a','a');
INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');

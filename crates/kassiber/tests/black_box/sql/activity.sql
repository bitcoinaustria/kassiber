
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,fiat_rate,fiat_value,review_status,note,description,counterparty) VALUES
 ('h','w','p','cold','h',NULL,'BTC','outbound',5000000,1000,'2025-03-05T10:00:00Z','2025','2025-03-05','withdrawal',50000,2.5,NULL,'','',''),
 ('i','w','p','ln','i',NULL,'BTC','inbound',4999000,0,'2025-03-05T10:30:00Z','2025','2025-03-05','deposit',50000,NULL,NULL,'','',''),
 ('j','w','p','cold','j',NULL,'BTC','outbound',20000000,0,'2025-04-10T12:00:00Z','2025','2025-04-10','peg-in',60000,12,NULL,'','',''),
 ('k','w','p','liquid','k',NULL,'LBTC','inbound',19990000,0,'2025-04-10T12:20:00Z','2025','2025-04-10','peg-in',60000,11.99,NULL,'','',''),
 ('l','w','p','exchange','l',NULL,'BTC','inbound',300000000,0,'2024-10-27T00:30:00Z','2024','2024-10-27','buy',40000,120,'completed','','',''),
 ('m','w','p','exchange','m',NULL,'BTC','inbound',2000000,0,'2023-12-31T23:30:00Z','2024','2024-01-01','buy',30000,NULL,'pending','','','');

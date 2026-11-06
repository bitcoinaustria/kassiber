-- The tables `quarantine_review.quarantine_membership` reads, with a root (a)
-- two rows wait on, a root of its own (t), a downstream row with no nameable
-- root (i), a downstream row naming a transaction that is not held (u), and
-- an unheld transaction (n) a test quarantines later.
CREATE TABLE wallets(id TEXT, label TEXT);
CREATE TABLE transactions(id TEXT, profile_id TEXT, wallet_id TEXT, occurred_at TEXT,
    asset TEXT, direction TEXT, amount INTEGER, fee INTEGER, external_id TEXT,
    excluded INTEGER DEFAULT 0);
CREATE TABLE journal_quarantines(transaction_id TEXT, profile_id TEXT, reason TEXT, detail_json TEXT);
INSERT INTO wallets VALUES('w1','Cold'),('w2','Hot');
INSERT INTO transactions(id,profile_id,wallet_id,occurred_at,asset,direction,amount,fee,external_id) VALUES
    ('a','p','w1','2024-01-01T00:00:00Z','BTC','inbound',100000,0,'ea'),
    ('b','p','w2','2024-02-01T00:00:00Z','BTC','Outbound',5000,7,NULL),
    ('s','p','w2','2024-03-01T00:00:00Z','LBTC','inbound',300,1,''),
    ('i','p','w1','2024-01-15T00:00:00Z','BTC','outbound',40,2,'ei'),
    ('t','p','w1','2023-12-01T00:00:00Z','BTC','inbound',9,0,'et'),
    ('u','p','w1','2024-04-01T00:00:00Z','BTC','inbound',11,NULL,'eu'),
    ('n','p','w1','2024-04-02T00:00:00Z','BTC','inbound',1,0,'en');
INSERT INTO journal_quarantines VALUES
    ('a','p','missing_spot_price','{}'),
    ('b','p','transfer_pair_dependency_blocked','{"blocked_by_transaction_ids":["a"]}'),
    ('s','p','custody_basis_barrier','{"root_transaction_ids":["a"]}'),
    ('i','p','basis_provenance_incomplete','{}'),
    ('t','p','insufficient_lots','not json'),
    ('u','p','custody_basis_barrier','{"root_transaction_ids":["zzz"]}'),
    ('a','other','missing_spot_price','{}');

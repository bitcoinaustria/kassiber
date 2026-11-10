
INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,loan_id,role,note,created_at) VALUES
 ('loan-1','w','p','h','loan','collateral','/home/user/loan.pdf','2025'),
 ('loan-2','w','p','i','loan','returned',NULL,'2025'),
 ('loan-3','w','p','k','terms','collateral','https://lender.invalid/terms','2025');
INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,source,changed_at,journal_input_version_after)
 VALUES('e9','w','p','j','gui','2025-07-01T00:00:00Z',9);


INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id,created_at) VALUES
 ('at-good','w','p','h','file','Board decision','Grüße Board/../minutes 2024.pdf','p/good.pdf',NULL,'application/pdf',4,'315d429b7714cedb6ad04ac31240145257692630457f3c88253c5beceac76027','at-file','i','2025-01-11'),
 ('../at odd/ü','w','p','h','file',NULL,'  ','p/good.pdf',NULL,NULL,NULL,NULL,NULL,NULL,'2025-01-12'),
 ('at-unhashed','w','p','i','file','Receipt','áb.txt','p/receipt.txt',NULL,'text/plain',NULL,NULL,NULL,NULL,'2025-01-13'),
 ('at-blank','w','p','i','file','Blank path',NULL,'  ',NULL,NULL,NULL,NULL,NULL,NULL,'2025-01-14');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-00','w','p','k','url',NULL,'https://shop.example/inv?Api%5FKey=s&x=+y&z','2025-02-01');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-01','w','p','k','url',NULL,'https://bob:pw@Docs.Example.COM:8443/doc;v=1?id=1#x','2025-02-02');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-02','w','p','k','url',NULL,'http://h.example/a;#frag?token=1&b=%E2%82%AC','2025-02-03');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-03','w','p','k','url',NULL,'https://h.example/p;?sig=1','2025-02-04');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-04','w','p','k','url',NULL,'HTTPS://user@[::1]:80/x?password=&q=a b','2025-02-05');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-05','w','p','k','url',NULL,'ftp://u:@host/f#key','2025-02-06');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-06','w','p','k','url',NULL,'mailto:x@example.com?subject=hi&auth=1','2025-02-07');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-07','w','p','k','url',NULL,'example.com/path?secret=1','2025-02-08');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-08','w','p','k','url',NULL,'//host/p?token=é%zz&ü=1','2025-02-09');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-09','w','p','k','url',NULL,'https://h/x#?a=1&b','2025-02-10');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-10','w','p','k','url',NULL,'  https://h/x?a=%2B&Bearer=1  ','2025-02-11');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-11','w','p','k','url',NULL,'custom:opaque;p?api_key=1','2025-02-12');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-12','w','p','k','url',NULL,'https://h/x?&&a&=b&passwordless','2025-02-13');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-13','w','p','k','url',NULL,'https://Grüße.example/€?KEY=~v*','2025-02-14');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) VALUES('url-14','w','p','k','url',NULL,'https://h/x#plain','2025-02-15');

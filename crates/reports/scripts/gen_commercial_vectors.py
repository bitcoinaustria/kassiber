#!/usr/bin/env python3
"""Freeze the reviewed commercial subledger and its CSV on disposable books."""
from pathlib import Path
import json
import sys
import tempfile

from kassiber.cli.handlers import _commercial_hooks
from kassiber.core import commercial
from kassiber.db import open_db
from kassiber.errors import AppError
from gen_completion_vectors import q

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/commercial.json"

SEED = f"""
INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,external_ref,raw_json,created_at,updated_at)
VALUES('doc','ws','p','invoice',{q('Invoice €: comma, quote " and CR' + chr(13) + chr(10) + 'LF')},'INV-17','{{"private":"not projected"}}','fixed','fixed');
INSERT INTO btcpay_provenance_records(id,workspace_id,profile_id,store_id,record_type,stable_key,invoice_id,payment_id,order_id,payment_request_id,origin_kind,origin_label,raw_json,created_at,updated_at)
VALUES('payment','ws','p','store','payment','stable','invoice','payment','order','request','pos','Café','{{"private":"not projected"}}','fixed','fixed');
INSERT INTO commercial_links(id,workspace_id,profile_id,document_id,transaction_id,link_type,state,commercial_kind,allocation_amount,allocation_fiat_exact,reconciliation_state,created_at,updated_at)
VALUES('reviewed','ws','p','doc','in','document_transaction','reviewed','income',7,'0.0001','mismatch','z','fixed'),
('earlier','ws','p','doc','in','other','reviewed',NULL,NULL,NULL,'matched','a','fixed'),
('suggested','ws','p','doc','out','document_transaction','suggested','expense',1,'9','unreviewed','fixed','fixed'),
('rejected','ws','p','doc','earn','document_transaction','rejected','income',1,'9','ignored','fixed','fixed'),
('excluded','ws','p','doc','excluded','document_transaction','reviewed','none',1,'9','matched','fixed','fixed'),
('document-only','ws','p','doc',NULL,'other','reviewed','none',NULL,NULL,'matched','fixed','fixed');
INSERT INTO commercial_links(id,workspace_id,profile_id,btcpay_record_id,transaction_id,link_type,state,commercial_kind,created_at,updated_at)
VALUES('merchant','ws','p','payment','out','btcpay_payment_transaction','reviewed','expense','fixed','fixed');
"""


def specs():
    cases = [("empty", "", {}), ("reviewed", SEED, {}), ("other-scope", SEED, {"profile": "Other"}),
             ("unknown-profile", SEED, {"profile": "absent"}),
             ("unknown-workspace", SEED, {"workspace": "absent"}),
             ("missing-context", SEED + "DELETE FROM settings WHERE key LIKE 'context_%';", {}),
             ("stale", SEED + "UPDATE profiles SET last_processed_at=NULL,journal_input_version=99;", {}),
             ("unsupported-tax-country", SEED + "UPDATE profiles SET tax_country='zz';", {}),
             ("no-journals", SEED + "DELETE FROM journal_entries;", {}),
             ("null-strings", SEED + "UPDATE transactions SET external_id=NULL,fiat_currency=NULL,fiat_value_exact=NULL,pricing_source_kind=NULL;", {}),
             ("exact-values", SEED + "UPDATE transactions SET amount=1001,fee=999,fiat_value_exact='1.2300000000000000000000001',pricing_source_kind='reviewed' WHERE id='in';", {}),
             ("large-signed-values", SEED + "UPDATE transactions SET amount=-9223372036854775807,fee=9223372036854775807 WHERE id='in';", {}),
             ("review-state", SEED + "UPDATE commercial_links SET state='suggested' WHERE id='reviewed'; UPDATE commercial_links SET state='reviewed' WHERE id='suggested';", {})]
    many = "".join(f"INSERT INTO commercial_links(id,workspace_id,profile_id,document_id,transaction_id,link_type,state,created_at,updated_at) VALUES('many{i}','ws','p','doc','in','many{i}','reviewed','{300-i:03d}','fixed');" for i in range(205))
    cases.append(("unbounded", SEED + many, {}))
    return cases


def build():
    result = []
    for name, sql, args in specs():
        with tempfile.TemporaryDirectory(prefix="commercial-vector-") as root:
            conn = open_db(Path(root) / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + sql)
            try:
                rows = commercial.build_reviewed_subledger_rows(conn, args.get('workspace'), args.get('profile'), _commercial_hooks())
                path = Path(root) / 'export.csv'
                exported = commercial.export_reviewed_subledger_csv(conn, args.get('workspace'), args.get('profile'), path, _commercial_hooks())
                expected = dict(ok=rows, exported=exported, csv=path.read_bytes().decode())
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
            result.append(dict(name=name, sql=sql, args=args, expected=json.loads(json.dumps(expected).replace(root, '<root>'))))
            conn.close()
    return json.dumps(result, ensure_ascii=True, indent=2) + '\n'


if __name__ == '__main__':
    content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'commercial vectors drifted'
    else:
        OUT.write_text(content)

#!/usr/bin/env python3
"""Freeze synthetic SQL report vectors from Python; never open a user book."""
from pathlib import Path
import json
import tempfile

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports, custody_filed_reports
from kassiber.daemon import _reports_balance_sheet_payload, _reports_portfolio_summary_payload
from kassiber.db import open_db
from kassiber.envelope import json_ready
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / "crates/reports/tests/data/book.sql"
OUT = BASE.with_name("vectors.json")


def build():
    specs = []
    def case(name, group, command, sql="", args=None):
        specs.append((name, group, command, sql, args or {}))
    from gen_history_vectors import add_cases
    history_args = add_cases(case)
    for name, command in (("balance", "balance-sheet"), ("holdings", "portfolio-summary"), ("gains", "capital-gains"), ("summary", "summary")):
        case(name, "holdings" if name in {"balance", "holdings"} else name, f"reports.{command}")
    for args in ({"wallet":"Cold"},{"wallet":"  cold  "},{"wallet":"HOT"},{"wallet":"absent"}):
        case(f"summary-{args['wallet']}", "summary", "reports.summary", args=args)
    case("summary-ambiguous", "summary", "reports.summary", "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('duplicate','ws','p','COLD','custom','{}','fixed');", {"wallet":"cold"})
    case("summary-text", "text", "reports.summary", args={"text":True})
    case("summary-text-scoped", "text", "reports.summary", args={"text":True,"wallet":"Cold"})
    case("summary-text-empty-flow", "text", "reports.summary", "UPDATE transactions SET excluded=1; UPDATE profiles SET last_processed_tx_count=0 WHERE id='p';", {"text":True})
    case("summary-multiple-inbound", "summary", "reports.summary", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at,kind) VALUES('in2','ws','p','cold','in2','BTC','inbound',100001,1,'2023-01-01T00:00:00Z','fixed','deposit'); UPDATE profiles SET last_processed_tx_count=5 WHERE id='p';")
    case("summary-null-account-sort", "summary", "reports.summary", "UPDATE journal_wallet_holdings SET account_code=NULL,cost_basis=10000000000000000 WHERE id='wz'; UPDATE journal_wallet_holdings SET account_code=NULL,cost_basis=0 WHERE id='wa'; UPDATE journal_wallet_holdings SET account_code='A',cost_basis=1 WHERE id='wh'; UPDATE journal_wallet_holdings SET account_code='B',cost_basis=1 WHERE id='wo';")
    cache = "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at,granularity,method) VALUES('BTC-EUR','2026-01-01T00:00:00Z',80000,'80000.12345678901234567890123456789','manual','fixed','exact','manual');"
    for name in ("balance-sheet","portfolio-summary"):
        case(f"cached-{name}","holdings",f"reports.{name}",cache)
        case(f"value-only-{name}","holdings",f"reports.{name}","UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE asset='BTC'; UPDATE transactions SET fiat_value=400,fiat_value_exact='400.1234567890123456789012345' WHERE id='out';")
        case(f"no-cache-table-{name}","holdings",f"reports.{name}","DROP TABLE rates_cache;")
    def quotes(values):
        return "".join("INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','%s',%d,'%s','%s');" % value for value in values)
    for label,values in (
        ("timestamp",[("2025",100,"manual","2026"),("2026",200,"provider","2025")]),
        ("manual",[("2026",100,"manual","2024"),("2026",200,"provider","2026")]),
        ("fetched",[("2026",100,"first","2025"),("2026",200,"second","2026")]),
        ("source",[("2026",100,"z-provider","2025"),("2026",200,"a-provider","2025")]),
    ):
        case("cache-priority-"+label,"holdings","reports.balance-sheet",quotes(values))
    case("half-even-market-minus-basis", "holdings", "reports.balance-sheet", "DELETE FROM journal_account_holdings; INSERT INTO journal_account_holdings(id,workspace_id,profile_id,account_code,account_label,asset,quantity,cost_basis,created_at) VALUES('tie','ws','p','treasury','Treasury','BTC',100000000000,1,'fixed'); INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2026',1,'1.00000000000000000000000000000005','manual','fixed');")
    for kind, fn in (("balance_sheet",_reports_balance_sheet_payload),("portfolio_summary",_reports_portfolio_summary_payload)):
        case(kind,"desktop",f"ui.reports.{kind}")
    case("fresh-with-excluded", "readiness", "reports.capital-gains")
    for name, sql in (
        ("missing-timestamp","UPDATE profiles SET last_processed_at=NULL WHERE id='p';"),
        ("stale-count","UPDATE profiles SET last_processed_tx_count=3 WHERE id='p';"),
        ("stale-input","UPDATE profiles SET journal_input_version=8 WHERE id='p';"),
        ("unsupported-country","UPDATE profiles SET tax_country='xx' WHERE id='p';"),
        ("missing-quantity-state","DROP TABLE journal_quantity_issues;"),
        ("mixed-network","UPDATE wallets SET config_json='{\"chain\":\"bitcoin\",\"network\":\"testnet\"}' WHERE id='cold'; UPDATE wallets SET config_json='{\"chain\":\"bitcoin\",\"network\":\"mainnet\"}' WHERE id='hot';"),
        ("component-blocked","INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('incomplete','lineage','ws','p',1,'migration','active','fixed');"),
    ):
        case(name,"readiness","reports.capital-gains",sql)
    issues = "".join("INSERT INTO journal_quantity_issues(issue_id,workspace_id,profile_id,issue_type,state,asset,amount_msat,occurred_at,reason,blocks_from,created_at) VALUES('%s','ws','p','suspense','custody_suspense','BTC',%d,'%s','missing custody','%s','fixed');" % (f"issue-{i:02d}",i+1,"2025-01-01" if i%2 else "2024-01-01", f"blocked-{i:02d}") for i in reversed(range(25)))
    case("quantity-prior-to-staleness","readiness","reports.capital-gains",issues+"UPDATE profiles SET last_processed_at=NULL;")
    case("component-prior-to-quantity","readiness","reports.capital-gains",issues+"INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('incomplete','lineage','ws','p',1,'migration','active','fixed');")
    snapshots = "".join("INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,report_scope_json,created_at) VALUES('%s','ws','p','summary','%s',%d,%d,'%s','%s','{\"gain\":\"1.2300\"}','{\"wallet_ids\":[\"cold\"]}','%s');" % (name,state,start,end,"a"*64,classification,at) for name,state,start,end,classification,at in [("z","filed",2025,2025,"{}","same"),("a","saved",2025,2025,"not json","same"),("x","saved",2024,2026,"[]","early"),("b","filed",2024,2025,'{\"kind\":\"sale\"}',"late")])
    case("snapshots-ignore-live-staleness","snapshots","reports.filed-snapshots.list",snapshots+"UPDATE profiles SET last_processed_at=NULL;")
    # Stored report continuation: category mapping, per-row rounding and totals.
    at = "UPDATE profiles SET tax_country='at' WHERE id='p';"
    for command in ('austrian-e1kv', 'austrian-tax-summary'):
        for name, sql in (('populated', at), ('empty', at + 'DELETE FROM journal_entries;')):
            case('csv-' + command + '-' + name, 'csv', 'reports.' + command,
                 sql, {'year': 2025, 'csv': True})
    tax = "INSERT INTO journal_tax_summary(id,workspace_id,profile_id,year,asset,transaction_type,capital_gains_type,quantity,proceeds,cost_basis,gain_loss,created_at) VALUES('tax','ws','p',2024,'BTC','sell','short_term',123556789,102.33,11.23,91.1,'fixed'),('tax2','ws','p',2025,'BTC','sell','short_term',100001,100,10,90,'fixed'),('tax3','ws','p',2025,'LBTC','interest','income',789001,0.123,0.123,0.123,'fixed');"
    for label,sql in (("generic",tax),("vienna",at),("empty","DELETE FROM journal_entries;"),("adjustment-zero",tax+"DELETE FROM journal_entries WHERE id='a';"),("kind-override",at+"UPDATE transactions SET kind_override='  MINING_REWARD ' WHERE id='earn';"),("null-gains",at+"UPDATE journal_entries SET capital_gains_type=NULL;"),("stale",at+"UPDATE profiles SET last_processed_at=NULL;")):
        case('tax-'+label,'tax','reports.tax-summary',sql)
    # Each recognized income kind has its own effective-kind bucket.
    for kind in ('airdrop','hard_fork','hardfork','income','interest','lending_interest','mining','mining_reward','routing_income','staking','unknown'):
        case('tax-kind-'+kind,'tax','reports.tax-summary',at+f"UPDATE transactions SET kind_override='{kind}' WHERE id='earn';")
    for label,sql in (("base",at),("country",""),("currency",at+"UPDATE profiles SET fiat_currency='USD';"),("category",at+"UPDATE journal_entries SET at_category='unknown' WHERE id='a';"),("rounding",at+"UPDATE journal_entries SET proceeds=1.005,cost_basis=2.005,gain_loss=-1.005,at_category='neu_loss',at_kennzahl=175 WHERE id='a';"),("stale",at+"UPDATE profiles SET last_processed_at=NULL;"),("validation-before-stale","UPDATE profiles SET last_processed_at=NULL;"),("aliases",at+"UPDATE journal_entries SET at_category='alt_spekulation' WHERE id='a'; UPDATE transactions SET kind_override='routing_income' WHERE id='earn';")):
        case('at-'+label,'austrian','reports.austrian-e1kv',sql,{'year':2025})
    for category in reports.AUSTRIAN_E1KV_CATEGORY_LABELS:
        case('at-category-'+category,'austrian','reports.austrian-e1kv',at+f"UPDATE journal_entries SET at_category='{category}',at_kennzahl=175 WHERE id='a';",{'year':2025})
    case('at-without-lbtc','austrian','reports.austrian-e1kv',at+"DELETE FROM journal_entries WHERE asset='LBTC';",{'year':2025})
    for year in (2008,2009,2024,2026,2100,2101,99999999999999999999999999):
        case('at-year-'+str(year),'austrian','reports.austrian-tax-summary',at,{'year':year})
    case('at-text','austrian-text','reports.austrian-e1kv',at,{'year':2025,'text':True})
    case('at-text-loss','austrian-text','reports.austrian-tax-summary',at+"UPDATE journal_entries SET proceeds=1.005,cost_basis=2.005,gain_loss=-1.005,at_category='neu_loss',at_kennzahl=175 WHERE id='a';",{'year':2025,'text':True})
    case('at-text-quantity-ties','austrian-text','reports.austrian-e1kv',at+"UPDATE journal_entries SET quantity=500 WHERE id IN ('a','income');",{'year':2025,'text':True})
    for year in (None,2024,2025,2026):
        case('desktop-tax-'+str(year),'desktop-tax','ui.reports.tax_summary',at,{'year':year})
    case('desktop-tax-generic-years','desktop-tax','ui.reports.tax_summary',tax)
    case('desktop-tax-negative-year','desktop-tax','ui.reports.tax_summary',tax+"UPDATE journal_tax_summary SET year=-1 WHERE id='tax';")
    for label,sql in (("all",""),("stale","UPDATE profiles SET last_processed_at=NULL;"),("nullable","UPDATE journal_entries SET account_id=NULL,description=NULL,at_category='',at_kennzahl=0 WHERE id='a';"),("empty","DELETE FROM journal_entries;"),("profile-scope","UPDATE journal_entries SET profile_id='other' WHERE id='a';")):
        case('entries-'+label,'entries','reports.journal-entries',sql)
    for label,sql in (
        ('fees-tagged',at+"UPDATE journal_entries SET at_category='neu_gain' WHERE entry_type='fee';"),
        ('non-taxable-tagged',at+"UPDATE journal_entries SET at_category='neu_gain' WHERE id='hidden';"),
        ('scope',at+"UPDATE journal_entries SET profile_id='other' WHERE id='a';"),
        ('note-external-id',at+"UPDATE transactions SET external_id='external|id',note='Note\nwith|pipe',kind_override='routing_income' WHERE id='out';"),
        ('quarantine-order',at+"INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('out','ws','p','z','fixed'),('earn','ws','p','z','fixed'),('in','ws','p','a','fixed');"),
        ('gate-before-components',"INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('incomplete','lineage','ws','p',1,'migration','active','fixed');"),
        ('same-category-split',at+"UPDATE journal_entries SET at_category='neu_gain',gain_loss=0.005 WHERE id IN ('a','first','z');"),
        ('income-loss-code',at+"UPDATE journal_entries SET at_category='neu_loss',gain_loss=-1.005 WHERE id='income';"),
    ):
        case('at-'+label,'austrian','reports.austrian-e1kv',sql,{'year':2025})
        case('at-text-'+label,'austrian-text','reports.austrian-e1kv',sql,{'year':2025,'text':True})
    # A persisted projection fixture isolates fee selection from custody composition.
    fee_table = "DROP VIEW journal_custody_projection_relations; CREATE TABLE journal_custody_projection_relations(profile_id TEXT,kind TEXT,policy TEXT,swap_fee_msat INTEGER,out_asset TEXT,in_asset TEXT,target_occurred_at TEXT,out_transaction_id TEXT);"
    for kind in (*reports.SWAP_FEE_PAIR_KINDS,'manual'):
        for country in ('generic','at'):
            sql=fee_table+f"UPDATE profiles SET tax_country='{country}' WHERE id='p'; INSERT INTO journal_custody_projection_relations VALUES('p','{kind}','carrying-value',1001,'BTC','BTC',NULL,'out'),('p','manual','taxable',2002,'BTC','LBTC','2025-01-02T00:00:00Z','out'),('p','manual','taxable',NULL,'BTC','LBTC',NULL,'out'),('other','manual','taxable',9000,'BTC','LBTC',NULL,'out');"
            case('tax-fees-'+kind+'-'+country,'tax','reports.tax-summary',sql)
    case('tax-fees-only','tax','reports.tax-summary',fee_table+"INSERT INTO journal_custody_projection_relations VALUES('p','manual','carrying-value',0,'l-btc','LBTC',NULL,'out');")
    for field in ('quantity','proceeds','cost_basis','gain_loss'):
        sql=tax+"UPDATE journal_entries SET capital_gains_type='short_term' WHERE id='hidden'; UPDATE journal_tax_summary SET "+field+"="+('100001' if field=='quantity' else '0.00000001')+" WHERE id='tax3';"
        case('tax-nonzero-'+field,'tax','reports.tax-summary',sql)
    # Main sync 1 (#706): the crypto Kennzahlen start on 1 March 2022 in Vienna;
    # earlier rows are stated in no field and the E 1kv refuses them.
    later = "UPDATE journal_entries SET occurred_at='2022-05-10T10:00:00Z' WHERE id IN ('z','first','swap');"
    for label, sql, year in (
        ('early', at+later+"UPDATE journal_entries SET occurred_at='2022-02-10T10:00:00Z' WHERE id IN ('a','income');", 2022),
        ('boundary', at+later+"UPDATE journal_entries SET occurred_at='2022-02-28T22:59:59Z' WHERE id='a'; UPDATE journal_entries SET occurred_at='2022-05-10T10:00:00Z' WHERE id='income';", 2022),
        ('first-day', at+later+"UPDATE journal_entries SET occurred_at='2022-02-28T23:00:00Z' WHERE id IN ('a','income');", 2022),
        ('alt-early', at+"UPDATE journal_entries SET occurred_at='2022-02-10T10:00:00Z',at_category='alt_spekulation',at_kennzahl=801 WHERE id='a';", 2022),
        ('old', at+"UPDATE journal_entries SET occurred_at='2021-06-01T10:00:00Z';", 2021),
        ('old-empty', at+"DELETE FROM journal_entries;", 2021),
    ):
        for command in ('austrian-e1kv', 'austrian-tax-summary'):
            case(f'kz2022-{label}-{command}', 'austrian', 'reports.'+command, sql, {'year': year})
        case(f'kz2022-csv-{label}', 'csv', 'reports.austrian-e1kv', sql, {'year': year, 'csv': True})
        case(f'kz2022-text-{label}', 'austrian-text', 'reports.austrian-e1kv', sql, {'year': year, 'text': True})
        case(f'kz2022-gains-{label}', 'gains', 'reports.capital-gains', sql)
    for label, sql in (
        ('text', "UPDATE journal_entries SET at_kennzahl='174' WHERE id='a';"),
        ('text-early', "UPDATE journal_entries SET at_kennzahl='174',occurred_at='2021-05-01T00:00:00Z' WHERE id='a';"),
        ('empty-text', "UPDATE journal_entries SET at_kennzahl='' WHERE id='a';"),
        ('float', "UPDATE journal_entries SET at_kennzahl=174.0 WHERE id='a';"),
        ('real-text', "UPDATE journal_entries SET at_kennzahl='174.5' WHERE id='a';"),
        ('padded', "UPDATE journal_entries SET at_kennzahl=' 801 ',occurred_at='2019-05-01T00:00:00Z' WHERE id='a';"),
        ('generic-early', "UPDATE journal_entries SET occurred_at='2020-05-01T00:00:00Z' WHERE id='a';"),
    ):
        case(f'kz2022-gains-stored-{label}', 'gains', 'reports.capital-gains', sql)
    case('at-zero-quantity','austrian','reports.austrian-e1kv',at+"UPDATE journal_entries SET quantity=0 WHERE id='a';",{'year':2025})
    case('at-year-required','austrian','reports.austrian-e1kv',at)
    case('at-year-malformed','austrian','reports.austrian-e1kv',at,{'year':'bad'})
    for field in ('proceeds','cost_basis','gain_loss'):
        case('tax-small-nonzero-'+field,'tax','reports.tax-summary',tax+"UPDATE journal_tax_summary SET quantity=0,proceeds=0,cost_basis=0,gain_loss=0 WHERE id='tax3'; UPDATE journal_tax_summary SET "+field+"=0.00000001 WHERE id='tax3';")
    # Replay raw component facts through the real SQL loader and the shared R2
    # validator. The expected report gate is still computed by Python.
    component_cases=json.loads((ROOT / "tests/fixtures/custody_component_cases.json").read_text())
    selected={"balanced","multi_allocation","conversion","suspense","native_contradicted","revision_valid","commitment_hash","anchor_excluded","component_order","swap_terms","anchor_partial"}
    component_specs=[]
    with tempfile.TemporaryDirectory(prefix="kassiber-component-schema-") as root:
        schema=open_db(root)
        def insert(table, row):
            columns={r["name"] for r in schema.execute(f"PRAGMA table_info({table})")}
            row={k:v for k,v in row.items() if k in columns}
            values=[schema.execute("SELECT quote(?)",(json.dumps(v) if isinstance(v,(dict,list)) else v,)).fetchone()[0] for v in row.values()]
            return "INSERT OR REPLACE INTO " + table + "(" + ",".join(row) + ") VALUES(" + ",".join(values) + ");"
        for case_ in component_cases:
            if case_["case"] not in selected: continue
            raw=case_["input"]
            profile={**raw["profile"],"last_processed_at":"2026-01-01T00:00:00Z"}
            sql=insert("workspaces",{"id":profile["workspace_id"],"label":"Main","created_at":"fixed"})+insert("profiles",profile)
            sql+="INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile'),('database_instance_id','0123456789abcdef0123456789abcdef');"
            wallets={r["id"]:r for r in raw["wallets"]}
            wallets.update({r["id"]:{**wallets.get(r["id"],{}),**r} for r in raw["anchor_wallets"]})
            for wallet in wallets.values():
                sql+=insert("wallets",{**wallet,"workspace_id":"ws","profile_id":"profile","label":wallet.get("label",wallet["id"]),"created_at":"fixed","account_id":None})
            transactions={r["id"]:r for r in raw["transactions"]}
            transactions.update({r["id"]:r for r in raw["anchors"].values() if r})
            for transaction in transactions.values():
                sql+=insert("transactions",{**transaction,"fingerprint":transaction.get("fingerprint",transaction["id"])})
            headers={r["id"]:r for r in raw["revision_headers"]}
            headers.update({c["header"]["id"]:c["header"] for c in raw["components"]})
            for header in headers.values():sql+=insert("custody_components",header)
            for component in raw["components"]:
                for key,table in (("legs","custody_component_legs"),("allocations","custody_component_allocations"),("economic_terms","custody_component_economic_terms"),("commitments","custody_component_evidence_commitments")):
                    for ordinal,row in enumerate(component[key]):
                        if key == "economic_terms":
                            row={"id":f"term-{ordinal}","component_id":component["header"]["id"],"workspace_id":"ws","profile_id":"profile","ordinal":ordinal,"term_kind":"transaction_pair","legacy_source_id":f"source-{ordinal}","source_row_hash":"0"*64,"review_kind":"reviewed","tax_policy":"carry","created_at":"fixed",**row}
                        sql+=insert(table,row)
            for row in raw["evidence_snapshots"]:
                sql+=insert("custody_authored_evidence_snapshots",{**row,"workspace_id":"ws","profile_id":"profile","subject_kind":"custody_component"})
            sql+="UPDATE profiles SET last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='profile' AND excluded=0) WHERE id='profile';"
            component_specs.append((case_["case"],"components","reports.capital-gains",sql,{}))
        schema.close()
    result=[]
    specs += [("history-"+name,"history-readiness","reports.balance-history",sql,history_args)
              for name,group,command,sql,args in list(specs) if group=="readiness"]
    for name,group,command,sql,args in specs+component_specs:
        with tempfile.TemporaryDirectory(prefix="kassiber-report-vector-") as root:
            conn=open_db(root)
            try:
                try: conn.executescript(("" if group=="components" else BASE.read_text())+sql)
                except Exception as exc: raise RuntimeError(name) from exc
                hooks=_report_hooks()
                if command=="reports.filed-snapshots.list":
                    run=lambda:{"snapshots":custody_filed_reports.list_filed_report_snapshots(conn,"p")}
                elif command=="ui.reports.balance_sheet": run=lambda:_reports_balance_sheet_payload(conn)
                elif command=="ui.reports.portfolio_summary": run=lambda:_reports_portfolio_summary_payload(conn)
                elif command=="ui.reports.tax_summary":
                    from kassiber.daemon import _reports_tax_summary_payload
                    run=lambda:_reports_tax_summary_payload(conn,args)
                elif command=="reports.balance-history":
                    run=lambda:reports.report_balance_history(conn,None,None,hooks,interval=args.get('interval','month'),start=args.get('start'),end=args.get('end'),wallet_ref=args.get('wallet'),account_ref=args.get('account'),asset=args.get('asset'))
                elif command=="ui.reports.balance_history":
                    from kassiber.daemon import _reports_balance_history_payload
                    run=lambda:_reports_balance_history_payload(conn,args)
                elif command in {"reports.austrian-e1kv","reports.austrian-tax-summary"}:
                    run=(lambda:"\n".join(reports.build_austrian_e1kv_report_lines(conn,None,None,hooks,tax_year=args.get('year')))) if args.get('text') else (lambda:reports.report_austrian_e1kv(conn,None,None,hooks,tax_year=args.get('year')))
                elif command=="reports.journal-entries":run=lambda:reports.report_journal_entries(conn,None,None,hooks)
                elif command=="reports.tax-summary":run=lambda:reports.report_tax_summary(conn,None,None,hooks)
                elif command=="reports.summary" and args.get("text"): run=lambda:"\n".join(reports.build_summary_report_lines(conn,None,None,hooks,wallet_ref=args.get("wallet")))
                elif command=="reports.summary": run=lambda:reports.report_summary(conn,None,None,hooks,wallet_ref=args.get("wallet"))
                else:
                    function={"reports.balance-sheet":reports.report_balance_sheet,"reports.portfolio-summary":reports.report_portfolio_summary,"reports.capital-gains":reports.report_capital_gains}[command]
                    run=lambda:function(conn,None,None,hooks)
                try:
                    payload = run()
                    if args.get('csv') and command in {'reports.austrian-e1kv', 'reports.austrian-tax-summary'}:
                        payload = payload['summary_rows' if command == 'reports.austrian-tax-summary' else 'rows']
                    expected={"ok":json_ready(payload)}
                except AppError as exc:
                    expected={"error":{"code":exc.code,"message":str(exc),"hint":exc.hint,"details":exc.details,"retryable":exc.retryable}}
                except Exception as exc:
                    expected={"error":{"code":"internal_error","message":str(exc),"hint":None,"details":None,"retryable":False}}
                result.append({"name":name,"group":group,"command":command,"sql":sql,"args":args,"base":group!="components","expected":json.dumps(expected,ensure_ascii=False,separators=(",",":"))})
            finally:conn.close()
    return json.dumps(result,ensure_ascii=False,indent=2)+"\n"

if __name__=="__main__":
    import sys
    content=build()
    if "--check" in sys.argv:
        assert OUT.read_text()==content,"report vectors drifted"
    else:OUT.write_text(content)

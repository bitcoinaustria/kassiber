#!/usr/bin/env python3
"""Freeze managed desktop CSV requests against the Python dispatcher."""
from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile
import time
from unittest.mock import patch
from uuid import UUID

import kassiber.cli.main as cli
from kassiber import daemon
from kassiber.core import reports, custody_filed_reports as snapshots, export_range as ranges
from kassiber.db import open_db
from kassiber.errors import AppError
from gen_export_sequences import FixedDateTime, STAMP

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/export_requests.json"
TABLES = ("filed_report_snapshots", "filed_report_chain_captures", "filed_report_chain_dependencies")


def build():
    seeds = json.loads((ROOT / "crates/reports/tests/data/export_sequences.json").read_text())
    specs = []
    for country in ("generic", "at"):
        def add(kind, args, sql=""):
            specs.append(dict(name=f"{country}-{len(specs)}",country=country,kind=kind,args=args,sql=sql))
        for args in ({}, {"wallet":"Cold","history_limit":2}, {"history_limit":-1}, {"year":None}, {"wallet":False}, {"workspace":"absent","profile":"absent"}):
            add("ui.reports.export_csv",args)
        for year in (None,2024,2025,"2024",2024.9,True,[],"bad"):
            add("ui.reports.export_capital_gains_csv",{"year":year})
        add("ui.reports.export_capital_gains_csv",{"year":[]},"UPDATE settings SET value='invalid' WHERE key='database_instance_id';")
        for args in ({},{"wallets":["Cold","Hot"]},{"wallets":[]},{"wallet":"Cold","wallets":[]},
                     {"wallets":[1]},{"range":""},{"timezone":5},{"range":"custom","from":"2024-01-01","to":"2024-12-31","timezone":"Europe/Vienna"},
                     {"range":"last_year"},{"from":"2024-01-01"},{"z":1,"a":2}):
            add("ui.transactions.export_csv",args)
        for year in (None,2024,2025,"2024",2024.9,True,[],"bad"):
            add("ui.reports.export_austrian_e1kv_csv",{"year":year})
        blocked = "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed');"
        for kind in ("ui.reports.export_csv","ui.reports.export_capital_gains_csv","ui.reports.export_austrian_e1kv_csv","ui.transactions.export_csv"):
            add(kind,{},blocked)
    for spec in specs:
        seed = next(c for c in seeds if c["name"].startswith(spec["country"]+"-") and c["name"].endswith("-ready"))
        spec["sql"] = seed["sql"] + spec["sql"]
        with tempfile.TemporaryDirectory(prefix="export-request-") as root_text:
            root=Path(root_text)
            conn=open_db(root / "data")
            conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text()+spec["sql"])
            try:
                with patch.object(ranges,"datetime",FixedDateTime),patch.object(reports,"datetime",FixedDateTime),patch.object(snapshots,"now_iso",return_value=STAMP),patch(
                    "kassiber.core.filed_report_chain.now_iso",return_value=STAMP),patch.object(snapshots.uuid,"uuid4",return_value=UUID(int=1)),patch.object(
                    daemon,"_report_hooks",return_value=replace(cli._report_hooks(),now_iso=lambda:STAMP)),patch.object(daemon.time,"gmtime",return_value=time.struct_time((2026,1,2,3,4,5,4,2,0))):
                    result=daemon._ui_report_export_payload_from_conn(conn,str(root / "data"),spec["kind"],spec["args"])
                conn.commit()
                expected={"ok":result}
            except Exception as exc:
                conn.rollback()
                expected={"error":dict(code=exc.code if isinstance(exc,AppError) else "internal_error",message=str(exc),hint=getattr(exc,"hint",None),details=getattr(exc,"details",None),retryable=getattr(exc,"retryable",False))}
            spec["expected"]=json.loads(json.dumps(expected).replace(root_text,"<root>"))
            spec["tables"]={t:[dict(r) for r in conn.execute(f'SELECT * FROM {t} ORDER BY 1,2,3')] for t in TABLES}
            spec["files"]={str(p.relative_to(root)):p.read_bytes().decode() for p in sorted((root / "exports").rglob("*.csv"))}
            conn.close()
    return json.dumps(specs,ensure_ascii=True,indent=2)+"\n"


if __name__ == "__main__":
    content=build()
    if "--check" in sys.argv:
        assert OUT.read_text()==content,"export request vectors drifted"
    else:
        OUT.write_text(content)

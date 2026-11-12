"""Export exact SQL and column definitions for handwritten Rust migrations.

This manifest contains data only. It never dispatches Python functions from
Rust. Every conditional branch and row transformation needs its own Rust port.
"""
from __future__ import annotations
import argparse
import ast
import hashlib
import inspect
import json
import unicodedata
from pathlib import Path

from kassiber import db
from kassiber import chain_analysis_watches_schema, wallet_descriptors
from kassiber.core.chain_analysis import projection_store, confirmations

DESTINATION=Path(__file__).resolve().parents[1]/"fixtures/compat.json"
PREFIXES=("_ensure_","_migrate_","_backfill_","_rebuild_","_create_custody_","_record_custody_","_drop_legacy_","_clear_stale_","_recreate_msat_","_preflight_","_repair_attachment_","_attachment_fk_","_custody_evidence_","custody_gap_review_","backfill_custody_gap_review_")

def literal(node):
    if isinstance(node,ast.Constant):return node.value
    if isinstance(node,ast.JoinedStr):
        parts=[]
        for value in node.values:
            if isinstance(value,ast.Constant):parts.append(value.value)
            elif isinstance(value,ast.FormattedValue) and isinstance(value.value,ast.Name) and value.value.id in ("DEFAULT_TAX_COUNTRY","DEFAULT_LONG_TERM_DAYS"):
                parts.append(str(getattr(db,value.value.id)))
            else:return None
        return "".join(parts)
    return None

def sql_calls(tree):
    result=[]
    for node in sorted(ast.walk(tree),key=lambda node:getattr(node,"lineno",0)):
        if isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute) and node.func.attr in ("execute","executescript","executemany") and node.args:
            value=literal(node.args[0])
            if isinstance(value,str):result.append(value)
    return result

def payload():
    functions={}
    sources={**vars(db),"ensure_inbox_sources":chain_analysis_watches_schema.ensure_inbox_sources}
    for name,function in sorted(sources.items()):
        if not inspect.isfunction(function) or not (name.startswith(PREFIXES) or name in ("ensure_schema_compat","ensure_inbox_sources")):continue
        source=inspect.getsource(function);tree=ast.parse(source)
        columns=[]
        for node in sorted(ast.walk(tree),key=lambda node:getattr(node,"lineno",0)):
            if isinstance(node,ast.Call) and isinstance(node.func,ast.Name) and node.func.id in ("ensure_column","_ensure_column_no_commit"):
                values=[literal(arg) for arg in node.args[1:]]
                if len(values)==3 and all(isinstance(value,str) for value in values):columns.append(values)
        constants={}
        loops=[]
        for node in ast.walk(tree):
            if isinstance(node,ast.Assign) and len(node.targets)==1 and isinstance(node.targets[0],ast.Name):
                try: value=ast.literal_eval(node.value)
                except (ValueError,TypeError):continue
                constants[node.targets[0].id]=value
            if isinstance(node,ast.For):
                try: value=ast.literal_eval(node.iter)
                except (ValueError,TypeError):continue
                loops.append(dict(target=ast.unparse(node.target),values=value))
        functions[name]=dict(source_sha256=hashlib.sha256(source.encode()).hexdigest(),sql=sql_calls(tree),columns=columns,constants=constants,loops=loops)
    tree=ast.parse(inspect.getsource(db._migrate_msat_columns));msat={}
    for node in ast.walk(tree):
        if isinstance(node,ast.If) and isinstance(node.test,ast.Name) and node.test.id.startswith("migrate_"):
            values=sql_calls(node)
            if values:msat[node.test.id]=values
    decimal_digits={}
    for point in range(0x110000):
        try: decimal_digits[str(point)]=unicodedata.decimal(chr(point))
        except ValueError:pass
    return dict(base_schema=db.SCHEMA,custody_schema=db.CUSTODY_COMPONENT_SCHEMA,functions=functions,msat=msat,
                ownership_private_fields=sorted(db._POLICY_EPOCH_PRIVATE_FIELDS),
                custody_ai_audit_columns=db._CUSTODY_AI_AUDIT_COLUMNS,
                custody_migration_explanations=db._CUSTODY_MIGRATION_EXPLANATIONS,
                normalization=dict(chain=wallet_descriptors.CHAIN_ALIASES,bitcoin_network=wallet_descriptors.BITCOIN_NETWORK_ALIASES,liquid_network=wallet_descriptors.LIQUID_NETWORK_ALIASES,policy_assets=wallet_descriptors.LIQUID_POLICY_ASSET_IDS),
                unicode_decimal=decimal_digits,
                graph=dict(ddl=projection_store.DDL,physical=projection_store.PHYSICAL,sources=projection_store.SOURCE_TABLES,tip_fields=confirmations.TIP_FIELDS))

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--check",action="store_true");args=parser.parse_args()
    encoded=json.dumps(payload(),indent=2,ensure_ascii=True,default=lambda value: sorted(value))+"\n"
    if args.check:assert DESTINATION.read_text()==encoded,"Python compatibility SQL changed; review the Rust migrations"
    else:DESTINATION.write_text(encoded)

if __name__=="__main__":main()

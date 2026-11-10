#!/usr/bin/env python3
"""Freeze synthetic backend responses and every affected stored value; no I/O clients."""
from __future__ import annotations
import argparse
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber import backends, daemon
from kassiber.core import accounts
from kassiber.db import open_db
from kassiber.errors import AppError

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/python_backends.json"

def generate():
    scenarios = []
    for manual in (False, True):
        with tempfile.TemporaryDirectory(prefix="kassiber-backend-vectors-") as root, mock.patch.dict(os.environ, {}, clear=True), mock.patch("kassiber.backends.now_iso", return_value="2026-01-01T00:00:00Z"):
            conn = open_db(Path(root) / "data")
            if manual:
                backends.set_backend_bootstrap_mode(conn, "manual")
            config = backends.load_runtime_config(Path(root) / "absent.env")
            ctx = SimpleNamespace(conn=conn, runtime_config=config)
            records = []
            def record(op, **args):
                backends.merge_db_backends(conn, ctx.runtime_config)
                command = "ui.backends." + op[3:] if op.startswith("ui:") else "backends." + op
                try:
                    if op.startswith("ui:"):
                        name = op[3:]
                        handler = {
                            "options": daemon._backend_options_payload,
                            "settings.list": daemon._backend_settings_list_payload,
                            "public_defaults": daemon._backend_public_defaults_payload,
                            "create": daemon._create_backend_payload,
                            "update": daemon._update_backend_payload,
                            "delete": daemon._delete_backend_payload,
                            "set_default": daemon._set_default_backend_payload,
                        }[name]
                        result = handler(ctx) if name in {"options", "settings.list", "public_defaults"} else handler(ctx, args)
                    elif op == "create":
                        supplied = dict(args)
                        result = accounts.create_backend(conn, supplied.pop("name"), supplied.pop("kind"), supplied.pop("url"), **supplied)
                    elif op == "update":
                        result = accounts.update_backend(conn, args["name"], {k:v for k,v in args.items() if k != "name"})
                    elif op == "list": result = accounts.list_backends(ctx.runtime_config)
                    elif op == "kinds": result = accounts.list_backend_kinds()
                    elif op == "get": result = accounts.get_backend_details(conn, ctx.runtime_config, args["name"])
                    elif op == "reveal-token": result = accounts.reveal_backend_secrets(conn, ctx.runtime_config, args["name"])
                    elif op == "delete": result = accounts.delete_backend(conn, args["name"], ctx.runtime_config)
                    elif op == "set-default": result = accounts.set_default_backend(conn, ctx.runtime_config, args["name"])
                    elif op == "clear-default": result = accounts.clear_default_backend(conn, ctx.runtime_config)
                    outcome = {"ok": result}
                except AppError as exc:
                    conn.rollback()
                    outcome = {"error": {"code": exc.code, "message": str(exc), "hint": exc.hint, "details": exc.details, "retryable": exc.retryable}}
                state = {"backends": [dict(r) for r in conn.execute("SELECT * FROM backends ORDER BY name")],
                         "settings": [dict(r) for r in conn.execute("SELECT * FROM settings WHERE key LIKE '%backend%' ORDER BY key")]}
                records.append(copy.deepcopy({"command": command, "args": args, **outcome, "state": state}))
            record("list")
            record("kinds")
            record("get", name="missing")
            record("set-default", name="missing")
            for name, kind, chain in [("commerce", "btcpay", "bitcoin"), ("liquid-only", "electrum", "liquid"), ("private", "esplora", "bitcoin")]:
                record("create", name=name, kind=kind, url="https://alice:synthetic-pass@example.test:8443/api?key=synthetic-query#fragment", chain=chain, network="mainnet", token="synthetic-token", auth_header="Bearer synthetic-header", tor_proxy="socks5h://bob:synthetic-proxy@LOCALHOST:9050", config={"password":"synthetic-password", "rpc_user":"synthetic-user", "macaroon":"synthetic-macaroon", "certificate":"/synthetic/ca.pem", "unknown":"synthetic-private", "silent_payments":True, "display_label":"Private"})
            record("create", name=" private ", kind="esplora", url="https://example.test")
            for name in ["PRIVATE", "fulcrum", "commerce"]: record("get", name=name)
            record("list")
            record("ui:options")
            record("ui:settings.list")
            record("ui:public_defaults")
            record("reveal-token", name="private")
            for changes in ({}, {"timeout":0}, {"batch_size":-1}, {"url":" "}, {"kind":"invalid"}, {"chain":"bad"}, {"network":"bad"}, {"clear":["not-a-field"]}, {"clear":["token"],"token":"synthetic-token"}, {"clear":["password"],"config":{"rpc_password":"synthetic-conflict"}}):
                record("update", name="private", **changes)
            record("update", name="private", config={"insecure":"yes", "display_name":"  New name  ","unknown":None, "silent_payments":False}, clear=["token","password","certificate"])
            record("get", name="private")
            record("ui:update", name="private", config={"insecure":None}, clear=["auth_header", "tor_proxy"])
            record("ui:settings.list")
            record("ui:update", name="private", config=[])
            record("ui:update", name="private", timeout="10")
            record("ui:update", name="private", silent_payments=1)
            record("ui:update", name="private", clear="token")
            record("ui:create", name="", kind="esplora", url="https://example.test")
            record("ui:create", name="desktop", kind="electrum", url="ssl://example.test:50002", config={"certificate":"/synthetic/ca.pem", "insecure":False})
            record("ui:set_default", name="desktop")
            record("ui:delete", name="desktop")
            record("set-default", name="private")
            record("delete", name="private")
            record("clear-default")
            record("delete", name="commerce")
            record("delete", name="liquid-only")
            record("clear-default")
            record("ui:options")
            record("ui:settings.list")
            if not manual:
                record("ui:update", name="mempool", config={"display_name":"Promoted"})
                record("ui:delete", name="mempool")
            scenarios.append({"manual":manual, "records":records})
            conn.close()
    return {"scenarios":scenarios}

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(generate(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if not DESTINATION.exists() or DESTINATION.read_text() != encoded:
            raise SystemExit("backend vectors differ; rerun export_backend_vectors.py")
    else:
        DESTINATION.write_text(encoded)

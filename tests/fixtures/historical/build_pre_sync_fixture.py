"""Build `pre_sync_bae01537b`: a processed book written by main #654.

Run from the repository root with the historical tree exported elsewhere:

    git archive bae01537b kassiber | tar -x -C <historical>
    uv run --locked python tests/fixtures/historical/build_pre_sync_fixture.py \
        --historical <historical> --out <new disposable directory>

Every CLI child runs `python -m kassiber` from <historical>, with its own XDG
directories under --out and KASSIBER_NO_EGRESS=1, so nothing resolves the
account's real app-data root. The book is an Austrian EUR book with a hot, a
cold and a Liquid wallet, buys, a sale, interest and an owned transfer,
processed with `journals process`. It also holds an AI provider and two custody
AI audit rows, written with #654's narrower consent CHECK, so opening it runs
every schema change main added after #654. No real data.
"""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
SCOPE = ["--workspace", "Main", "--profile", "Default"]
ROWS = {
    "Hot": [
        "2024-01-03T09:00:00Z,buy-1,inbound,BTC,0.04000000,0.00000000,40000,Monthly purchase,buy",
        "2024-02-03T09:00:00Z,buy-2,inbound,BTC,0.03000000,0.00000000,42000,Monthly purchase,buy",
        f"2024-03-08T09:00:00Z,{'ab' * 32},outbound,BTC,0.03500000,0.00002000,55000,To cold storage,transfer",
        "2024-04-03T09:00:00Z,buy-3,inbound,BTC,0.02000000,0.00000000,60000,Monthly purchase,buy",
        "2024-08-15T09:00:00Z,spend-1,outbound,BTC,0.00500000,0.00000000,52000,Hardware purchase,expense",
    ],
    "Cold": [
        f"2024-03-08T09:00:00Z,{'ab' * 32},inbound,BTC,0.03498000,0.00000000,55000,From hot wallet,transfer",
        "2024-11-20T09:00:00Z,sell-1,outbound,BTC,0.01000000,0.00003000,85000,Exchange sale,sell",
    ],
    "Liquid": [
        "2024-03-12T09:00:00Z,interest-1,inbound,LBTC,0.00040000,0.00000000,55000,Lending interest,interest",
    ],
}
WALLETS = {
    "Hot": ["--chain", "bitcoin"],
    "Cold": ["--chain", "bitcoin"],
    "Liquid": ["--source-file", "/dev/null", "--chain", "liquid", "--network", "liquidv1"],
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--historical", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True)  # refuses an existing directory
    root = out / "data"
    env = {key: value for key, value in os.environ.items() if not key.startswith("XDG_")}
    env.update({key: str(out / "xdg" / key.lower()) for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME")})
    env.update(PYTHONPATH=str(args.historical.resolve()), KASSIBER_NO_EGRESS="1", KASSIBER_DISABLE_UPDATE_CHECK="1")

    def run(*argv: str) -> dict:
        result = subprocess.run([sys.executable, "-m", "kassiber", "--machine", "--data-root", str(root), *argv],
                                cwd=out, env=env, capture_output=True, text=True, timeout=600)
        if result.returncode != 0:
            raise SystemExit(f"FAILED {' '.join(argv)}: {result.stdout[:800]} {result.stderr[-800:]}")
        return json.loads(result.stdout)

    run("init")
    run("workspaces", "create", "Main")
    run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "at",
        "--gains-algorithm", "MOVING_AVERAGE_AT", "Default")
    for wallet, extra in WALLETS.items():
        run("wallets", "create", *SCOPE, "--label", wallet, "--kind", "custom", *extra)
        path = out / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(ROWS[wallet]) + "\n")
        run("wallets", "import-csv", *SCOPE, "--wallet", wallet, "--file", str(path))
    run("journals", "process", *SCOPE)
    run("ai", "providers", "create", "local", "--base-url", "http://127.0.0.1:9/v1", "--kind", "local",
        "--default-model", "synthetic-model")
    database = root / "kassiber.sqlite3"
    with sqlite3.connect(database) as conn:
        workspace, profile = conn.execute("SELECT workspace_id, id FROM profiles").fetchone()
        for ordinal, decision in enumerate(("allow_once", "deny")):
            conn.execute(
                """INSERT INTO custody_ai_assistance_audits(
                       id, workspace_id, profile_id, tool_name, daemon_kind, call_id, provider_kind, model,
                       facts_sha256, consent_decision, consent_requested_at, consent_decided_at,
                       execution_status, created_at)
                   VALUES(?, ?, ?, 'ui.custody.review.apply', 'ui.custody.review.apply', ?, 'local',
                          'synthetic-model', ?, ?, '2025-01-01T00:00:00Z', '2025-01-01T00:00:01Z', ?,
                          '2025-01-01T00:00:02Z')""",
                (f"audit-{ordinal}", workspace, profile, f"call-{ordinal}", f"{ordinal:064x}", decision,
                 "executed" if decision == "allow_once" else "denied"),
            )
    conn.close()
    raw = database.read_bytes()
    encoded = base64.b64encode(gzip.compress(raw, mtime=0)).decode()
    (out / "pre_sync_bae01537b.sqlite3.gz.b64").write_text(encoded + "\n")
    print(json.dumps({"database_sha256": hashlib.sha256(raw).hexdigest(),
                      "db_py_sha256": hashlib.sha256((args.historical / "kassiber/db.py").read_bytes()).hexdigest()}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Repro: `wallets sync --wallet X` still crashes on X's unreadable config.

    uv run --locked python -m tests.rust_port.repro_cli_wallet_sync_bad_config_explicit

Round r7-6 (7bca137d4) made an unreadable stored wallet configuration a typed
`invalid_wallet_config` refusal in the freshness planner and in
`cli.handlers.sync_wallet`'s `--all` selection. The explicit `--wallet` path
resolves the wallet and goes on to `core_sync.classify_wallet_sync`, which
runs `json.loads` on the stored text unguarded, so the CLI answers
`internal_error` with the decoder's message ("Expecting value: line 1 column 1
(char 0)" for text, "'list' object has no attribute 'get'" for `[]`). The Rust
CLI answers `internal_error` too, with its own message ("Invalid wallet
configuration JSON", "Wallet configuration must be an object"), so the two
messages differ; tests/test_rust_wallet_sync_cli.py compares `--all` only.
Scratch stays under target/. Exit 0 means the bug reproduced.
"""
from pathlib import Path
import tempfile

from kassiber.cli import handlers
from kassiber.db import open_db
from kassiber.errors import AppError


def main() -> int:
    scratch = Path(__file__).resolve().parents[2] / "target/cli-wallet-sync-bad-config-repro"
    scratch.mkdir(parents=True, exist_ok=True)
    reproduced = True
    with tempfile.TemporaryDirectory(dir=scratch.resolve()) as directory:
        conn = open_db(Path(directory) / "data")
        try:
            conn.executescript(
                "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
                "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at)"
                " VALUES('p','w','Book','EUR','at','2025');"
                "INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');"
            )
            for stored in ("bad", "[]"):
                conn.execute("DELETE FROM wallets")
                conn.execute(
                    "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)"
                    " VALUES('bad','w','p','Bad','descriptor',?,'2025')",
                    (stored,),
                )
                for sync_all in (True, False):
                    try:
                        handlers.sync_wallet(conn, {}, None, None, None if sync_all else "Bad", sync_all=sync_all)
                    except AppError as error:
                        outcome = f"AppError {error.code}"
                    except Exception as error:  # noqa: BLE001 - the repro reports it
                        outcome = f"{type(error).__name__}: {error}"
                    else:
                        outcome = "no error"
                    print(f"{stored!r} {'--all' if sync_all else '--wallet Bad'}: {outcome}")
                    if sync_all:
                        reproduced &= outcome == "AppError invalid_wallet_config"
                    else:
                        reproduced &= not outcome.startswith("AppError")
        finally:
            conn.close()
    return 0 if reproduced else 1


if __name__ == "__main__":
    raise SystemExit(main())

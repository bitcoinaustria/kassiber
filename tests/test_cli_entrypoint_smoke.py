from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from kassiber import __version__
import kassiber.cli.main as cli_main
from kassiber.cli.main import build_parser


ROOT = Path(__file__).resolve().parent.parent

HELP_PATHS = (
    (),
    ("wallets", "sync"),
    ("wallets", "sync-btcpay"),
    ("wallets", "import-river"),
    ("wallets", "import-21bitcoin"),
    ("wallets", "import-strike"),
    ("wallets", "import-ledger"),
    ("wallets", "import-ledger-live"),
    ("wallets", "import-binance-supplemental"),
    ("wallets", "sync-kraken"),
    ("wallets", "sync-coinbase"),
    ("wallets", "sync-binance"),
    ("wallets", "ledger-template"),
    ("profiles", "create"),
    ("metadata", "records"),
    ("attachments", "list"),
    ("source-funds",),
    ("source-funds", "sources", "create"),
    ("source-funds", "links", "review"),
    ("btcpay", "provenance"),
    ("btcpay", "provenance", "review"),
    ("documents", "create"),
    ("journals", "events"),
    ("reports", "commercial-subledger"),
    ("reports", "export-commercial-subledger-csv"),
    ("reports", "source-funds"),
    ("reports", "export-source-funds-pdf"),
    ("reports", "balance-history"),
    ("reports", "exit-tax"),
    ("reports", "export-exit-tax-pdf"),
    ("reports", "export-exit-tax-xlsx"),
    ("rates",),
    ("diagnostics", "collect"),
    ("chat",),
    ("update",),
    ("verify-download",),
    ("chats",),
    ("ai",),
    ("ai", "providers"),
    ("ai", "providers", "create"),
    ("secrets",),
    ("secrets", "init"),
    ("secrets", "change-passphrase"),
    ("secrets", "remember-unlock"),
    ("secrets", "forget-unlock"),
    ("secrets", "verify"),
    ("secrets", "status"),
    ("secrets", "migrate-credentials"),
    ("backup",),
    ("backup", "export"),
    ("backup", "import"),
    ("sync",),
    ("sync", "transport", "add"),
    ("sync", "lan"),
    ("sync", "tor"),
    ("sync", "gc"),
    ("backends", "reveal-token"),
    ("wallets", "reveal-descriptor"),
)


@pytest.fixture(scope="module")
def cli_parser():
    return build_parser()


@pytest.mark.parametrize("command_path", HELP_PATHS)
def test_help_surfaces_parse_in_process(cli_parser, command_path):
    output = io.StringIO()
    with contextlib.redirect_stdout(output), pytest.raises(SystemExit) as raised:
        cli_parser.parse_args([*command_path, "--help"])
    assert raised.value.code == 0
    assert "usage:" in output.getvalue().lower()


def test_version_is_database_free(cli_parser):
    output = io.StringIO()
    with contextlib.redirect_stdout(output), pytest.raises(SystemExit) as raised:
        cli_parser.parse_args(["--version"])
    assert raised.value.code == 0
    assert output.getvalue().strip() == f"Kassiber {__version__}"


def _run_cli(home: Path, *args: str, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["HOME"] = str(home)
    env.pop("XDG_DATA_HOME", None)
    if os.name == "nt":
        env["USERPROFILE"] = str(home)
    env["PYTHONPYCACHEPREFIX"] = str(home / "pycache")
    return subprocess.run(
        [sys.executable, "-m", "kassiber", *args],
        cwd=ROOT,
        env=env,
        input=input_text,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )


@pytest.mark.parametrize("command", (("status",), ("health",), ("next-actions",)))
def test_machine_entrypoints_are_real_subprocesses(tmp_path, command):
    initialized = _run_cli(tmp_path, "--machine", "init")
    assert initialized.returncode == 0, initialized.stderr
    result = _run_cli(tmp_path, "--machine", *command)
    assert result.returncode == 0, result.stderr
    payload = json.loads(result.stdout)
    assert payload["kind"]


def test_version_entrypoint_is_a_real_subprocess(tmp_path):
    legacy_catalog = tmp_path / ".kassiber" / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")
    result = _run_cli(tmp_path, "--version")
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == f"Kassiber {__version__}"
    assert legacy_catalog.is_file()


def test_explicit_data_root_does_not_migrate_hidden_home_state(tmp_path):
    legacy_catalog = tmp_path / ".kassiber" / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")

    result = _run_cli(
        tmp_path,
        "--machine",
        "--data-root",
        str(tmp_path / "explicit" / "data"),
        "init",
    )

    assert result.returncode == 0, result.stderr
    assert legacy_catalog.is_file()


def test_explicit_env_file_does_not_migrate_hidden_home_state(tmp_path):
    legacy_catalog = tmp_path / ".kassiber" / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")
    env_file = tmp_path / "backends.env"
    env_file.write_text("", encoding="utf-8")

    result = _run_cli(
        tmp_path,
        "--machine",
        "--env-file",
        str(env_file),
        "init",
    )

    assert result.returncode == 0, result.stderr
    assert legacy_catalog.is_file()
    assert not (tmp_path / ".local" / "share" / "kassiber").exists()


def test_backup_import_does_not_migrate_hidden_home_state(tmp_path):
    legacy_catalog = tmp_path / ".kassiber" / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")

    result = _run_cli(
        tmp_path,
        "--machine",
        "backup",
        "import",
        str(tmp_path / "missing.kassiber"),
    )

    assert result.returncode != 0
    assert legacy_catalog.is_file()
    assert not (tmp_path / ".local" / "share" / "kassiber").exists()


@pytest.mark.parametrize(
    ("argv", "migrates"),
    (
        (("operator", "unlock"), True),
        (("operator", "mode", "manual"), True),
        (("operator", "status"), False),
        (("operator", "lock"), False),
    ),
)
def test_operator_startup_migrates_before_creating_a_lease(monkeypatch, argv, migrates):
    calls = []
    monkeypatch.delenv("KASSIBER_SKIP_DEFAULT_STATE_ROOT_MIGRATION", raising=False)
    monkeypatch.setattr(
        cli_main, "migrate_hidden_home_state_root_if_needed", lambda: calls.append(True)
    )

    cli_main._maybe_migrate_default_state_root(build_parser().parse_args(argv))

    assert bool(calls) is migrates


def test_hidden_migration_helper_moves_default_state(tmp_path):
    legacy_catalog = tmp_path / ".kassiber" / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")

    result = _run_cli(tmp_path, "--migrate-default-state-root")

    native_catalog = (
        tmp_path / ".local" / "share" / "kassiber" / "config" / "projects.json"
    )
    assert result.returncode == 0, result.stderr
    assert not (tmp_path / ".kassiber" / "config").exists()
    assert (
        tmp_path / ".kassiber" / "run" / "operator-v1.start.lock"
    ).is_file()
    assert native_catalog.is_file()


def test_command_catalog_entrypoint_is_a_real_subprocess(tmp_path):
    result = _run_cli(
        tmp_path,
        "--machine",
        "commands",
        "describe",
        "wallets",
        "sync",
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["kind"] == "commands.describe"


def test_daemon_accepts_eof_in_a_real_subprocess(tmp_path):
    result = _run_cli(tmp_path, "daemon", input_text="")
    assert result.returncode == 0, result.stderr

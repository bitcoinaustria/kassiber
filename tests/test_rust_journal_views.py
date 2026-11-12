"""Registered desktop journal reads against the unchanged Python daemon.

`ui.journals.snapshot`, `ui.journals.events.list` and `ui.journals.transfers.list`
must return Python's ordered rows for current, stale, quarantined, paired and
Austrian neutral-swap books, every argument refusal, and books without an
active profile. Reads never write.
"""
import copy
import json
import shutil

import pytest

from kassiber.db import open_db
from tests.test_rust_journal_commands import BINARY, KEY, book_dump, invoke, seed, seed_auto
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_desktop import Journals
from tests.test_rust_journal_process import scalar_exact
from tests.rust_port.journal_refresh_oracle import SCENARIOS

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")
KINDS = ("ui.journals.snapshot", "ui.journals.events.list", "ui.journals.transfers.list",
         "ui.journals.quarantine")
ARGUMENTS = [
    None, {}, [], 7, "text", {"limit": 1}, {"limit": True}, {"limit": "2"}, {"limit": 2.9},
    {"limit": 0}, {"limit": -3}, {"limit": "x"}, {"limit": None}, {"limit": [1]}, {"limit": 10**30},
    {"limit": 9999}, {"unexpected": 1, "another": 2},
]
EVENT_ARGUMENTS = [
    {"transaction": "a"}, {"transaction": "  a  "}, {"transaction": "   "}, {"transaction": "missing"},
    {"transaction": 7}, {"transaction": None}, {"transaction": "a", "limit": 1},
]


QUARANTINE_ARGUMENTS = [
    {"scope": "attention"}, {"scope": "waiting"}, {"scope": "all", "offset": 1}, {"scope": "x"},
    {"scope": 7}, {"offset": -1}, {"offset": True}, {"offset": "1"}, {"offset": 10**30},
    {"limit": 1, "offset": 2}, {"limit": 200, "scope": "waiting"},
]


def requests():
    for kind in KINDS:
        for args in ARGUMENTS:
            yield kind, args
    for args in EVENT_ARGUMENTS:
        yield "ui.journals.events.list", args
    for args in QUARANTINE_ARGUMENTS:
        yield "ui.journals.quarantine", args


class Reads(Journals):
    """Python's tracebacks go to a file: a full stderr pipe would stall the reference."""

    def __init__(self, root, env, *, native):
        import subprocess
        import sys
        from tests.test_rust_journal_commands import ROOT
        prefix = [str(BINARY)] if native else [sys.executable, "-m", "tests.rust_port.journal_cli_reference"]
        self.stderr_file = open(root.parent / f"{root.name}-{'native' if native else 'python'}-stderr.txt", "wb")
        self.process = subprocess.Popen([*prefix, "--data-root", str(root), "daemon"], cwd=ROOT, env=env,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr_file)
        self.buffer = b""
        self.rows = []

    def __exit__(self, *_):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=30)
        for stream in (self.process.stdin, self.process.stdout, self.stderr_file):
            stream.close()


def session(root, env, native, cipher):
    """Every request's terminal row from one daemon session."""
    with Reads(root, env, native=native) as daemon:
        daemon.unlock(cipher)
        rows = []
        for number, (kind, args) in enumerate(requests()):
            daemon.send(kind, f"r{number}", args)
            rows.append(daemon.until(f"r{number}", kind))
        daemon.send("daemon.shutdown", "stop")
        daemon.until("stop")
        daemon.process.stdin.close()
        daemon.process.wait(timeout=60)
        assert daemon.process.returncode == 0
        daemon.stderr_file.flush()
        stderr = open(daemon.stderr_file.name, "rb").read()
        if native:
            assert stderr == b"", stderr
    return rows


def normalized(rows):
    """Only the established interpreter traceback allowance for internal_error."""
    rows = copy.deepcopy(rows)
    for row in rows:
        error = row.get("error") if row["kind"] == "error" else None
        if error and error["code"] == "internal_error" and isinstance(error.get("debug"), str):
            assert error["debug"].startswith("Traceback (most recent call last):")
            error["debug"] = None
    return rows


def compare(tmp_path, environment, root, cipher=False):
    native = tmp_path / "rust"
    shutil.copytree(root, native)
    before = book_dump(native, cipher)
    expected = session(root, environment, False, cipher)
    actual = session(native, environment, True, cipher)
    (tmp_path / "views.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=1))
    for (kind, args), got, want in zip(requests(), actual, normalized(expected)):
        assert json.dumps(got) == json.dumps(want), (kind, args, got, want)
    assert scalar_exact(book_dump(native, cipher), before)
    return actual


def processed(root, environment, cipher=False, auto_pair=False):
    result = invoke(root, environment, cipher=cipher, extra=("--auto-pair",) if auto_pair else ())
    assert result.returncode == 0, result
    return root


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("shape", ["unchanged", "changed", "blocked", "empty"])
@pytest.mark.parametrize("process", [False, True])
def test_journal_views_plain_books_match_python(tmp_path, environment, cipher, shape, process):
    root = seed(tmp_path / "python", cipher=cipher, shape=shape)
    if process:
        processed(root, environment, cipher)
    rows = compare(tmp_path, environment, root, cipher)
    if process and shape == "blocked":
        assert any(row["data"]["status"]["quarantines"] for row in rows
                   if row["kind"] == "ui.journals.snapshot")


@pytest.mark.parametrize("current", [False, True])
@pytest.mark.parametrize("scenario,country", [(s, "generic") for s in SCENARIOS] + [("exact", "at")])
def test_journal_views_paired_books_match_python(tmp_path, environment, scenario, country, current):
    root = seed_auto(tmp_path / "python", scenario=scenario, current=current, country=country)
    if not current:
        processed(root, environment, auto_pair=True)
    rows = compare(tmp_path, environment, root)
    if scenario == "exact" and not current:
        transfers = [row for row in rows if row["kind"] == "ui.journals.transfers.list"]
        assert any(row["data"]["pairs"] for row in transfers)


def seed_paired_fee(root, cipher=False, swap=False, received=5000000000):
    """A priced book whose paired transfer pays a fee, so journal rows carry the pair."""
    from kassiber.cli import handlers
    seed(root, cipher=cipher, shape="unchanged")
    with open_db(root, passphrase=KEY if cipher else None) as conn:
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) "
                     "VALUES('cold','w','p','Cold','manual','2020')")
        txid = "ab" * 32
        for tx, wallet, direction, amount, fee, asset in (
            ("x-out", "wallet", "outbound", 5000000000, 2000000, "BTC"),
            ("x-in", "cold", "inbound", received, 0, "LBTC" if swap else "BTC"),
        ):
            conn.execute(
                """INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
                direction,asset,amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,
                pricing_quality,pricing_provider,raw_json,created_at)
                VALUES(?,'w','p',?,?,?,'2024-05-01T00:00:00Z',?,?,?,?,'transfer','EUR',40000.0,'40000','manual',
                'exact','reviewed-receipt','{}','2020')""",
                (tx, wallet, txid, tx, direction, asset, amount, fee),
            )
        handlers.create_transaction_pair(conn, "w", "p", "x-out", "x-in", kind="peg-in" if swap else "manual")
    conn.close()
    return root


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("swap", [False, True])
def test_journal_views_rows_carry_pair_metadata_like_python(tmp_path, environment, cipher, swap):
    root = processed(seed_paired_fee(tmp_path / "python", cipher, swap), environment, cipher)
    rows = compare(tmp_path, environment, root, cipher)
    assert any('"pairId"' in json.dumps(row.get("data")) for row in rows), "no journal row carried a pair"


def generated_books():
    """Seeds whose processed books hold varied quarantine causes and roots."""
    from tests.custody_gap_stage import suites
    for suite, scenarios, _ in suites():
        if suite in ("gaps", "safety", "components", "evidence", "samourai", "channels", "pipeline", "fanout"):
            for seed_id in range(0, len(scenarios), max(1, len(scenarios) // 6)):
                yield suite, seed_id


@pytest.mark.parametrize("suite,seed_id", list(generated_books()))
def test_journal_views_generated_quarantines_match_python(tmp_path, environment, suite, seed_id):
    import importlib.util
    from tests.custody_gap_stage import suites
    from tests.test_rust_journal_commands import ROOT
    spec = importlib.util.spec_from_file_location("route_campaign", ROOT / "scripts/check-journal-route-campaign.py")
    campaign = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(campaign)
    generate = dict((name, g) for name, _, g in suites())[suite]
    root = tmp_path / "python"
    relaxed, _ = campaign.build_book(generate(seed_id), campaign.template(tmp_path), root)
    if relaxed:
        pytest.skip("relaxed adapter schema is not an admitted book")
    invoke(root, environment)
    compare(tmp_path, environment, root)


@pytest.mark.parametrize("cipher", [False, True])
def test_journal_quarantine_pair_evidence_and_refresh_error_match_python(tmp_path, environment, cipher):
    """A short-received pair holds suspense; a later refresh failure is reported."""
    root = seed_paired_fee(tmp_path / "python", cipher, received=4000000000)
    invoke(root, environment, cipher=cipher)
    with open_db(root, passphrase=KEY if cipher else None) as conn:
        conn.execute(
            "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,last_error_at,"
            "last_error_code,last_error_message,last_success_at,updated_at) VALUES('p','journals:p','journals','Journals','error',"
            "'2099-01-01T00:00:00Z','journal_refresh_failed','failed for cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd','2020-01-01T00:00:00Z','2020')")
    conn.close()
    rows = compare(tmp_path, environment, root, cipher)
    pages = [row["data"] for row in rows if row["kind"] == "ui.journals.quarantine"]
    assert any(page["summary"].get("freshness", {}).get("last_error") for page in pages)
    assert any("pair_fingerprint" in item["evidence"] for page in pages for item in page["items"]), pages[0]


@pytest.mark.parametrize("context", ["no-context", "missing-profile", "foreign-workspace-profile"])
def test_journal_views_without_an_active_profile_match_python(tmp_path, environment, context):
    root = processed(seed(tmp_path / "python", shape="changed"), environment)
    with open_db(root) as conn:
        if context == "no-context":
            conn.execute("DELETE FROM settings WHERE key IN ('context_workspace','context_profile')")
        elif context == "missing-profile":
            conn.execute("UPDATE settings SET value='gone' WHERE key='context_profile'")
        else:
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w2','Other','2020')")
            conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p2','w2','Elsewhere','2020')")
            conn.execute("UPDATE settings SET value='p2' WHERE key='context_profile'")
    conn.close()
    compare(tmp_path, environment, root)


@pytest.mark.parametrize("marker", ["text-count", "real-version", "bad-text", "blob-version", "stale-version"])
def test_journal_views_malformed_freshness_markers_match_python(tmp_path, environment, marker):
    root = processed(seed(tmp_path / "python", shape="changed"), environment)
    with open_db(root) as conn:
        conn.execute({
            "text-count": "UPDATE profiles SET last_processed_tx_count=CAST(last_processed_tx_count AS TEXT)||' '",
            "real-version": "UPDATE profiles SET journal_input_version=journal_input_version+0.5",
            "bad-text": "UPDATE profiles SET last_processed_input_version='x'",
            "blob-version": "UPDATE profiles SET journal_input_version=CAST('7' AS BLOB)",
            "stale-version": "UPDATE profiles SET journal_input_version=journal_input_version+1",
        }[marker])
    conn.close()
    compare(tmp_path, environment, root)

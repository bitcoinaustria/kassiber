import json
import os
from pathlib import Path
import subprocess

import pytest

from kassiber.core import freshness
from kassiber.daemon_freshness import _freshness_wallet_source_specs
from kassiber.envelope import build_envelope, build_error_envelope
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, create_book, dump_bytes
from tests.test_rust_freshness_queue import FIXTURE, op, parity
from tests.test_rust_wallet_discovery import _config

pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires the Rust freshness fixture")


def config_sql(config, wallet="a"):
    return "UPDATE wallets SET config_json='" + json.dumps(config).replace("'", "''") + "' WHERE id='" + wallet + "';"


@pytest.mark.parametrize("config,kind", [
    ({}, "custom"),
    ({"descriptor": "test"}, "descriptor"),
    ({"descriptor": "   "}, "descriptor"),
    ({"xpub": "test", "script_types": ["p2wpkh"]}, "xpub"),
    ({"xpub": "test", "script_types": []}, "xpub"),
    ({"change_descriptor": "test"}, "descriptor"),
    ({"addresses": " , one, one, two , "}, "address"),
    ({"addresses": ["", " ", "one"]}, "address"),
    ({"descriptor": "test", "source_file": "inert.csv", "source_format": "csv"}, "custom"),
    ({"descriptor": "test", "source_file": "inert.csv"}, "custom"),
    ({"descriptor": "test"}, "coreln"),
    ({"descriptor": "test"}, "lnd"),
    ({"descriptor": "test", "deprecated": "yes"}, "descriptor"),
    # Retired with its history imported: unscoped discovery skips it.
    ({"descriptor": "test", "deprecated": "yes", "last_synced_at": "2026-01-01T00:00:00Z"}, "descriptor"),
    ({"sync_source": " BTCPAY ", "backend": "merchant", "store_id": "store"}, "custom"),
    ({"descriptor": "test", "sync_source": "btcpay", "backend": "merchant", "store_id": "store",
      "btcpay_provenance": [{"backend": "merchant", "store_id": "store"}]}, "descriptor"),
    ({"btcpay_provenance": [{"backend": "merchant", "store_id": "store"}]}, "custom"),
])
def test_freshness_source_specs_match_python(tmp_path, config, kind):
    sql = config_sql(config) + f"UPDATE wallets SET kind='{kind}';"
    parity(tmp_path, [op("specs", profile_id="p"),
                     op("specs", profile_id="p", wallet_ref="a", force_full=True, auto_pair_before_journals=True),
                     op("specs", profile_id="p", include_rates=False, include_journals=False)], sql)


@pytest.mark.parametrize("config", [
    {"sync_source": "other"}, {"sync_source": "btcpay"},
    {"sync_source": "btcpay", "backend": "merchant"},
    {"btcpay_provenance": {}}, {"btcpay_provenance": [1]},
    {"btcpay_provenance": [{"backend": "merchant"}]}, {"deprecated": "maybe"},
])
def test_freshness_source_validation_matches_python(tmp_path, config):
    parity(tmp_path, [op("specs", profile_id="p")], config_sql(config))


def test_freshness_source_account_routes_order_and_wallet_scope(tmp_path):
    sql = """
        INSERT INTO btcpay_account_routes(id,workspace_id,profile_id,backend_name,store_id,payment_method_id,action,label,created_at,updated_at)
        VALUES('z','w','p','z','s','BTC-CHAIN','provenance_only','','fixed','fixed'),
              ('b','w','p','a','s','BTC-LN','provenance_only','','fixed','fixed'),
              ('a','w','p','a','s','BTC-CHAIN','provenance_only','','fixed','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        VALUES('b','w','p','Alpha','descriptor','{"descriptor":"test"}','fixed');
    """
    parity(tmp_path, [op("specs", profile_id="p", force_full=True),
                     op("specs", profile_id="p", wallet_ref="a"),
                     op("specs", profile_id="p", wallet_ref="alpha"),
                     op("specs", profile_id="p", wallet_ref="missing")], sql)


@pytest.mark.parametrize("force", [False, True])
def test_freshness_source_policy_filter(tmp_path, force):
    specs = [{"source_type": name, "payload": {}} for name in ["journals", "onchain_wallet", "market_rates", "unknown"]]
    policy = {"background_enabled": False, "report_read_sync": False,
              "source_classes": {"onchain_wallet": True, "journals": True, "market_rates": False}}
    parity(tmp_path, [op("filter_specs", specs=specs, policy=policy, force=force)])


def backend(protocol="esplora"):
    return {"name": "fixture", "kind": protocol, "url": "http://127.0.0.1:1" if protocol != "electrum" else "tcp://127.0.0.1:1", "batch_size": 2}


def _book(tmp_path, config=None, sql=""):
    book = tmp_path / "kassiber.sqlite3"
    create_book(book, _config("wpkh-fixed", 3) if config is None else config, extra_sql="""
        INSERT INTO settings VALUES('context_workspace','w');
        INSERT INTO settings VALUES('context_profile','p');
    """ + sql)
    return book


def _invoke(tmp_path, book, operations, **extra_env):
    result = subprocess.run([str(Path(FIXTURE).resolve())],
                            input=json.dumps({"book": str(book), "operations": operations}).encode(),
                            capture_output=True, timeout=30, env={**child_env(tmp_path), **extra_env})
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    return json.loads(result.stdout)["results"]


def plan_op(name="plan", **args):
    return op(name, journals=False, rates=False, backend=backend(), **args)


def _expected(conn, workspace=None, wallet=None, force_full=False):
    if workspace is None:
        profiles = conn.execute("SELECT * FROM profiles WHERE id=(SELECT value FROM settings WHERE key='context_profile')").fetchall()
    else:
        profiles = conn.execute("SELECT * FROM profiles WHERE workspace_id=? ORDER BY created_at ASC,label ASC", (workspace,)).fetchall()
    return {"workspace": None if workspace is None else dict(conn.execute("SELECT id,label FROM workspaces WHERE id=?", (workspace,)).fetchone()),
            "books": [{"profile": {"id": row["id"], "label": row["label"]},
                       "specs": _freshness_wallet_source_specs(conn, row["id"], wallet_ref=wallet, include_rates=False,
                                                              include_journals=False, force_full=force_full)} for row in profiles]}


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("wallet,force", [(None, False), ("a", True), ("cold", False)])
def test_freshness_admission_is_inert_and_matches_source_plan(tmp_path, protocol, wallet, force):
    book = _book(tmp_path)
    before = dump_bytes(book)
    args = {"journals": False, "rates": False, "backend": backend(protocol), "force_full": force}
    if wallet:
        args["wallet"] = wallet
    actual = _invoke(tmp_path, book, [op("plan", **args)], KASSIBER_TEST_NO_EGRESS="1", KASSIBER_NO_EGRESS="1")
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn, wallet=wallet, force_full=force))]
    assert dump_bytes(book) == before


def test_freshness_admission_workspace_order_is_inert(tmp_path):
    sql = """
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at)
        VALUES('q','w','Alpha','EUR','aaa'),('r','w','Last','EUR','zzz');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        SELECT 'b','w','q','Other','descriptor',config_json,'fixed' FROM wallets WHERE id='a';
    """
    book = _book(tmp_path, sql=sql)
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op(workspace_id="w")])
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn, workspace="w"))]
    assert dump_bytes(book) == before


def test_freshness_admission_honors_disabled_journal_policy(tmp_path):
    book = _book(tmp_path, sql="INSERT INTO settings VALUES('freshness.policy.profile.p','{\"source_classes\":{\"journals\":false}}');")
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [op("plan", backend=backend()), plan_op()])
    assert actual[0]["kind"] == "plan"
    assert actual[1]["kind"] == "plan"
    assert dump_bytes(book) == before


def test_freshness_admission_no_active_profile_and_local_only_wallets(tmp_path):
    book = _book(tmp_path, config={})
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op(), op("sql", sql="UPDATE settings SET value='' WHERE key='context_profile'"), plan_op()])
    assert actual[0]["data"]["books"] == [{"profile": {"id": "p", "label": "book"}, "specs": []}]
    assert actual[2]["data"] == {"workspace": None, "books": []}
    with connect(book) as conn:
        conn.execute("UPDATE settings SET value='p' WHERE key='context_profile'")
    assert dump_bytes(book) == before


@pytest.mark.parametrize("kind", ["silent-payment", "coreln", "lnd", "nwc", "phoenix"])
def test_freshness_admission_plans_kinds_without_a_freshness_source(tmp_path, kind):
    # Python plans no job for these wallets; only a direct sync reaches them
    # (docs/reference/rust-native-wallet-gaps.md, S5), so a run plans.
    book = _book(tmp_path, config={}, sql=f"UPDATE wallets SET kind='{kind}';")
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op()])
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn))]
    assert dump_bytes(book) == before


def test_freshness_admission_plans_every_wallet_an_id_or_label_selects(tmp_path):
    # `_load_wallets_for_freshness` matches the id and every label.
    sql = """
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        SELECT 'b','w','p','a','descriptor',config_json,'fixed' FROM wallets WHERE id='a';
    """
    book = _book(tmp_path, sql=sql)
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op(wallet="a")], KASSIBER_TEST_NO_EGRESS="1", KASSIBER_NO_EGRESS="1")
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn, wallet="a"))]
        assert len(actual[0]["data"]["books"][0]["specs"]) == 2
    assert dump_bytes(book) == before


@pytest.mark.parametrize("args,code", [({"wallet": "missing"}, "not_found"),
                                        ({"wallet": " "}, "validation"),
                                        ({"workspace_id": "missing"}, "not_found"),
                                        ({"workspace_id": " "}, "validation"),
                                        ({"workspace_id": "w", "force_full": True}, "validation")])
def test_freshness_admission_validation_is_not_capability_fallback(tmp_path, args, code):
    book = _book(tmp_path)
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op(**args), plan_op()])
    assert actual[0]["error"]["code"] == code
    assert actual[1]["kind"] == "plan"
    assert dump_bytes(book) == before


@pytest.mark.parametrize("config,extra_sql", [
    ({"source_file": "wallet.csv", "source_format": "csv"}, ""),
    ({"sync_source": "btcpay", "backend": "merchant", "store_id": "store"}, ""),
    (None, "INSERT INTO settings VALUES('freshness.policy.profile.p','{\"source_classes\":{\"market_rates\":true}}');"),
])
def test_freshness_admission_plans_btcpay_rates_and_file_sources(tmp_path, config, extra_sql):
    # BTCPay and market-rate jobs run natively and file sources plan no job
    # (docs/reference/rust-native-wallet-gaps.md, S4 and S5).
    book = _book(tmp_path, config, extra_sql)
    before = dump_bytes(book)
    result = _invoke(tmp_path, book, [op("plan", **{"backend": backend(), "journals": False})])[0]
    assert result["kind"] == "plan", result
    assert dump_bytes(book) == before


@pytest.mark.parametrize("config,extra_sql,args,reason", [
    (None, "", {"journals": False, "backend": backend("custom")}, "backend_kind"),
])
def test_freshness_admission_refuses_unsupported_whole_shapes(tmp_path, config, extra_sql, args, reason):
    book = _book(tmp_path, config, extra_sql)
    before = dump_bytes(book)
    result = _invoke(tmp_path, book, [op("plan", **{"backend": backend(), **args})])[0]
    assert result["kind"] == "error"
    assert result["error"]["code"] == "native_fallback_required"
    if reason is not None:
        assert result["error"]["details"]["reason"] == reason
    assert dump_bytes(book) == before


def test_freshness_admission_plans_an_address_list(tmp_path):
    # Address lists are native (docs/reference/rust-native-wallet-gaps.md, P5);
    # an address Python cannot decode fails its job, not the plan.
    book = _book(tmp_path, {"addresses": ["one"]})
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op()], KASSIBER_TEST_NO_EGRESS="1", KASSIBER_NO_EGRESS="1")
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn))]
    assert dump_bytes(book) == before


def test_freshness_admission_plans_a_profile_with_several_wallets(tmp_path):
    # Source ownership across wallets is native (docs/reference/rust-native-wallet-gaps.md, P1).
    sql = ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
           "VALUES('b','w','p','Second','custom','{}','fixed');")
    sql += ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
            "SELECT 'c','w','p','Hot','descriptor',config_json,'fixed' FROM wallets WHERE id='a';")
    book = _book(tmp_path, None, sql)
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op()], KASSIBER_TEST_NO_EGRESS="1", KASSIBER_NO_EGRESS="1")
    with connect(book) as conn:
        assert actual == [build_envelope("plan", _expected(conn))]
    assert dump_bytes(book) == before


def test_freshness_admission_mixed_workspace_refuses_before_recovery(tmp_path):
    sql = """
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','unsupported','EUR','zzz');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        SELECT 'b','w','q','Other backend','descriptor',json_set(config_json,'$.backend','other'),'fixed'
        FROM wallets WHERE id='a';
    """
    book = _book(tmp_path, sql=sql)
    with connect(book) as conn:
        freshness.enqueue_job(conn, profile_id="p", job_type="onchain_wallet_history", source_key="onchain_wallet:a",
                              source_type="onchain_wallet", source_label="test")
        conn.execute("UPDATE freshness_jobs SET status='running'")
    before = dump_bytes(book)
    actual = _invoke(tmp_path, book, [plan_op(workspace_id="w"), plan_op()])
    assert actual[0]["error"]["details"]["reason"] == "backend_selection"
    assert actual[1]["kind"] == "plan"
    assert dump_bytes(book) == before


def test_freshness_admission_execution_slot_released_after_refusal_and_drop(tmp_path):
    book = _book(tmp_path)
    before = dump_bytes(book)
    result = _invoke(tmp_path, book, [plan_op("plan_busy"), plan_op(),
        op("plan", backend=backend("custom")), plan_op()])
    assert result[0] == build_error_envelope("project_operation_in_progress", "Another refresh is still running",
                                           hint="Wait for the current refresh to finish, then retry.", retryable=True)
    assert [r["kind"] for r in result] == ["error", "plan", "error", "plan"]
    assert dump_bytes(book) == before


@pytest.mark.parametrize("sql", [
    "UPDATE settings SET value='' WHERE key='context_profile'",
    "UPDATE wallets SET label='Changed'",
    "UPDATE wallets SET config_json='{}'",
    "UPDATE profiles SET label='Changed'",
    "INSERT INTO settings VALUES('database_instance_id','changed')",
    "INSERT INTO settings VALUES('freshness.policy.profile.p','{\"background_enabled\":true}')",
])
def test_freshness_admission_rechecks_scope_config_and_policy(tmp_path, sql):
    book = _book(tmp_path)
    actual = _invoke(tmp_path, book, [plan_op("plan_changed", sql=sql)])
    assert actual[0]["error"]["code"] == "stale_context"


def test_freshness_admission_rejects_different_connection_and_backend(tmp_path):
    book = _book(tmp_path)
    before = dump_bytes(book)
    result = _invoke(tmp_path, book, [plan_op("plan_other_connection"), plan_op(changed_backend={**backend(), "url": "http://127.0.0.1:2"})])
    assert [row["error"]["code"] for row in result] == ["stale_context", "stale_context"]
    assert dump_bytes(book) == before


@pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires the guarded Rust daemon")
def test_freshness_admission_uses_admitted_book_seam(tmp_path):
    book = _book(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {**child_env(tmp_path), "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
           "KASSIBER_BACKEND_FIXTURE_KIND": "esplora", "KASSIBER_BACKEND_FIXTURE_URL": "http://127.0.0.1:1",
           "KASSIBER_DEFAULT_BACKEND": "fixture", "KASSIBER_TEST_NO_EGRESS": "1", "KASSIBER_NO_EGRESS": "1"}
    requests = [{"request_id": "plan", "kind": "test.freshness.plan", "args": {"rates": False, "journals": False}},
                {"request_id": "stop", "kind": "daemon.shutdown"}]
    result = subprocess.run([str(Path(os.environ["KASSIBER_RUST_BIN"]).resolve()), "--data-root", str(tmp_path), "daemon"],
                            input="".join(json.dumps(row) + "\n" for row in requests), text=True,
                            capture_output=True, timeout=30, env=env)
    assert result.returncode == 0 and result.stderr == "", result.stderr
    rows = [json.loads(row) for row in result.stdout.splitlines()]
    assert [row["kind"] for row in rows] == ["daemon.ready", "test.freshness.plan", "daemon.shutdown"]
    with connect(book) as conn:
        assert rows[1] == {**build_envelope("test.freshness.plan", _expected(conn)), "request_id": "plan"}
    assert "ui.wallets.sync" in rows[0]["data"]["supported_kinds"]

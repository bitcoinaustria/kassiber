"""Offline mode: the persisted switch and every guard that must honor it.

`tests/conftest.py` points the preference at a disposable file for the whole
run; each test here points it at its own file so the switch starts off and one
test's choice cannot leak into another.
"""

from __future__ import annotations

import io
import json
import os
import queue
import threading
import urllib.request
from pathlib import Path
from unittest import mock

import pytest

from kassiber import egress_policy
from kassiber.ai.contracts import CLI_PROVIDER_LOCATORS
from kassiber.errors import AppError


@pytest.fixture
def preference(tmp_path, monkeypatch) -> Path:
    path = tmp_path / "config" / egress_policy.OFFLINE_FILENAME
    monkeypatch.setenv(egress_policy.OFFLINE_PREFERENCE_ENV, str(path))
    monkeypatch.delenv(egress_policy.NO_EGRESS_ENV, raising=False)
    return path


def _assert_offline_error(excinfo) -> None:
    assert excinfo.value.code == "network_egress_disabled"
    assert "offline mode" in str(excinfo.value)
    assert excinfo.value.retryable is False


def test_missing_preference_is_online(preference):
    assert not preference.exists()
    assert egress_policy.offline_mode_enabled() is False
    assert egress_policy.egress_block_reason() is None
    assert egress_policy.offline_status() == {"offline": False, "environment_blocked": False}


def test_switch_round_trips_through_an_owner_only_file(preference):
    egress_policy.set_offline_mode(True)
    assert egress_policy.offline_mode_enabled() is True
    assert preference.stat().st_mode & 0o077 == 0
    assert json.loads(preference.read_text()) == {"enabled": True, "schema_version": 1}

    egress_policy.set_offline_mode(False)
    assert egress_policy.offline_mode_enabled() is False


@pytest.mark.parametrize(
    "content",
    [
        "not json",
        "[]",
        '{"schema_version": 2, "enabled": false}',
        '{"schema_version": 1, "enabled": "no"}',
        '{"schema_version": 1}',
    ],
)
def test_unreadable_preference_fails_closed(preference, content):
    preference.parent.mkdir(parents=True)
    preference.write_text(content)
    preference.chmod(0o600)
    assert egress_policy.offline_mode_enabled() is True


def test_symlinked_preference_fails_closed(preference, tmp_path):
    target = tmp_path / "elsewhere.json"
    target.write_text('{"schema_version": 1, "enabled": false}')
    preference.parent.mkdir(parents=True)
    preference.symlink_to(target)
    assert egress_policy.offline_mode_enabled() is True


def test_environment_switch_takes_precedence(preference, monkeypatch):
    egress_policy.set_offline_mode(True)
    monkeypatch.setenv(egress_policy.NO_EGRESS_ENV, "1")
    assert egress_policy.egress_block_reason() == egress_policy.REASON_ENVIRONMENT
    with pytest.raises(AppError, match="KASSIBER_NO_EGRESS"):
        egress_policy.require_egress_enabled("Outbound chain observation is")
    assert egress_policy.offline_status() == {"offline": True, "environment_blocked": True}


def test_transport_guard_names_offline_mode(preference):
    egress_policy.require_egress_enabled()
    egress_policy.set_offline_mode(True)
    with pytest.raises(AppError) as excinfo:
        egress_policy.require_egress_enabled("Outbound chain observation is")
    _assert_offline_error(excinfo)
    assert str(excinfo.value).startswith("Outbound chain observation is disabled")
    assert excinfo.value.hint == "Turn off offline mode to connect."


@pytest.mark.parametrize(
    "url",
    ["http://localhost:11434/v1", "http://127.0.0.1:8080/v1", "http://[::1]:1234/v1"],
)
def test_require_online_allows_on_device_urls(preference, url):
    egress_policy.set_offline_mode(True)
    egress_policy.require_online("Remote AI providers are", on_device_url=url)


@pytest.mark.parametrize(
    "url",
    [
        "https://api.openai.com/v1",
        "http://192.168.1.20:11434/v1",
        "http://localhost.example/v1",
        "ftp://127.0.0.1/v1",
        None,
    ],
)
def test_require_online_blocks_everything_else(preference, url):
    egress_policy.require_online("Remote AI providers are", on_device_url=url)
    egress_policy.set_offline_mode(True)
    with pytest.raises(AppError) as excinfo:
        egress_policy.require_online("Remote AI providers are", on_device_url=url)
    _assert_offline_error(excinfo)


def test_shared_transport_refuses_before_opening(preference):
    from kassiber import proxy

    egress_policy.set_offline_mode(True)
    with mock.patch.object(proxy.urlrequest, "build_opener") as build_opener:
        with pytest.raises(AppError) as excinfo:
            proxy.urlopen_with_proxy(urllib.request.Request("https://mempool.space/api/blocks/tip/height"))
    _assert_offline_error(excinfo)
    build_opener.assert_not_called()


def test_ai_client_keeps_local_models_and_blocks_remote_ones(preference):
    from kassiber.ai import client as ai_client

    egress_policy.set_offline_mode(True)
    opener = mock.Mock()
    opener.open.return_value = io.BytesIO(b"{}")
    with mock.patch.object(ai_client.urllib.request, "build_opener", return_value=opener):
        local = ai_client.ai_client_for_locator("http://127.0.0.1:11434/v1", kind="local")
        local._open("models", method="GET", body=None, accept_sse=False)
        assert opener.open.call_count == 1

        remote = ai_client.ai_client_for_locator(
            "https://api.openai.com/v1", api_key="test-key", kind="remote"
        )
        with pytest.raises(AppError) as excinfo:
            remote._open("models", method="GET", body=None, accept_sse=False)
    _assert_offline_error(excinfo)
    assert opener.open.call_count == 1


@pytest.mark.parametrize("kind", ["remote", "tee", None])
def test_ai_client_blocks_a_loopback_gateway_to_a_remote_model(preference, kind):
    # A local gateway or tunnel on a loopback URL still forwards the prompt off
    # the machine; only a provider marked local counts as on-device.
    from kassiber.ai import client as ai_client

    egress_policy.set_offline_mode(True)
    client = ai_client.ai_client_for_locator("http://127.0.0.1:4000/v1", api_key="k", kind=kind)
    with mock.patch.object(ai_client.urllib.request, "build_opener") as build_opener:
        with pytest.raises(AppError) as excinfo:
            client._open("models", method="GET", body=None, accept_sse=False)
    _assert_offline_error(excinfo)
    build_opener.assert_not_called()


@pytest.mark.parametrize("locator", CLI_PROVIDER_LOCATORS)
def test_cli_provider_broker_refuses_before_spawning(preference, locator):
    from kassiber.ai import broker_client

    egress_policy.set_offline_mode(True)
    client = broker_client.BrokerAIClient(locator=locator)
    with (
        mock.patch.object(broker_client.subprocess, "run") as run,
        mock.patch.object(broker_client.subprocess, "Popen") as popen,
    ):
        # Model discovery and runtime status start the provider CLIs too.
        for probe in (client.list_models, broker_client.BrokerAIClient.runtime_status):
            with pytest.raises(AppError) as excinfo:
                probe()
            _assert_offline_error(excinfo)
        with pytest.raises(AppError) as excinfo:
            list(client.stream_chat(messages=[{"role": "user", "content": "hi"}], model="m"))
        _assert_offline_error(excinfo)
    run.assert_not_called()
    popen.assert_not_called()


def test_open_electrum_session_stops_once_offline(preference):
    from kassiber.core import sync_backends

    client = sync_backends.ElectrumClient({"name": "fulcrum", "url": "ssl://fulcrum.example:50002"})
    client.socket = mock.Mock()
    client.reader = io.BytesIO(b'{"jsonrpc": "2.0", "id": 1, "result": 1}\n')
    assert client.call("server.ping") == 1
    assert client.socket.sendall.call_count == 1

    # Switched on mid-sync: the already open session sends nothing more.
    egress_policy.set_offline_mode(True)
    with pytest.raises(AppError) as excinfo:
        client.call("blockchain.scripthash.get_history", ["00" * 32])
    _assert_offline_error(excinfo)
    with pytest.raises(AppError) as excinfo:
        client.batch_call([("blockchain.scripthash.get_history", ["00" * 32])])
    _assert_offline_error(excinfo)
    assert client.socket.sendall.call_count == 1


def test_update_check_refuses_offline_even_with_consent(preference, tmp_path):
    from kassiber import update_check

    consent = tmp_path / "update-consent.json"
    update_check.set_update_checks_enabled(True, consent)
    egress_policy.set_offline_mode(True)
    opener = mock.Mock()
    with pytest.raises(AppError) as excinfo:
        update_check.fetch_latest_release(opener=opener, consent=consent)
    _assert_offline_error(excinfo)
    opener.assert_not_called()


def test_core_lightning_refuses_before_spawning(preference):
    from kassiber.core.lightning import cln

    egress_policy.set_offline_mode(True)
    with mock.patch.object(cln.subprocess, "run") as run:
        with pytest.raises(AppError) as excinfo:
            cln.call_core_lightning({"kind": "coreln"}, "getinfo")
    _assert_offline_error(excinfo)
    run.assert_not_called()


def test_background_acquisition_pauses_offline_but_watches_still_run(preference):
    from kassiber import daemon_chain_analysis_watches as watch_worker

    egress_policy.set_offline_mode(True)
    conn = mock.Mock()
    conn.execute.return_value.fetchall.return_value = [{"profile_id": "p1"}]
    conn.execute.return_value.fetchone.return_value = (1,)
    with (
        mock.patch.object(watch_worker, "has_pending_work", return_value=True),
        mock.patch.object(watch_worker.backfill, "run_due") as run_due,
        mock.patch.object(watch_worker.watches, "evaluate_due") as evaluate_due,
    ):
        watch_worker.worker_tick(conn)
        run_due.assert_not_called()
        evaluate_due.assert_called_once()

        egress_policy.set_offline_mode(False)
        watch_worker.worker_tick(conn)
        run_due.assert_called_once()


def test_explicit_acquisition_run_refuses_offline(preference):
    from kassiber.core import chain_analysis_backfill

    egress_policy.set_offline_mode(True)
    conn = mock.Mock()
    with pytest.raises(AppError) as excinfo:
        chain_analysis_backfill.dispatch(conn, "p1", "sources.run", {"id": "grant"})
    _assert_offline_error(excinfo)
    conn.execute.assert_not_called()


def test_background_refresh_pauses_offline(preference):
    from kassiber import daemon_freshness

    egress_policy.set_offline_mode(True)
    conn = mock.Mock()
    with mock.patch.object(daemon_freshness, "_active_profile_row") as active_profile:
        daemon_freshness._freshness_background_tick(conn, {}, mock.Mock())
    active_profile.assert_not_called()
    conn.execute.assert_not_called()


def _locked_daemon_context(tmp_path):
    from kassiber import daemon
    from kassiber.secrets.auth_backoff import AuthAttemptBackoff

    return daemon.DaemonContext(
        conn=None,
        data_root=str(tmp_path / "data"),
        runtime_config={},
        active_ai_chats=daemon.ActiveAiChats(),
        main_thread_tasks=queue.Queue(),
        auth_backoff=AuthAttemptBackoff(str(tmp_path / "auth.json")),
        input_lines=queue.Queue(),
        deferred_input_lines=[],
        out=io.StringIO(),
        freshness_stop_event=threading.Event(),
    )


def test_daemon_reads_and_sets_the_switch_while_locked(preference, tmp_path):
    from kassiber import daemon

    ctx = _locked_daemon_context(tmp_path)
    reply, _ = daemon.handle_request(ctx, {"kind": "ui.network.offline", "request_id": "r1"}, ctx.out)
    assert reply["request_id"] == "r1"
    assert reply["data"] == {"offline": False, "environment_blocked": False}

    reply, _ = daemon.handle_request(
        ctx,
        {"kind": "ui.network.offline.set", "request_id": "r2", "args": {"enabled": True}},
        ctx.out,
    )
    assert reply["kind"] == "ui.network.offline.set"
    assert reply["data"]["offline"] is True
    assert egress_policy.offline_mode_enabled() is True


@pytest.mark.parametrize(
    ("kind", "args"),
    [
        ("ui.network.offline.set", {"enabled": "true"}),
        ("ui.network.offline.set", {}),
        ("ui.network.offline.set", {"enabled": True, "extra": 1}),
        ("ui.network.offline", {"enabled": True}),
    ],
)
def test_daemon_rejects_malformed_switch_requests(preference, tmp_path, kind, args):
    from kassiber import daemon

    ctx = _locked_daemon_context(tmp_path)
    with pytest.raises(AppError) as excinfo:
        daemon.handle_request(ctx, {"kind": kind, "request_id": "bad", "args": args}, ctx.out)
    assert excinfo.value.code == "validation"
    assert not preference.exists()


def test_switch_is_read_and_set_by_capability(preference):
    from kassiber.command_capabilities import Capability, cli_capability, daemon_capability

    assert daemon_capability("ui.network.offline") is Capability.READ
    assert daemon_capability("ui.network.offline.set") is Capability.OPERATOR
    assert cli_capability("offline.status") is Capability.READ
    assert cli_capability("offline.on") is Capability.OPERATOR
    assert cli_capability("offline.off") is Capability.OPERATOR


def test_cli_switch_shares_the_daemon_preference(preference, tmp_path, capsys):
    from kassiber.cli.main import main

    data_root = str(tmp_path / "data")
    assert main(["--data-root", data_root, "--machine", "offline", "on"]) == 0
    assert json.loads(capsys.readouterr().out)["data"]["offline"] is True
    assert egress_policy.offline_mode_enabled() is True
    assert main(["--data-root", data_root, "--machine", "offline", "status"]) == 0
    envelope = json.loads(capsys.readouterr().out)
    assert envelope["kind"] == "offline.status"
    assert envelope["data"] == {"offline": True, "environment_blocked": False}
    assert main(["--data-root", data_root, "--machine", "offline", "off"]) == 0
    assert egress_policy.offline_mode_enabled() is False
    assert os.path.exists(preference)

"""Backend edits must preserve the desktop daemon session."""
from types import SimpleNamespace

from kassiber import backends, daemon
from kassiber.daemon_backup import BackupSessions
from kassiber.db import open_db


def test_configuration_mutations_keep_daemon_running(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        runtime = backends.load_runtime_config(tmp_path / "absent.env")
        ctx = SimpleNamespace(conn=conn, runtime_config=runtime, backup_sessions=BackupSessions())
        for action, args in (
            ("create", {"name": "local", "kind": "electrum", "url": "ssl://example.test:50002"}),
            ("update", {"name": "local", "config": {"display_name": "Edited"}}),
            ("delete", {"name": "local"}),
        ):
            response, should_shutdown = daemon.handle_request(
                ctx, {"kind": "ui.backends." + action, "request_id": action, "args": args}, None,
            )
            assert response["kind"] == "ui.backends." + action
            assert should_shutdown is False, action
            following, should_shutdown = daemon.handle_request(
                ctx, {"kind": "ui.backends.options", "request_id": "after-" + action}, None,
            )
            assert following["kind"] == "ui.backends.options"
            assert should_shutdown is False
    finally:
        conn.close()


def test_malformed_backend_locations_never_echo_embedded_credentials():
    for redact, locations in (
        (backends.redact_backend_url, (
            "https://alice:synthetic-password@[invalid]/api?token=synthetic-token",
            "https://alice:synthetic-password@/api?token=synthetic-token",
            "https://alice:synthetic-password@example.test:synthetic-token/api",
            "https:alice:synthetic-password@example.test?token=synthetic-token",
        )),
        (backends.redact_backend_proxy_url, (
            "socks5h://alice:synthetic-password@[invalid]:9050",
            "socks5h://alice:synthetic-password@:9050",
            "alice:synthetic-password@example.test:synthetic-token",
        )),
    ):
        for location in locations:
            assert redact(location) == "<redacted>"

"""The hybrid sync handoff is a pre-consent signal, never an error retry."""
import io
import json
from subprocess import CompletedProcess
from types import SimpleNamespace

import pytest

from tests import cli_runner
from tests.rust_port import hybrid_cli


@pytest.mark.parametrize("argument", ["--db-passphrase-fd", "--db-passphrase-fd=9", "tty"])
def test_sync_hybrid_keeps_consumable_unlock_inputs_on_python(monkeypatch, argument):
    monkeypatch.setattr(hybrid_cli.subprocess, "run", lambda *a, **kw:
                        CompletedProcess(a, 0, b"wallets.sync\n", b""))
    monkeypatch.setattr(hybrid_cli.os, "isatty", lambda fd: argument == "tty")
    assert hybrid_cli.route("rust", [argument, "wallets", "sync"]) == ("wallets.sync", "python")


@pytest.mark.parametrize("code,stdout", [(0, b'{"data": []}'),
                                         (1, b'{"error": {"code": "native_fallback_required"}}')])
def test_sync_hybrid_never_retries_a_native_result(monkeypatch, tmp_path, code, stdout):
    monkeypatch.setenv(hybrid_cli.RUST_ENV, "rust")
    monkeypatch.setenv(hybrid_cli.LOG_ENV, str(tmp_path / "served"))
    monkeypatch.setattr(hybrid_cli.sys, "argv", ["hybrid", "wallets", "sync"])
    monkeypatch.setattr(hybrid_cli, "route", lambda *a: ("wallets.sync", "rust"))
    output, errors = io.BytesIO(), io.BytesIO()
    monkeypatch.setattr(hybrid_cli.sys, "stdout", SimpleNamespace(buffer=output))
    monkeypatch.setattr(hybrid_cli.sys, "stderr", SimpleNamespace(buffer=errors))

    def native(*args, **kwargs):
        assert kwargs["env"]["KASSIBER_RUST_TEST_SYNC_HANDOFF"] == "1"
        return CompletedProcess(args, code, stdout, b"native-stderr")

    monkeypatch.setattr(hybrid_cli.subprocess, "run", native)
    monkeypatch.setattr(hybrid_cli.os, "execv", lambda *a: pytest.fail("native result retried"))
    with pytest.raises(SystemExit) as error:
        hybrid_cli.main()
    assert error.value.code == code
    assert output.getvalue() == stdout and errors.getvalue() == b"native-stderr"
    assert json.loads((tmp_path / "served").read_text())["served_by"] == "rust"


@pytest.mark.parametrize("stdout,stderr", [(b"", b""), (b"result", b""), (b"", b"warning")])
def test_sync_hybrid_requires_an_empty_explicit_handoff(monkeypatch, tmp_path, stdout, stderr):
    monkeypatch.setenv(hybrid_cli.RUST_ENV, "rust")
    monkeypatch.setenv(hybrid_cli.LOG_ENV, str(tmp_path / "served"))
    monkeypatch.delenv("KASSIBER_RUST_TEST_OWNER_ROOT", raising=False)
    monkeypatch.setattr(hybrid_cli.sys, "argv", ["hybrid", "wallets", "sync"])
    monkeypatch.setattr(hybrid_cli, "route", lambda *a: ("wallets.sync", "rust"))
    monkeypatch.setattr(hybrid_cli.subprocess, "run", lambda *a, **kw:
                        CompletedProcess(a, hybrid_cli.SYNC_HANDOFF, stdout, stderr))
    calls = []

    def execute(*args):
        calls.append(args)
        raise SystemExit(23)

    monkeypatch.setattr(hybrid_cli.os, "execv", execute)
    with pytest.raises(SystemExit) as error:
        hybrid_cli.main()
    if stdout or stderr:
        assert not calls and "empty output" in str(error.value)
    else:
        assert error.value.code == 23
        assert calls == [(hybrid_cli.sys.executable,
                          cli_runner.argv(["wallets", "sync"], reference=True))]
        assert json.loads((tmp_path / "served").read_text())["served_by"] == "python"

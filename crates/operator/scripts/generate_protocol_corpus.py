#!/usr/bin/env python3
"""Regenerate the operator protocol corpus the Rust port is pinned to.

``crates/operator/tests/data/protocol_corpus.json`` holds, from the Python
reference (``kassiber/operator/protocol.py`` and ``client.py``):

- ``json_frames``: payloads (JSON text in Python's insertion order) and the
  exact ``J`` frame bytes ``BrokerChannel.send_json`` writes;
- ``secret_frames``: challenges, secrets and their ``S`` frame bytes;
- ``decode_errors``: frames ``receive_json``/``receive_secret`` refuse, with
  the exception Python raises;
- ``client``: scripted scenarios for every client action: the replies each
  connection answers, the exact bytes the client sent on each connection,
  the connect timeouts and the result or ``AppError``.

``tests/test_rust_operator_protocol.py`` regenerates it and fails on drift;
``crates/operator/tests/corpus.rs`` replays it against the Rust code.

    uv run --locked python crates/operator/scripts/generate_protocol_corpus.py
"""

from __future__ import annotations

import json
import os
import sys
from contextlib import ExitStack
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from kassiber.errors import AppError  # noqa: E402
from kassiber.operator import client as client_module  # noqa: E402
from kassiber.operator.client import BrokerClient, PreparedArguments  # noqa: E402
from kassiber.operator.protocol import (  # noqa: E402
    MAX_SECRET_PAYLOAD_BYTES,
    SECRET_CHALLENGE_WIRE_BYTES,
    BrokerChannel,
)

DEST = ROOT / "crates" / "operator" / "tests" / "data" / "protocol_corpus.json"
BUILD = {
    "version": "0.22.78",
    "channel": None,
    "commit": None,
    "frozen": False,
    "origin": "0123456789ab",
}
OTHER_BUILD = {**BUILD, "origin": "ffffffffffff"}


class _Capture:
    """A transport that records what is sent and serves fixed bytes."""

    def __init__(self, incoming: bytes = b"") -> None:
        self.sent = bytearray()
        self.incoming = bytearray(incoming)

    def send_all(self, payload: bytes) -> None:
        self.sent.extend(payload)

    def recv_exact(self, size: int) -> bytes:
        if len(self.incoming) < size:
            raise EOFError("operator IPC connection closed")
        chunk = bytes(self.incoming[:size])
        del self.incoming[:size]
        return chunk

    def close(self) -> None:
        pass


def _frame(payload: dict) -> bytes:
    capture = _Capture()
    BrokerChannel(capture).send_json(payload)
    return bytes(capture.sent)


def _secret(challenge: str, secret: bytes) -> bytes:
    capture = _Capture()
    BrokerChannel(capture).send_secret(challenge, secret)
    return bytes(capture.sent)


JSON_PAYLOADS = [
    {"version": 3, "action": "ping"},
    {"version": 3, "action": "agent_session_heartbeat"},
    {"z": 1, "a": {"y": [3, 2, 1], "b": None}, "m": True},
    {"text": "Bücher € \U0001f511 \x00\x1f\x7f \"\\/"},
    {"lone": "\ud800 and \udcfc"},
    {"é": 1, "e": 2, "\U0001f511": 3, "E": 4, "": 5},
    {"floats": [1.0, -0.0, 1e100, 1.5e-7, 0.1, 123456789.0, float("nan"), float("inf"), -float("inf")]},
    {"ints": [0, -1, 2**63, -(2**64), 10**40]},
    {"nested": {"deeper": {"deepest": [{}, [], ""]}}},
    {},
    {
        "ok": False,
        "error": {
            "code": "operator_client_limit",
            "message": "the operator broker has too many connected clients",
            "hint": "Close an idle client or retry after an active request finishes.",
            "details": {"limit": 64},
            "retryable": True,
        },
    },
]

SECRET_CASES = [
    ("challenge", b"not-in-json"),
    ("a" * SECRET_CHALLENGE_WIRE_BYTES, b""),
    ("c", "Synthetic '\U0001f511\nsecond line".encode("utf-8")),
    ("a" * SECRET_CHALLENGE_WIRE_BYTES, b"\x00\xff" * 8),
]

DECODE_CASES = [
    ("json", "", b"S\x00\x00\x00\x01x"),
    ("json", "", b"J\x00\x00\x00\x01\xff"),
    ("json", "", b"J\x00\x00\x00\x01{"),
    ("json", "", b"J\x00\x00\x00\x02[]"),
    ("json", "", b"J\x00\x80\x00\x01"),
    ("json", "", b"J\x00\x00\x00\x05{\"a\""),
    ("secret", "challenge", b"J\x00\x00\x00\x02{}"),
    ("secret", "challenge", b"S\x00\x00\x00\x09challenge"),
    ("secret", "challenge", b"S\x00\x00\x00\x0achallengX\x00"),
    ("secret", "challenge", b"S\x00\x00\x40\x01" + b"x" * 0x4001),
    ("secret", "challenge", b"S\x00\x00\x00\x0achallenge\x00"),
]


def _describe(exc: BaseException) -> dict:
    if isinstance(exc, AppError):
        return {
            "class": "AppError",
            "code": exc.code,
            "message": str(exc),
            "hint": exc.hint,
            "details": exc.details,
            "retryable": exc.retryable,
        }
    return {"class": type(exc).__name__}


def decode_errors() -> list[dict]:
    out = []
    for kind, challenge, frame in DECODE_CASES:
        channel = BrokerChannel(_Capture(frame))
        try:
            if kind == "json":
                result = {"value": channel.receive_json()}
            else:
                result = {"secret_hex": bytes(channel.receive_secret(challenge)).hex()}
        except Exception as exc:  # noqa: BLE001 - recorded for the port
            result = {"raises": _describe(exc)}
        out.append({"kind": kind, "challenge": challenge, "frame_hex": frame.hex(), **result})
    return out


# A scripted connection: a list of replies, or "refused".
# A reply: {"json": payload}, {"io": "ConnectionResetError"} or {"eof": true}.
def _ok(data: dict) -> dict:
    return {"json": {"ok": True, "data": data}}


def _ping(build: dict | None = BUILD, **extra: object) -> dict:
    data = {
        "broker": "running",
        "generation": "g",
        "native_auth_available": False,
        "native_auth_identity": None,
        "caller_context": True,
        **extra,
    }
    if build is not None:
        data["build"] = build
    return _ok(data)


def _continue(kind: str, build: dict | None = BUILD, **extra: object) -> dict:
    payload = {"ok": True, "continue": kind, **extra}
    if build is not None:
        payload["build"] = build
    return {"json": payload}


SCENARIOS = [
    {"name": "ping", "call": ["ping"], "connections": [[_ping()]]},
    {"name": "ping_refused", "call": ["ping"], "connections": ["refused"]},
    {"name": "status_all", "call": ["status", None], "connections": [[_ok({"broker": "running", "generation": "g", "leases": []})]]},
    {"name": "status_project", "call": ["status", "/project"], "connections": [[_ok({"broker": "running", "generation": "g", "project": "0123456789abcdef", "lease": "locked", "broker_build": BUILD})]]},
    {"name": "status_stopped", "call": ["status", "/project"], "connections": ["refused"]},
    {
        "name": "unlock_password",
        "call": ["unlock", "/project", "c2VjcmV0", 3600, "admin", "password", False, None, None],
        "connections": [[_ping()], [_continue("secret", challenge="c" * 48), _ok({"lease": "unlocked", "lease_id": "l"})]],
    },
    {
        "name": "unlock_agent",
        "call": ["unlock", "/project", "c2VjcmV0", None, "operator", "password", True, "Y29udHJvbA==", 900],
        "connections": [[_ping()], [_continue("secret", challenge="a" * 48, agent_control_challenge="b" * 48), _ok({"lease": "unlocked"})]],
    },
    {
        "name": "unlock_other_build",
        "call": ["unlock", "/project", "c2VjcmV0", None, "read", "password", False, None, None],
        "connections": [[_ping(OTHER_BUILD)]],
    },
    {
        "name": "unlock_bad_challenge",
        "call": ["unlock", "/project", "c2VjcmV0", None, "read", "password", False, None, None],
        "connections": [[_ping()], [_continue("secret", challenge=5)]],
    },
    {"name": "lock", "call": ["lock", "/project", "lease-1", True], "connections": [[_ok({"locked": True, "lease_existed": True})]]},
    {"name": "lock_plain", "call": ["lock", "/project", None, False], "connections": [[_ok({"locked": True})]]},
    {"name": "lock_stopped", "call": ["lock", "/project", None, False], "connections": ["refused"]},
    {
        "name": "submit",
        "call": ["submit", "/project", ["backends", "create", "--token-fd", "broker-secret-1"], [["broker-secret-1", "dG9rZW4="]], "YWRtaW4=", True, False, True, "agent-token"],
        "env": {"KASSIBER_NO_EGRESS": "yes"},
        "connections": [
            [_ping()],
            [
                _continue("argv"),
                _continue("secrets", challenges={"broker-secret-1": "d" * 48}, admin_challenge="e" * 48),
                _ok({"operation_id": "g.client." + "f" * 32, "state": "queued", "build": BUILD}),
            ],
        ],
    },
    {
        "name": "submit_lost_then_known",
        "call": ["submit", "/project", ["status"], [], None, True, False, False, None],
        "connections": [
            [_ping()],
            [{"io": "ConnectionResetError"}],
            "refused",
            [_ok({"operation_id": "g.client." + "f" * 32, "state": "completed", "exit_code": 0})],
        ],
    },
    {
        "name": "submit_no_broker",
        "call": ["submit", "/project", ["status"], [], None, False, False, False, None],
        "connections": ["refused"],
    },
    {"name": "operation_status", "call": ["operation_status", "g.client.1"], "connections": [[_ok({"operation_id": "g.client.1", "state": "running"})]]},
    {"name": "operation_status_unreachable", "call": ["operation_status", "g.client.1"], "connections": ["refused"]},
    {"name": "operation_cancel", "call": ["cancel", "g.client.1"], "connections": [[_ok({"operation_id": "g.client.1", "state": "cancelled"})]]},
    {
        "name": "set_mode",
        "call": ["set_mode", "/project", "brokered", "cGFzcw=="],
        "connections": [[_ping()], [_continue("secret", challenge="c" * 48), _ok({"mode": "brokered"})]],
    },
    {
        "name": "stop",
        "call": ["stop"],
        "connections": [[_ping()], [_ok({"restart": "accepted"})]],
    },
    {
        "name": "stop_busy",
        "call": ["stop"],
        "connections": [[_ping()], [{"json": {"ok": False, "error": {"code": "operator_broker_busy", "message": "busy", "hint": None, "details": {}, "retryable": True}}}]],
    },
    {
        "name": "stop_other_protocol",
        "call": ["stop"],
        "connections": [[{"json": {"ok": False, "error": {"code": "operator_protocol_version_mismatch", "message": "operator protocol version mismatch", "hint": None, "details": {"supported": 4}, "retryable": False}}}]],
    },
    {
        "name": "agent_session_open",
        "call": ["open_agent_session", "/project"],
        "connections": [[_ok({"session_id": "s-1", "state": "pending", "token": "t-1"})]],
    },
    {
        "name": "agent_session_decide",
        "call": ["decide_agent_session", "/project", "s-1", True, "Y29udHJvbA=="],
        "connections": [[_continue("secret", challenge="c" * 48), _ok({"session_id": "s-1", "state": "allowed"})]],
    },
    {
        "name": "unlock_touch_id",
        "call": ["unlock_touch_id", "/project", 600, "read"],
        "connections": [
            [_ping(native_auth_available=True, native_auth_identity="helper-identity")],
            [_ok({"lease": "unlocked"})],
        ],
    },
    {
        "name": "unlock_touch_id_unavailable",
        "call": ["unlock_touch_id", "/project", None, "read"],
        "connections": [[_ping()]],
    },
    {
        "name": "touch_id_configure",
        "call": ["configure_touch_id", "/project", "cGFzcw==", True],
        "connections": [
            [_ping(native_auth_available=True, native_auth_identity="helper-identity")],
            [_continue("secret", challenge="c" * 48), _ok({"configured": True})],
        ],
    },
    {
        "name": "broker_error_passthrough",
        "call": ["cancel", "g.client.1"],
        "connections": [[{"json": {"ok": False, "error": {"code": "operator_lease_required", "message": "lease \ud800 ended", "hint": "unlock first", "details": {"limit": 64, "nested": [1.5]}, "retryable": 1}}}]],
    },
    {"name": "invalid_response", "call": ["cancel", "g.client.1"], "connections": [[{"json": {"ok": True, "data": [1]}}]]},
    {"name": "eof_reply", "call": ["cancel", "g.client.1"], "connections": [[{"eof": True}]]},
]


class _ScriptedConnect:
    def __init__(self, connections: list) -> None:
        self.connections = list(connections)
        self.sent: list[bytes] = []
        self.timeouts: list[list] = []

    def __call__(self, *, timeout: float = 2.0, io_timeout: float | None = None) -> BrokerChannel:
        self.timeouts.append([timeout, io_timeout])
        script = self.connections.pop(0) if self.connections else "refused"
        if script == "refused":
            raise ConnectionRefusedError()
        incoming = bytearray()
        failure = None
        for reply in script:
            if "json" in reply:
                incoming.extend(_frame(reply["json"]))
            elif "io" in reply:
                failure = reply["io"]
        transport = _ScriptedTransport(bytes(incoming), failure)
        index = len(self.sent)
        self.sent.append(b"")

        def record(payload: bytes) -> None:
            self.sent[index] += payload

        transport.record = record
        return BrokerChannel(transport)


class _ScriptedTransport:
    def __init__(self, incoming: bytes, failure: str | None) -> None:
        self.incoming = bytearray(incoming)
        self.failure = failure
        self.record = None

    def send_all(self, payload: bytes) -> None:
        self.record(payload)

    def recv_exact(self, size: int) -> bytes:
        if not self.incoming and self.failure:
            raise {"ConnectionResetError": ConnectionResetError}[self.failure]()
        if len(self.incoming) < size:
            raise EOFError("operator IPC connection closed")
        chunk = bytes(self.incoming[:size])
        del self.incoming[:size]
        return chunk

    def close(self) -> None:
        pass


def _b64(text: str | None) -> bytearray | None:
    import base64

    return None if text is None else bytearray(base64.b64decode(text))


def _run(scenario: dict) -> dict:
    connect = _ScriptedConnect(scenario["connections"])
    launches = []
    client = BrokerClient()
    call = scenario["call"]
    with ExitStack() as stack:
        stack.enter_context(mock.patch.object(client_module, "connect", connect))
        stack.enter_context(mock.patch.object(client_module, "build_identity", lambda: dict(BUILD)))
        stack.enter_context(mock.patch.object(client_module.secrets, "token_hex", lambda n=16: "f" * (n * 2)))
        stack.enter_context(mock.patch.object(client_module.os, "getcwd", lambda: "/caller"))
        stack.enter_context(mock.patch.object(client_module.subprocess, "Popen", lambda *a, **k: launches.append(a)))
        stack.enter_context(mock.patch.object(client_module.time, "sleep", lambda _s: None))
        stack.enter_context(mock.patch.object(client_module, "_broker_startup_timeout_seconds", lambda: 0.0))
        stack.enter_context(mock.patch.dict(os.environ, {"KASSIBER_NO_EGRESS": "", **scenario.get("env", {})}))
        stack.enter_context(mock.patch.object(client_module.sys, "stderr", mock.Mock()))
        # Off macOS, or without a signed helper, no native-auth handoff runs.
        os.environ.pop("KASSIBER_NATIVE_AUTH_HELPER", None)
        try:
            method = call[0]
            if method == "ping":
                value = client.ping()
            elif method == "status":
                value = client.status(call[1])
            elif method == "unlock":
                value = client.unlock(
                    call[1], _b64(call[2]), duration_seconds=call[3], capability=call[4],
                    authentication_method=call[5], only_if_locked=call[6],
                    agent_control=None if call[7] is None else bytes(_b64(call[7])),
                    idle_timeout_seconds=call[8],
                )
            elif method == "lock":
                value = client.lock(call[1], expected_lease_id=call[2], restore_manual=call[3])
            elif method == "submit":
                prepared = PreparedArguments(list(call[2]), {label: _b64(value) for label, value in call[3]})
                value = client.submit(
                    call[1], prepared, admin_authentication=_b64(call[4]), start_broker=call[5],
                    require_caller_context=call[6], require_same_build=call[7], agent_session_token=call[8],
                )
            elif method == "operation_status":
                value = client.operation_status(call[1])
            elif method == "cancel":
                value = client.cancel(call[1])
            elif method == "set_mode":
                value = client.set_mode(call[1], call[2], _b64(call[3]))
            elif method == "stop":
                value = client.stop()
            elif method == "open_agent_session":
                handle = client.open_agent_session(call[1])
                value = {"session_id": handle.session_id, "token": handle.token}
                handle.close()
            elif method == "decide_agent_session":
                value = client.decide_agent_session(call[1], call[2], allow=call[3], agent_control=bytes(_b64(call[4])))
            elif method == "unlock_touch_id":
                value = client.unlock_touch_id(call[1], duration_seconds=call[2], capability=call[3])
            elif method == "configure_touch_id":
                value = client.configure_touch_id(call[1], _b64(call[2]), configured=call[3])
            else:  # pragma: no cover - a corpus authoring error
                raise AssertionError(method)
            result = {"value": value}
        except Exception as exc:  # noqa: BLE001 - recorded for the port
            result = {"raises": _describe(exc)}
    return {
        **scenario,
        "sent_hex": [sent.hex() for sent in connect.sent],
        "timeouts": connect.timeouts,
        "launches": len(launches),
        **result,
    }


def corpus() -> dict:
    return {
        "json_frames": [
            {"payload": json.dumps(payload), "frame_hex": _frame(payload).hex()} for payload in JSON_PAYLOADS
        ],
        "secret_frames": [
            {"challenge": challenge, "secret_hex": secret.hex(), "frame_hex": _secret(challenge, secret).hex()}
            for challenge, secret in SECRET_CASES
        ],
        "limits": {"max_secret_payload_bytes": MAX_SECRET_PAYLOAD_BYTES},
        "decode_errors": decode_errors(),
        "client": [_run(scenario) for scenario in SCENARIOS],
    }


def render() -> str:
    return json.dumps(corpus(), indent=1, sort_keys=False) + "\n"


if __name__ == "__main__":
    DEST.parent.mkdir(parents=True, exist_ok=True)
    DEST.write_text(render(), encoding="utf-8")

from __future__ import annotations

"""Backend-specific wallet sync helpers used by the CLI sync layer."""

import base64
import hashlib
import json
import os
import queue
import socket
import ssl
import stat
import threading
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager, nullcontext
from contextvars import ContextVar, copy_context
from dataclasses import replace
from decimal import Decimal
from pathlib import Path
from types import MappingProxyType
from urllib import parse as urlparse
from urllib import request as urlrequest

from .. import __version__
from .. import http_client
from ..backends import backend_batch_size, backend_timeout, backend_value, resolve_backend
from ..db import APP_NAME
from ..egress_ledger import get_egress_ledger
from ..envelope import json_ready
from ..errors import AppError
from ..msat import SATS_PER_BTC, dec
from ..proxy import (
    _connect_via_socks5,
    is_onion_endpoint,
    urlopen_with_proxy,
)
from ..redaction import redact_operational_text
from ..time_utils import UNKNOWN_OCCURRED_AT, parse_iso_datetime_or_none, timestamp_to_iso
from ..time_utils import iso_to_unix
from ..transfers import canonical_txid
from ..util import normalize_chain_value, normalize_network_value, parse_bool, parse_int
from ..wallet_descriptors import (
    SCRIPT_TYPE_BRANCH_BASE,
    branch_descriptor,
    branch_limits,
    decode_liquid_transaction,
    derive_descriptor_target,
    derive_descriptor_targets,
    liquid_asset_code,
    liquid_blinding_secret,
    liquid_plan_can_unblind,
)
from . import htlc_parser
from . import silent_payments
from .address_scripts import address_to_scriptpubkey
from .onchain import (
    input_script,
    input_value_sats,
    normalized_script_hex,
    output_script,
    output_value_sats,
)
from .sync import WalletSyncState, emit_sync_progress, normalize_backend_kind
from .wallets import (
    load_wallet_descriptor_plan_from_config,
    normalize_addresses,
    wallet_policy_asset_id,
)


ELECTRUM_STORED_GRAPH_VERSION = 1
ELECTRUM_STORED_GRAPH_MARKER = "_kassiber_electrum_graph"


def _emit_backend_progress(phase: str, **payload):
    event = {"phase": phase, **payload}
    if "processed" not in event:
        processed = event.get("targets_checked", event.get("transactions_seen"))
        if processed is not None:
            event["processed"] = processed
    if "total" not in event:
        total = event.get("target_count", event.get("transactions_total"))
        if total is not None:
            event["total"] = total
    emit_sync_progress(event)


def _emit_http_backoff(retry_number, max_retries, wait_seconds):
    """Surface a 429/503 backoff wait as sync progress so it does not look like a
    hang. Routed through the per-wallet progress emitter (when one is active in
    this context), so the event carries the wallet label like other phases."""
    _emit_backend_progress(
        "rate_limited",
        retry_attempt=int(retry_number),
        retry_max=int(max_retries),
        wait_seconds=round(float(wait_seconds), 2),
    )


def http_get_json(
    url,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
    _sleeper=None,
    _rng=None,
    _max_attempts=None,
):
    def _opener():
        request_headers = {
            "Accept": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
        }
        request_headers.update(headers or {})
        request = urlrequest.Request(url, headers=request_headers)
        with urlopen_with_proxy(
            request,
            url,
            timeout,
            proxy_url=proxy_url,
            source_label="backend",
        ) as response:
            return json.loads(response.read().decode("utf-8"))

    return http_client.request_with_retry(
        url,
        _opener,
        sleeper=_sleeper,
        rng=_rng,
        max_attempts=_max_attempts,
        on_retry=_emit_http_backoff,
    )


def http_get_text(
    url,
    timeout=30,
    accept="text/plain",
    *,
    headers=None,
    proxy_url=None,
    _sleeper=None,
    _rng=None,
    _max_attempts=None,
):
    def _opener():
        request_headers = {
            "Accept": accept,
            "User-Agent": f"{APP_NAME}/{__version__}",
        }
        request_headers.update(headers or {})
        request = urlrequest.Request(url, headers=request_headers)
        with urlopen_with_proxy(
            request,
            url,
            timeout,
            proxy_url=proxy_url,
            source_label="backend",
        ) as response:
            return response.read().decode("utf-8")

    return http_client.request_with_retry(
        url,
        _opener,
        sleeper=_sleeper,
        rng=_rng,
        max_attempts=_max_attempts,
        on_retry=_emit_http_backoff,
    )


def http_post_json(
    url,
    payload,
    headers=None,
    timeout=30,
    *,
    proxy_url=None,
    _sleeper=None,
    _rng=None,
    _max_attempts=None,
):
    def _opener():
        request = urlrequest.Request(
            url,
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "User-Agent": f"{APP_NAME}/{__version__}",
                **(headers or {}),
            },
            method="POST",
        )
        with urlopen_with_proxy(
            request,
            url,
            timeout,
            proxy_url=proxy_url,
            source_label="backend",
        ) as response:
            return json.loads(response.read().decode("utf-8"))

    return http_client.request_with_retry(
        url,
        _opener,
        sleeper=_sleeper,
        rng=_rng,
        max_attempts=_max_attempts,
        on_retry=_emit_http_backoff,
    )


def _backend_proxy_url(backend):
    return backend_value(backend, "tor_proxy", "proxy")


def append_url_path(base_url, extra_path):
    parts = urlparse.urlsplit(base_url)
    path = (parts.path or "").rstrip("/")
    full_path = f"{path}/{extra_path.lstrip('/')}" if extra_path else (path or "/")
    return urlparse.urlunsplit((parts.scheme, parts.netloc, full_path, parts.query, parts.fragment))


def parse_socket_backend_url(url, default_scheme="ssl", default_ports=None):
    default_ports = default_ports or {}
    parsed = urlparse.urlsplit(url if "://" in url else f"{default_scheme}://{url}")
    scheme = (parsed.scheme or default_scheme).lower()
    host = parsed.hostname
    port = parsed.port or default_ports.get(scheme)
    if not host or not port:
        raise AppError(f"Invalid backend socket URL: {url}")
    return scheme, host, port


def _connect_backend_socket(backend, host, port):
    timeout = backend_timeout(backend)
    proxy = backend_value(backend, "tor_proxy", "proxy")
    get_egress_ledger().record(
        subsystem="sync",
        host=host,
        port=port,
        scheme="electrum",
        operation="socket.connect",
        via_proxy=bool(proxy),
    )
    if proxy:
        return _connect_via_socks5(proxy, host, port, timeout)
    if is_onion_endpoint(host):
        raise AppError(
            ".onion backend URLs require a Tor/SOCKS proxy",
            code="network_proxy_required",
            hint=(
                "Configure --tor-proxy for this backend; Kassiber will not "
                "connect to .onion hosts directly."
            ),
        )
    return socket.create_connection((host, port), timeout=timeout)


def sanitize_wallet_segment(value):
    text = str(value).strip().lower()
    cleaned = []
    for char in text:
        if char.isalnum() or char in {"-", "_", "."}:
            cleaned.append(char)
        else:
            cleaned.append("-")
    sanitized = "".join(cleaned).strip("-")
    return sanitized or APP_NAME


class ElectrumClient:
    def __init__(self, backend):
        self.backend = backend
        self.socket = None
        self.reader = None
        self.request_id = 0
        self.server_version = None
        self._egress_host = None
        self._egress_port = None
        self._io_lock = threading.RLock()

    def __enter__(self):
        with self._io_lock:
            scheme, host, port = parse_socket_backend_url(
                self.backend["url"],
                default_scheme="ssl",
                default_ports={"ssl": 50002, "tcp": 50001},
            )
            raw_socket = _connect_backend_socket(self.backend, host, port)
            self._egress_host = host
            self._egress_port = port
            if scheme in {"ssl", "tls"}:
                certificate = backend_value(self.backend, "certificate")
                context = ssl.create_default_context(cafile=certificate)
                if parse_bool(backend_value(self.backend, "insecure"), default=False):
                    context.check_hostname = False
                    context.verify_mode = ssl.CERT_NONE
                raw_socket = context.wrap_socket(raw_socket, server_hostname=host)
            elif scheme != "tcp":
                raise AppError(f"Unsupported Electrum transport '{scheme}'")
            self.socket = raw_socket
            self.reader = raw_socket.makefile("r", encoding="utf-8", newline="\n")
            self.server_version = self.call("server.version", ["Kassiber", "1.6"])
            return self

    def __exit__(self, exc_type, exc, tb):
        with self._io_lock:
            if self.reader is not None:
                self.reader.close()
            if self.socket is not None:
                self.socket.close()
            self.reader = None
            self.socket = None
            self.server_version = None
            return False

    def _decode_message(self, line):
        try:
            message = json.loads(line)
        except json.JSONDecodeError as exc:
            preview = line.strip().replace("\n", "\\n")
            if len(preview) > 120:
                preview = preview[:117] + "..."
            raise AppError(
                f"Backend '{self.backend['name']}' did not respond with Electrum-format JSON",
                hint="Check that the backend URL points to an Electrum server and uses the correct tcp/ssl port.",
                details={"response_preview": preview},
                retryable=True,
            ) from exc
        if not isinstance(message, dict):
            raise AppError(
                f"Backend '{self.backend['name']}' did not respond with Electrum-format JSON",
                hint="Check that the backend URL points to an Electrum server and uses the correct tcp/ssl port.",
                details={"response_type": type(message).__name__},
                retryable=True,
            )
        return message

    def call(self, method, params=None):
        with self._io_lock:
            return self._call_locked(method, params)

    def _call_locked(self, method, params=None):
        if self.socket is None or self.reader is None:
            raise AppError("Electrum client is not connected")
        self.request_id += 1
        payload = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": self.request_id,
                "method": method,
                "params": params or [],
            }
        ).encode("utf-8") + b"\n"
        self.socket.sendall(payload)
        get_egress_ledger().record(
            subsystem="sync",
            host=self._egress_host,
            port=self._egress_port,
            scheme="electrum",
            operation="socket.write",
            method=method,
            bytes_out=len(payload),
        )
        while True:
            line = self.reader.readline()
            if not line:
                raise AppError(f"Electrum backend '{self.backend['name']}' closed the connection")
            message = self._decode_message(line)
            if message.get("id") != self.request_id:
                continue
            if message.get("error"):
                error = message["error"]
                if isinstance(error, dict):
                    detail = f"({error.get('code', 'unknown')}): {error.get('message', error)}"
                else:
                    detail = str(error)
                # The Electrum server message is untrusted free text that can
                # echo a txid/amount; pseudonymize at the source.
                raise AppError(
                    f"Electrum call {method} failed {redact_operational_text(detail)}",
                    code="electrum_rpc_error",
                )
            return message.get("result")

    def batch_call(self, requests):
        with self._io_lock:
            return self._batch_call_locked(requests)

    def _batch_call_locked(self, requests):
        if self.socket is None or self.reader is None:
            raise AppError("Electrum client is not connected")
        if not requests:
            return []
        payload_lines = []
        pending = {}
        for index, (method, params) in enumerate(requests):
            self.request_id += 1
            pending[self.request_id] = (index, method)
            payload_lines.append(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": self.request_id,
                        "method": method,
                        "params": params or [],
                    }
                )
            )
        payload = ("\n".join(payload_lines) + "\n").encode("utf-8")
        self.socket.sendall(payload)
        get_egress_ledger().record(
            subsystem="sync",
            host=self._egress_host,
            port=self._egress_port,
            scheme="electrum",
            operation="socket.write",
            method="batch",
            bytes_out=len(payload),
        )
        results = [None] * len(requests)
        remaining = len(requests)
        while remaining:
            line = self.reader.readline()
            if not line:
                raise AppError(f"Electrum backend '{self.backend['name']}' closed the connection")
            message = self._decode_message(line)
            response_id = message.get("id")
            if response_id not in pending:
                continue
            index, method = pending.pop(response_id)
            if message.get("error"):
                error = message["error"]
                if isinstance(error, dict):
                    detail = f"({error.get('code', 'unknown')}): {error.get('message', error)}"
                else:
                    detail = str(error)
                # The Electrum server message is untrusted free text that can
                # echo a txid/amount; pseudonymize at the source.
                raise AppError(
                    f"Electrum call {method} failed {redact_operational_text(detail)}",
                    code="electrum_rpc_error",
                )
            results[index] = message.get("result")
            remaining -= 1
        return results


def _electrum_pool_key(backend):
    return (
        str(_mapping_get(backend, "name", "") or ""),
        str(_mapping_get(backend, "url", "") or ""),
        str(backend_value(backend, "tor_proxy") or ""),
        str(backend_value(backend, "certificate") or ""),
        str(backend_value(backend, "insecure") or ""),
    )


class _ElectrumClientPool:
    def __init__(self):
        self._dispatchers = {}
        self._lock = threading.Lock()

    def client(self, backend):
        key = _electrum_pool_key(backend)
        with self._lock:
            dispatcher = self._dispatchers.get(key)
            if dispatcher is None:
                dispatcher = _ElectrumBatchDispatcher(backend)
                self._dispatchers[key] = dispatcher
            return dispatcher

    def close(self):
        with self._lock:
            dispatchers = list(self._dispatchers.values())
            self._dispatchers.clear()
        for dispatcher in reversed(dispatchers):
            dispatcher.close()


class _ElectrumBatchDispatcher:
    _COALESCE_SECONDS = 0.002

    def __init__(self, backend):
        self.backend = backend
        self.batch_size = backend_batch_size(backend)
        self._queue = queue.Queue()
        self._closed = False
        self._state_lock = threading.Lock()
        self._thread = threading.Thread(
            target=self._run,
            name="kassiber-electrum-batch",
            daemon=True,
        )
        self._thread.start()

    def call(self, method, params=None):
        return self.batch_call([(method, params or [])])[0]

    def batch_call(self, requests):
        requests = list(requests)
        if not requests:
            return []
        pending = {
            "requests": requests,
            "event": threading.Event(),
            "result": None,
            "error": None,
        }
        with self._state_lock:
            if self._closed:
                raise AppError("Electrum client pool is closed")
            self._queue.put(pending)
        pending["event"].wait()
        if pending["error"] is not None:
            raise pending["error"]
        return pending["result"]

    def close(self):
        with self._state_lock:
            if self._closed:
                return
            self._closed = True
            self._queue.put(None)
        self._thread.join()

    def _run(self):
        client = None
        stop_after_batch = False

        def discard_client():
            nonlocal client
            current = client
            client = None
            if current is not None:
                try:
                    current.__exit__(None, None, None)
                except Exception:
                    pass

        def execute_requests(requests):
            nonlocal client
            if client is None:
                candidate = ElectrumClient(self.backend)
                candidate.__enter__()
                client = candidate
            results = []
            for start in range(0, len(requests), self.batch_size):
                results.extend(
                    client.batch_call(requests[start : start + self.batch_size])
                )
            return results

        try:
            while True:
                first = self._queue.get()
                if first is None:
                    break
                pending_calls = [first]
                deadline = time.monotonic() + self._COALESCE_SECONDS
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    try:
                        item = self._queue.get(timeout=remaining)
                    except queue.Empty:
                        break
                    if item is None:
                        stop_after_batch = True
                        break
                    pending_calls.append(item)
                try:
                    combined = [
                        request
                        for pending in pending_calls
                        for request in pending["requests"]
                    ]
                    try:
                        combined_results = execute_requests(combined)
                    except Exception as combined_error:
                        discard_client()
                        can_isolate_rpc_error = (
                            len(pending_calls) > 1
                            and isinstance(combined_error, AppError)
                            and combined_error.code == "electrum_rpc_error"
                        )
                        if can_isolate_rpc_error:
                            # Read-only Electrum calls are safe to retry. Split a
                            # request-level RPC error back into its logical callers
                            # so one rejected request cannot contaminate another
                            # wallet. Transport/session failures affect the whole
                            # connection and must not multiply timeouts per caller.
                            for pending in pending_calls:
                                try:
                                    pending["result"] = execute_requests(
                                        pending["requests"]
                                    )
                                except Exception as exc:
                                    pending["error"] = exc
                                    discard_client()
                        else:
                            for pending in pending_calls:
                                pending["error"] = combined_error
                    else:
                        offset = 0
                        for pending in pending_calls:
                            count = len(pending["requests"])
                            pending["result"] = combined_results[offset : offset + count]
                            offset += count
                except BaseException as exc:
                    discard_client()
                    for pending in pending_calls:
                        if pending["error"] is None and pending["result"] is None:
                            pending["error"] = exc
                finally:
                    for pending in pending_calls:
                        pending["event"].set()
                if stop_after_batch:
                    break
        finally:
            discard_client()


_active_electrum_client_pool = ContextVar("active_electrum_client_pool", default=None)


@contextmanager
def shared_electrum_client_pool():
    active = _active_electrum_client_pool.get()
    if active is not None:
        yield active
        return
    pool = _ElectrumClientPool()
    token = _active_electrum_client_pool.set(pool)
    try:
        yield pool
    finally:
        _active_electrum_client_pool.reset(token)
        pool.close()


def _electrum_client_context(backend):
    pool = _active_electrum_client_pool.get()
    return nullcontext(pool.client(backend)) if pool is not None else ElectrumClient(backend)


def batched(items, batch_size):
    for start in range(0, len(items), batch_size):
        yield items[start : start + batch_size]


def _bounded_http_workers(backend):
    return max(1, min(8, backend_batch_size(backend)))


def _map_bounded(items, worker, max_workers, on_result=None):
    items = list(items)
    if not items:
        return []
    if max_workers <= 1 or len(items) == 1:
        results = []
        for index, item in enumerate(items, start=1):
            result = worker(item)
            results.append(result)
            if on_result is not None:
                on_result(index, result, len(items))
        return results
    # Each worker runs inside a copy of the submit-time context so the per-wallet
    # ``sync_progress_emitter`` ContextVar (set on the calling thread) reaches the
    # worker — otherwise a 429/503 backoff inside a worker would emit no progress
    # and look like a hang. Iterating futures in submission order preserves input
    # order and surfaces the first worker exception at its position, matching the
    # previous ``executor.map`` semantics. ``on_result`` still runs on this thread.
    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = [executor.submit(copy_context().run, worker, item) for item in items]
        results = []
        for index, future in enumerate(futures, start=1):
            result = future.result()
            results.append(result)
            if on_result is not None:
                on_result(index, result, len(items))
        return results


def electrum_call_many(client, requests, batch_size):
    if not requests:
        return []
    normalized_batch_size = max(1, int(batch_size))
    batch_call = getattr(client, "batch_call", None)
    results = []
    for chunk in batched(requests, normalized_batch_size):
        if callable(batch_call):
            results.extend(batch_call(chunk))
            continue
        for method, params in chunk:
            results.append(client.call(method, params))
    return results


def sync_target_from_address(address, chain, network, address_index):
    return {
        "chain": chain,
        "network": network,
        "branch_index": 0,
        "branch_label": "address",
        "address_index": address_index,
        "address": address,
        "unconfidential_address": None,
        "script_pubkey": address_to_scriptpubkey(address).hex(),
    }


def sync_target_from_derived(target):
    return {
        "chain": target.chain,
        "network": target.network,
        "branch_index": target.branch_index,
        "branch_label": target.branch_label,
        "address_index": target.address_index,
        "address": target.address,
        "unconfidential_address": target.unconfidential_address,
        "script_pubkey": target.script_pubkey,
        "derivation_path": target.derivation_path,
        "derivation_paths": list(target.derivation_paths),
        "key_origins": list(target.key_origins),
    }


def scriptpubkey_scripthash(script_pubkey_hex):
    return hashlib.sha256(bytes.fromhex(script_pubkey_hex)).digest()[::-1].hex()


def _mapping_get(mapping, key, default=None):
    try:
        return mapping[key]
    except (KeyError, IndexError, TypeError):
        getter = getattr(mapping, "get", None)
        if callable(getter):
            return getter(key, default)
        return default


def validate_backend_for_wallet(backend, chain, network, has_descriptor=False):
    kind = normalize_backend_kind(backend["kind"])
    backend_chain = backend_value(backend, "chain")
    if backend_chain:
        expected_chain = normalize_chain_value(backend_chain)
        if expected_chain != chain:
            raise AppError(
                f"Backend '{backend['name']}' is configured for {expected_chain}, "
                f"but source refresh requires {chain}"
            )
    backend_network = backend_value(backend, "network")
    if backend_network:
        expected_network = normalize_network_value(chain, backend_network)
        if expected_network != network:
            raise AppError(
                f"Backend '{backend['name']}' is configured for {expected_network}, "
                f"but source refresh requires {network}"
            )
    if chain == "liquid" and kind not in {"esplora", "electrum"}:
        raise AppError("Liquid live refresh currently requires an Esplora-compatible or Electrum backend")
    if chain != "bitcoin" and kind == "bitcoinrpc":
        raise AppError(f"Backend kind '{kind}' does not support {chain} wallets")
    return kind


def _highest_used_branch_index(highest_used, branch_index):
    if not isinstance(highest_used, dict):
        return None
    value = highest_used.get(str(branch_index), highest_used.get(branch_index))
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def scan_compatibility_descriptor_targets(
    plan,
    *,
    target_used_batch,
    scan_batch_size=100,
    highest_used=None,
):
    """Discover descriptor history for an explicit compatibility route.

    BDK/LWK own ordinary descriptor discovery. Compatibility observers still
    need their legacy backend-backed gap scan after source-overlap preflight,
    because the finite local target set used by that preflight is not a safe
    first-refresh discovery boundary.
    """

    limits = branch_limits(plan)
    targets = []
    targets_checked = 0

    def emit_discovery_progress(branch_index, branch_gap_limit, consecutive_unused):
        _emit_backend_progress(
            "discovery",
            targets_checked=targets_checked,
            retained_targets=len(targets),
            branch_index=branch_index,
            gap_limit=branch_gap_limit,
            unused_streak=consecutive_unused,
        )

    for branch in plan.branches:
        branch_gap_limit = limits.get(branch.branch_index, plan.gap_limit)
        if branch_gap_limit <= 1:
            targets.append(
                sync_target_from_derived(
                    derive_descriptor_target(plan, branch.branch_index, 0)
                )
            )
            continue
        consecutive_unused = 0
        address_index = 0
        known_highest = _highest_used_branch_index(highest_used, branch.branch_index)
        if known_highest is not None:
            targets.extend(
                sync_target_from_derived(target)
                for target in derive_descriptor_targets(
                    plan,
                    branch_index=branch.branch_index,
                    start=0,
                    end=known_highest + 1,
                )
            )
            address_index = known_highest + 1
        while consecutive_unused < branch_gap_limit:
            batch_targets = [
                sync_target_from_derived(target)
                for target in derive_descriptor_targets(
                    plan,
                    branch_index=branch.branch_index,
                    start=address_index,
                    end=address_index + scan_batch_size,
                )
            ]
            if not batch_targets:
                break
            used_batch = list(target_used_batch(batch_targets))
            if len(used_batch) != len(batch_targets):
                raise AppError(
                    "Compatibility descriptor discovery returned an unexpected number of usage checks"
                )
            targets_checked += len(batch_targets)
            for target, is_used in zip(batch_targets, used_batch):
                targets.append(target)
                if is_used:
                    consecutive_unused = 0
                else:
                    consecutive_unused += 1
                address_index += 1
                if consecutive_unused >= branch_gap_limit:
                    break
            emit_discovery_progress(
                branch.branch_index,
                branch_gap_limit,
                consecutive_unused,
            )
    return targets


def _esplora_auth_headers(backend):
    auth_header = str(backend_value(backend, "auth_header") or "").strip()
    if auth_header:
        return {"Authorization": auth_header}
    token = str(backend_value(backend, "token") or "").strip()
    if token:
        return {"Authorization": f"Bearer {token}"}
    return None


def _esplora_call_kwargs(*, timeout, headers=None, proxy_url=None, **extra):
    """Keep unauthenticated compatibility calls on their historical shape."""
    kwargs = {"timeout": timeout, "proxy_url": proxy_url, **extra}
    if headers is not None:
        kwargs["headers"] = headers
    return kwargs


def esplora_scripthash_stats(
    base_url,
    script_pubkey_hex,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    resource = append_url_path(base_url, f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}")
    return http_get_json(
        resource,
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    )


def esplora_stats_fingerprint(payload):
    chain_stats = payload.get("chain_stats") or {}
    mempool_stats = payload.get("mempool_stats") or {}
    return hashlib.sha256(
        json.dumps(
            {
                "chain": {
                    "funded_txo_count": chain_stats.get("funded_txo_count"),
                    "funded_txo_sum": chain_stats.get("funded_txo_sum"),
                    "spent_txo_count": chain_stats.get("spent_txo_count"),
                    "spent_txo_sum": chain_stats.get("spent_txo_sum"),
                    "tx_count": chain_stats.get("tx_count"),
                },
                "mempool": {
                    "funded_txo_count": mempool_stats.get("funded_txo_count"),
                    "funded_txo_sum": mempool_stats.get("funded_txo_sum"),
                    "spent_txo_count": mempool_stats.get("spent_txo_count"),
                    "spent_txo_sum": mempool_stats.get("spent_txo_sum"),
                    "tx_count": mempool_stats.get("tx_count"),
                },
            },
            sort_keys=True,
        ).encode("utf-8")
    ).hexdigest()


def esplora_scripthash_has_history(
    base_url,
    script_pubkey_hex,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    payload = esplora_scripthash_stats(
        base_url,
        script_pubkey_hex,
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    )
    chain_stats = payload.get("chain_stats") or {}
    mempool_stats = payload.get("mempool_stats") or {}
    return int(chain_stats.get("tx_count") or 0) + int(mempool_stats.get("tx_count") or 0) > 0


def _probe_scripts_have_history(backend, kind, script_pubkeys, *, timeout):
    if kind == "esplora":
        workers = _bounded_http_workers(backend)
        headers = _esplora_auth_headers(backend)
        proxy_url = _backend_proxy_url(backend)

        def probe(script_pubkey):
            return esplora_scripthash_has_history(
                backend["url"],
                script_pubkey,
                **_esplora_call_kwargs(
                    timeout=timeout,
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            )

        return _map_bounded(script_pubkeys, probe, workers)
    if kind == "electrum":
        scripthashes = [scriptpubkey_scripthash(spk) for spk in script_pubkeys]
        with _electrum_client_context(backend) as client:
            statuses = electrum_call_many(
                client,
                [("blockchain.scripthash.subscribe", [scripthash]) for scripthash in scripthashes],
                batch_size=backend_batch_size(backend),
            )
        return [status is not None for status in statuses]
    raise AppError(f"Script-type detection is not implemented for backend kind '{kind}'")


def detect_active_script_types(backend, xpub, *, chain="bitcoin", network=None, timeout=None):
    """Report which candidate script types a bare xpub has on-chain history for.

    Probes only receive index 0 of each of the four script types -- four history
    checks against one host, bounded by the backend worker cap. There is
    deliberately no gap-window scan here: detection must stay rate-limit-safe.
    """
    chain = normalize_chain_value(chain)
    network = normalize_network_value(chain, network)
    kind = validate_backend_for_wallet(backend, chain, network, has_descriptor=True)
    if timeout is None:
        timeout = backend_timeout(backend)
    candidates = list(SCRIPT_TYPE_BRANCH_BASE)
    script_pubkeys = []
    for script_type in candidates:
        plan = load_wallet_descriptor_plan_from_config(
            {
                "xpub": xpub,
                "script_types": [script_type],
                "chain": chain,
                "network": network,
                "gap_limit": 1,
            }
        )
        base = SCRIPT_TYPE_BRANCH_BASE[script_type]
        script_pubkeys.append(derive_descriptor_target(plan, base, 0).script_pubkey)
    history = _probe_scripts_have_history(backend, kind, script_pubkeys, timeout=timeout)
    return [
        {"script_type": script_type, "has_history": bool(used)}
        for script_type, used in zip(candidates, history)
    ]


def discover_compatibility_descriptor_targets(backend, plan, kind, checkpoint=None):
    """Run backend-backed gap discovery for a named compatibility observer."""

    timeout = backend_timeout(backend)
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    if kind == "esplora":
        scan_batch_size = _bounded_http_workers(backend)
        headers = _esplora_auth_headers(backend)
        proxy_url = _backend_proxy_url(backend)
        cached_stats = checkpoint.get("esplora_scripthashes") or {}

        def target_used(target):
            scripthash = scriptpubkey_scripthash(target["script_pubkey"])
            cached = cached_stats.get(scripthash)
            if isinstance(cached, dict) and int(cached.get("tx_count") or 0) > 0:
                return True
            return esplora_scripthash_has_history(
                backend["url"],
                target["script_pubkey"],
                **_esplora_call_kwargs(
                    timeout=timeout,
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            )

        return {
            "targets": scan_compatibility_descriptor_targets(
                plan,
                target_used_batch=lambda targets: _map_bounded(
                    targets,
                    target_used,
                    scan_batch_size,
                ),
                scan_batch_size=scan_batch_size,
                highest_used=checkpoint.get("highest_used"),
            ),
            "history_cache": {},
        }
    if kind == "electrum":
        electrum_batch_size = backend_batch_size(backend)
        cached_statuses = dict(checkpoint.get("electrum_scripthash_statuses") or {})
        with _electrum_client_context(backend) as client:

            def target_used_batch(targets):
                scripthashes = [
                    scriptpubkey_scripthash(target["script_pubkey"])
                    for target in targets
                ]
                missing = [
                    scripthash
                    for scripthash in scripthashes
                    if cached_statuses.get(scripthash) is None
                ]
                if missing:
                    statuses = electrum_call_many(
                        client,
                        [
                            ("blockchain.scripthash.subscribe", [scripthash])
                            for scripthash in missing
                        ],
                        batch_size=electrum_batch_size,
                    )
                    for scripthash, status in zip(missing, statuses):
                        cached_statuses[scripthash] = status
                return [
                    cached_statuses.get(scripthash) is not None
                    for scripthash in scripthashes
                ]

            return {
                "targets": scan_compatibility_descriptor_targets(
                    plan,
                    target_used_batch=target_used_batch,
                    scan_batch_size=electrum_batch_size,
                    highest_used=checkpoint.get("highest_used"),
                ),
                "history_cache": {},
            }
    raise AppError(
        f"Compatibility descriptor discovery is not implemented for backend kind '{kind}'"
    )


def discover_bitcoinrpc_descriptor_targets(plan, checkpoint=None):
    """Resolve Core's local import range without backend access."""

    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    return {
        "targets": _bitcoinrpc_descriptor_targets_for_checkpoint(plan, checkpoint),
        "history_cache": {},
    }


def _offline_descriptor_targets(plan, checkpoint=None):
    """Resolve a finite overlap horizon without contacting a backend."""

    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    highest_used = checkpoint.get("highest_used") or {}
    targets = []
    for branch in plan.branches:
        try:
            observed = int(highest_used.get(str(branch.branch_index), highest_used.get(branch.branch_index, -1)))
        except (TypeError, ValueError):
            observed = -1
        end = max(plan.gap_limit, observed + plan.gap_limit + 1)
        targets.extend(
            derive_descriptor_targets(
                plan,
                branch_index=branch.branch_index,
                start=0,
                end=end,
            )
        )
    return [target.__dict__ if hasattr(target, "__dict__") else dict(target) for target in targets]


def resolve_wallet_sync_targets(backend, wallet):
    config = json.loads(wallet["config_json"] or "{}")
    checkpoint = _mapping_get(wallet, "_freshness_checkpoint", {}) or {}
    stored_history_cache = _mapping_get(wallet, "_history_cache", {}) or {}
    if silent_payments.has_silent_payment_sync_material(config):
        plan = silent_payments.build_plan(config)
        kind = validate_backend_for_wallet(backend, plan.chain, plan.network, has_descriptor=False)
        silent_payments.validate_backend_capability(backend, plan, kind=kind)
        targets = [silent_payments.sync_target(plan)]
        return WalletSyncState(
            chain=plan.chain,
            network=plan.network,
            descriptor_plan=plan,
            policy_asset_id="",
            targets=targets,
            tracked_scripts={},
            history_cache={},
            checkpoint=checkpoint,
        )
    descriptor_plan = (
        load_wallet_descriptor_plan_from_config(config)
        if (config.get("descriptor") or config.get("xpub"))
        else None
    )
    history_cache = dict(stored_history_cache)
    if descriptor_plan:
        chain = descriptor_plan.chain
        network = descriptor_plan.network
        if chain == "liquid" and not liquid_plan_can_unblind(descriptor_plan):
            raise AppError("Liquid descriptor wallets require private blinding keys for full sync and fee accounting")
        if chain == "liquid" and not config.get("backend"):
            raise AppError("Liquid wallets must name a backend explicitly; no public Liquid default is built in")
        kind = validate_backend_for_wallet(backend, chain, network, has_descriptor=True)
        # Discovery must stay local so source-overlap checks run before either
        # BDK or an explicit compatibility adapter opens a network connection.
        if kind in {"esplora", "electrum"}:
            discovery = {
                "targets": _offline_descriptor_targets(descriptor_plan, checkpoint),
                "history_cache": {},
            }
        else:
            discovery = discover_bitcoinrpc_descriptor_targets(descriptor_plan, checkpoint)
        targets = discovery["targets"]
        history_cache.update(discovery.get("history_cache") or {})
    else:
        addresses = normalize_addresses(config.get("addresses"))
        if not addresses:
            return WalletSyncState(
                chain="",
                network="",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[],
                tracked_scripts={},
                history_cache={},
            )
        chain = normalize_chain_value(config.get("chain"))
        network = normalize_network_value(chain, config.get("network"))
        if chain == "liquid":
            raise AppError("Liquid live refresh currently requires descriptor-backed wallets so outputs can be unblinded locally")
        validate_backend_for_wallet(backend, chain, network, has_descriptor=False)
        targets = [sync_target_from_address(address, chain, network, index) for index, address in enumerate(addresses)]
    tracked_scripts = {
        target["script_pubkey"]: target
        for target in targets
        if target.get("script_pubkey")
    }
    return WalletSyncState(
        chain=chain,
        network=network,
        descriptor_plan=descriptor_plan,
        policy_asset_id=wallet_policy_asset_id(config, chain, network),
        targets=targets,
        tracked_scripts=tracked_scripts,
        history_cache=history_cache,
        checkpoint=checkpoint,
    )


def _observer_discovered_targets(observer_updates):
    """Collect owned scripts discovered beyond the finite preflight horizon."""

    targets = {}

    def add_target(value):
        target = dict(value) if isinstance(value, dict) else {"script_pubkey": value}
        script = str(
            target.get("script_pubkey") or target.get("scriptpubkey") or ""
        ).strip().lower()
        if not script:
            return
        try:
            bytes.fromhex(script)
        except ValueError:
            return
        target["script_pubkey"] = script
        targets.setdefault(script, target)

    for prepared in observer_updates:
        update = getattr(prepared, "update", None)
        facts = update.get("facts") if isinstance(update, dict) else None
        if not isinstance(facts, dict):
            continue
        for output in facts.get("outputs") or []:
            add_target(output)
        for record in facts.get("transaction_records") or []:
            if not isinstance(record, dict):
                continue
            raw = record.get("raw_json")
            try:
                raw = json.loads(raw) if isinstance(raw, str) else dict(raw or {})
            except (TypeError, ValueError, json.JSONDecodeError):
                continue
            for script in raw.get("observer_owned_scripts") or []:
                add_target(script)
            for entry in [*(raw.get("vin") or []), *(raw.get("vout") or [])]:
                if not isinstance(entry, dict):
                    continue
                owned = (
                    entry.get("prevout")
                    if isinstance(entry.get("prevout"), dict)
                    else entry
                )
                if isinstance(owned, dict) and owned.get("role") == "owned":
                    add_target(owned)
    return list(targets.values())


def prepare_dependency_observer_fetch(conn, profile, wallet, discovery):
    """Prepare supported Bitcoin/Liquid descriptor refreshes through dependencies."""

    from .chain_observer import (
        ObserverPrepareRequest,
        discard_prepared_observer_updates,
        prepare_observer_update,
    )
    from .chain_observer.bdk import (
        BdkObserver,
        bdk_branches_for_identity,
        bdk_compatibility_reason,
    )
    from .chain_observer.identity import identities_for_wallet
    from .chain_observer.lwk import LwkObserver, lwk_compatibility_reason
    from .chain_observer.store import load_observer_values
    from .sync import WalletBackendFetch, _overlap_filtered_skip_outcome

    state = discovery.sync_state
    if discovery.skip_outcome is not None or state is None:
        return None

    def compatibility_fetch(reason):
        if state.chain not in {"bitcoin", "liquid"} or discovery.kind not in {"esplora", "electrum"}:
            return None
        compatibility_state = state
        descriptor_plan = state.descriptor_plan
        if (
            descriptor_plan is not None
            and getattr(descriptor_plan, "kind", None) != "silent-payment"
        ):
            online_discovery = discover_compatibility_descriptor_targets(
                discovery.backend,
                descriptor_plan,
                discovery.kind,
                checkpoint=state.checkpoint,
            )
            online_targets = list(online_discovery["targets"])
            compatibility_state = replace(
                state,
                targets=online_targets,
                tracked_scripts={
                    target["script_pubkey"]: target
                    for target in online_targets
                    if target.get("script_pubkey")
                },
                history_cache={
                    **dict(state.history_cache),
                    **(online_discovery.get("history_cache") or {}),
                },
            )
            # The finite local horizon was filtered before any connection.
            # Reapply ownership to every online result so deeper targets cannot
            # introduce an overlap that did not exist inside that horizon.
            from . import source_overlap

            compatibility_state = source_overlap.filter_sync_state_for_canonical_owner(
                conn,
                profile,
                wallet,
                compatibility_state,
            )
            if not compatibility_state.targets:
                return WalletBackendFetch(
                    backend=discovery.backend,
                    sync_state=None,
                    normalized_records=(),
                    adapter_meta={},
                    kind=discovery.kind,
                    started=discovery.started,
                    force_full=discovery.force_full,
                    skip_outcome=_overlap_filtered_skip_outcome(
                        wallet,
                        started=discovery.started,
                        force_full=discovery.force_full,
                        original_target_count=len(online_targets),
                    ),
                )
        adapter = COMPATIBILITY_SYNC_BACKEND_ADAPTERS[discovery.kind]
        try:
            records, meta = adapter(discovery.backend, wallet, compatibility_state)
        except AppError:
            raise
        except Exception as exc:
            safe_error = redact_operational_text(str(exc))
            raise AppError(
                f"{state.chain.title()} compatibility observation failed for backend '{discovery.backend.get('name')}'",
                code="backend_sync_failed",
                details={"observer_route": "compatibility", "error": safe_error},
                retryable=True,
            ) from exc
        route = (
            "silent_payments"
            if reason == "silent_payment"
            else "bitcoin_script"
            if state.chain == "bitcoin" and reason == "address_list"
            else "compatibility"
        )
        normalized_records = tuple(records)
        authoritative = reason != "silent_payment" or (
            (meta or {}).get("silent_payment_scan_complete") is True
            and all(
                record.get("confirmed_at") not in (None, "")
                for record in normalized_records
            )
        )
        return WalletBackendFetch(
            backend=discovery.backend,
            sync_state=compatibility_state,
            normalized_records=normalized_records,
            adapter_meta={
                **dict(meta or {}),
                "observer_route": route,
                "observer_compatibility_reason": reason,
            },
            kind=discovery.kind,
            started=discovery.started,
            force_full=discovery.force_full,
            authoritative_chain_observer=authoritative,
        )

    dependency_kind = "lwk" if state.chain == "liquid" else "bdk"
    compatibility_reason = (
        lwk_compatibility_reason(discovery.backend, state)
        if state.chain == "liquid"
        else bdk_compatibility_reason(discovery.backend, state)
    )
    if compatibility_reason is not None:
        return compatibility_fetch(compatibility_reason)
    expected_scripts = {
        target.get("script_pubkey")
        for target in _offline_descriptor_targets(state.descriptor_plan, state.checkpoint)
        if target.get("script_pubkey")
    }
    actual_scripts = {
        target.get("script_pubkey") for target in state.targets if target.get("script_pubkey")
    }
    if actual_scripts != expected_scripts:
        # BDK scans whole descriptors. A finite source-overlap exclusion cannot
        # safely be represented, so keep this named compatibility route.
        return compatibility_fetch("source_overlap_partial_descriptor")
    identities = identities_for_wallet(wallet, observer_kind=dependency_kind)
    if not identities:
        raise AppError(
            "The dependency observer route resolved no wallet identities",
            code="observer_identity_invalid",
            hint="Review the wallet descriptor source; Kassiber will not treat an empty scan as authoritative.",
            details={"observer": dependency_kind, "wallet_id": str(wallet["id"])},
            retryable=False,
        )
    prepared = []
    try:
        for identity in identities:
            if dependency_kind == "lwk":
                # A full refresh reconstructs the wollet in memory. Existing
                # encrypted values remain durable until successful apply, but
                # are never loaded into this request-local ForeignStore.
                stored_values = (
                    {}
                    if discovery.force_full
                    else load_observer_values(conn, identity)
                )
                observer = LwkObserver(
                    identity=identity,
                    backend=discovery.backend,
                    descriptor_plan=state.descriptor_plan,
                    policy_asset_id=state.policy_asset_id,
                    stored_values=stored_values,
                )
            else:
                observer = BdkObserver(
                    identity=identity,
                    backend=discovery.backend,
                    branches=bdk_branches_for_identity(state.descriptor_plan, identity),
                    gap_limit=state.descriptor_plan.gap_limit,
                )
            prepared.append(
                prepare_observer_update(
                    conn,
                    identity,
                    observer,
                    ObserverPrepareRequest(
                        backend_name=str(discovery.backend["name"]),
                        backend_kind=discovery.kind,
                        force_full=discovery.force_full,
                        checkpoint=dict(state.checkpoint or {}),
                    ),
                )
            )
        discovered_targets = _observer_discovered_targets(prepared)
        newly_discovered = [
            target
            for target in discovered_targets
            if target.get("script_pubkey") not in actual_scripts
        ]
        if newly_discovered:
            from . import source_overlap

            expanded_targets = {
                str(target.get("script_pubkey") or "").strip().lower(): target
                for target in state.targets
                if target.get("script_pubkey")
            }
            for target in newly_discovered:
                expanded_targets.setdefault(target["script_pubkey"], target)
            expanded_state = replace(
                state,
                targets=list(expanded_targets.values()),
                tracked_scripts=dict(expanded_targets),
            )
            filtered_state = source_overlap.filter_sync_state_for_canonical_owner(
                conn,
                profile,
                wallet,
                expanded_state,
            )
            filtered_scripts = {
                str(target.get("script_pubkey") or "").strip().lower()
                for target in filtered_state.targets
                if target.get("script_pubkey")
            }
            if filtered_scripts != set(expanded_targets):
                discard_prepared_observer_updates(prepared)
                fallback = compatibility_fetch("source_overlap_dependency_discovery")
                if fallback is None:
                    raise AppError(
                        "Dependency-discovered ownership overlap has no safe observer route",
                        code="source_overlap",
                        retryable=False,
                    )
                return fallback
    except BaseException:
        discard_prepared_observer_updates(prepared)
        raise
    return WalletBackendFetch(
        backend=discovery.backend,
        sync_state=state,
        normalized_records=(),
        adapter_meta={"observer_route": dependency_kind},
        kind=discovery.kind,
        started=discovery.started,
        force_full=discovery.force_full,
        observer_updates=tuple(prepared),
        authoritative_chain_observer=True,
    )


def fetch_esplora_history(
    base_url,
    resource_path,
    max_pages=None,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    transactions = []
    seen_txids = set()
    last_seen = None
    page_count = 0
    while True:
        if max_pages is not None and page_count >= max_pages:
            break
        chain_url = (
            append_url_path(base_url, f"{resource_path}/txs/chain/{last_seen}")
            if last_seen
            else append_url_path(base_url, f"{resource_path}/txs/chain")
        )
        page = http_get_json(
            chain_url,
            **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
        )
        if not page:
            break
        for tx in page:
            txid = tx.get("txid")
            if txid and txid not in seen_txids:
                seen_txids.add(txid)
                transactions.append(tx)
        last_seen = page[-1]["txid"]
        page_count += 1
        if len(page) < 25:
            break
    mempool_url = append_url_path(base_url, f"{resource_path}/txs/mempool")
    for tx in http_get_json(
        mempool_url,
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    ):
        txid = tx.get("txid")
        if txid and txid not in seen_txids:
            seen_txids.add(txid)
            transactions.append(tx)
    return transactions


def fetch_esplora_scripthash_transactions(
    base_url,
    script_pubkey_hex,
    max_pages=None,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    return fetch_esplora_history(
        base_url,
        f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}",
        **_esplora_call_kwargs(
            timeout=timeout,
            headers=headers,
            proxy_url=proxy_url,
            max_pages=max_pages,
        ),
    )


def fetch_esplora_scripthash_utxos(
    base_url,
    script_pubkey_hex,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    return http_get_json(
        append_url_path(
            base_url,
            f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}/utxo",
        ),
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    )


def fetch_esplora_transaction(
    base_url,
    txid,
    timeout=30,
    *,
    headers=None,
    proxy_url=None,
):
    """Fetch one transaction's JSON by txid.

    Esplora returns ``vin`` entries with inline ``prevout`` (including
    ``scriptpubkey``) plus ``vout`` entries with ``scriptpubkey`` for both
    Bitcoin and Liquid, so a single call yields every input and output script
    needed for ownership classification. Routed through the shared retry/backoff
    HTTP layer like all other backend reads.
    """
    return http_get_json(
        append_url_path(base_url, f"tx/{txid}"),
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    )


def _legs_from_esplora_tx(tx, chain=""):
    inputs = []
    for vin in tx.get("vin", []) if isinstance(tx, dict) else []:
        if not isinstance(vin, dict):
            continue
        prevout = vin.get("prevout") or {}
        outpoint = None
        if vin.get("txid") is not None and vin.get("vout") is not None:
            outpoint = f"{str(vin['txid']).lower()}:{int(vin['vout'])}"
        inputs.append({"outpoint": outpoint, "script": prevout.get("scriptpubkey")})
    outputs = []
    for index, vout in enumerate(tx.get("vout", []) if isinstance(tx, dict) else []):
        if not isinstance(vout, dict):
            continue
        outputs.append({"n": index, "script": vout.get("scriptpubkey")})
    return {"inputs": inputs, "outputs": outputs, "chain": chain, "source": "chain"}


def _legs_from_bitcoin_tx(parsed):
    inputs = [
        {
            "outpoint": (
                f"{str(vin['txid']).lower()}:{int(vin['vout'])}"
                if vin.get("txid") is not None and vin.get("vout") is not None
                else None
            ),
            "script": None,
        }
        for vin in parsed.get("vin", [])
    ]
    outputs = [{"n": vout.get("n"), "script": vout.get("script_hex")} for vout in parsed.get("vout", [])]
    return {"inputs": inputs, "outputs": outputs, "chain": "bitcoin", "source": "chain"}


def _legs_from_liquid_tx(decoded):
    inputs = []
    for vin in decoded.vin:
        prev_vout = getattr(vin, "vout", None)
        outpoint = f"{liquid_input_txid(vin)}:{int(prev_vout)}" if prev_vout is not None else None
        inputs.append({"outpoint": outpoint, "script": None})
    outputs = []
    for index, output in enumerate(decoded.vout):
        script_hex = output.script_pubkey.data.hex()
        outputs.append({"n": index, "script": script_hex or None})
    return {"inputs": inputs, "outputs": outputs, "chain": "liquid", "source": "chain"}


def fetch_transaction_legs(backend, txid, chain=None, *, client=None):
    """Fetch a transaction and reduce it to ownership-matching legs.

    Returns ``{"inputs": [{"outpoint", "script"}], "outputs": [{"n", "script"}],
    "chain", "source": "chain"}``. Esplora yields full input + output scripts in
    one request for both chains; Electrum yields output scripts plus input
    outpoints (input ownership then relies on the local UTXO inventory, since
    Electrum does not return prevout scripts inline). Liquid output scripts are
    visible without unblinding, so ownership matching needs no blinding keys.

    ``client`` lets the caller reuse one open ``ElectrumClient`` across a batch
    of txids instead of reconnecting per call.
    """
    kind = normalize_backend_kind(backend.get("kind"))
    timeout = backend_timeout(backend)
    # Esplora returns scripts regardless of chain; for Electrum the chain decides
    # bitcoin vs Liquid decoding, so fall back to the backend's configured chain
    # when the caller could not infer one from the candidate.
    chain_source = chain or backend_value(backend, "chain")
    normalized_chain = normalize_chain_value(chain_source) if chain_source else ""
    if kind == "esplora":
        tx = fetch_esplora_transaction(
            backend["url"],
            txid,
            **_esplora_call_kwargs(
                timeout=timeout,
                headers=_esplora_auth_headers(backend),
                proxy_url=_backend_proxy_url(backend),
            ),
        )
        return _legs_from_esplora_tx(tx, normalized_chain)
    if kind == "electrum":
        # Electrum can't be probed for chain, so a Liquid tx decoded as Bitcoin
        # (or vice versa) yields garbage; require an explicit chain instead.
        if normalized_chain not in ("bitcoin", "liquid"):
            raise AppError(
                "Electrum on-chain verification needs the backend's chain set to bitcoin or liquid",
                code="validation",
                hint="Set the backend chain (e.g. --chain liquid) or use an Esplora backend.",
            )
        if client is not None:
            raw_hex = client.call("blockchain.transaction.get", [txid])
        else:
            with _electrum_client_context(backend) as owned_client:
                raw_hex = owned_client.call("blockchain.transaction.get", [txid])
        if normalized_chain == "liquid":
            return _legs_from_liquid_tx(decode_liquid_transaction(raw_hex))
        return _legs_from_bitcoin_tx(decode_raw_transaction(raw_hex))
    raise AppError(
        f"On-chain verification needs an Esplora or Electrum backend, not '{kind}'",
        code="validation",
        hint="Pass --verify-backend pointing at an Esplora or Electrum endpoint.",
    )


def resolve_verify_backend(runtime_config, name=None):
    """Resolve and validate the backend used for on-chain verification.

    Normalizes the kind first (so aliases like ``liquid-esplora`` are accepted)
    and requires an Esplora or Electrum endpoint. Shared by the CLI
    ``--verify-on-chain`` handler and the ``ui.wallets.identify_onchain`` daemon
    kind so the resolution + allowlist live in one place.
    """
    if not isinstance(runtime_config, dict):
        raise AppError(
            "On-chain verification is unavailable without backend configuration",
            code="validation",
        )
    backend = resolve_backend(runtime_config, name)
    kind = normalize_backend_kind(backend.get("kind"))
    if kind not in {"esplora", "electrum"}:
        raise AppError(
            f"On-chain verification needs an Esplora or Electrum backend, not '{kind}'",
            code="validation",
            hint="Use an Esplora or Electrum backend for verification.",
        )
    return backend


@contextmanager
def verify_session(backend):
    """Yield a ``(txid, chain) -> legs`` fetcher for a batch of verifications.

    For Electrum one socket is opened for the whole batch and reused across
    txids; Esplora is stateless HTTP so each call stands alone. The yielded
    fetcher matches the ``verify_fetcher`` contract consumed by
    ``kassiber.core.ownership.identify``.
    """
    kind = normalize_backend_kind(backend.get("kind"))
    if kind == "electrum":
        with _electrum_client_context(backend) as client:
            yield lambda txid, chain=None: fetch_transaction_legs(
                backend, txid, chain, client=client
            )
    else:
        yield lambda txid, chain=None: fetch_transaction_legs(backend, txid, chain)


def _target_output_metadata(target):
    branch_label = target.get("branch_label") or "address"
    address_index = target.get("address_index")
    if address_index is None:
        label = branch_label
    else:
        label = f"{branch_label} #{address_index}"
    return {
        "address": target.get("address") or target.get("unconfidential_address") or "",
        "script_pubkey": target.get("script_pubkey") or "",
        "address_label": label,
        "branch_label": branch_label,
        "branch_index": target.get("branch_index"),
        "address_index": address_index,
    }


def _block_time_from_status(status):
    if not isinstance(status, dict):
        return None
    return _backend_time_to_iso(status.get("block_time"), default=None)


def _confirmations_from_heights(block_height, tip_height):
    if block_height in (None, "", 0, "0") or tip_height in (None, "", 0, "0"):
        return None
    try:
        normalized_block_height = int(block_height)
        normalized_tip_height = int(tip_height)
    except (TypeError, ValueError):
        return None
    if normalized_block_height <= 0 or normalized_tip_height < normalized_block_height:
        return None
    return normalized_tip_height - normalized_block_height + 1


def _esplora_tip_height(base_url, timeout=30, *, headers=None, proxy_url=None):
    try:
        return int(
            http_get_text(
                append_url_path(base_url, "blocks/tip/height"),
                **_esplora_call_kwargs(
                    timeout=timeout,
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            ).strip()
        )
    except (AppError, TypeError, ValueError):
        return None


def _block_height_from_status(status):
    if not isinstance(status, dict):
        return None
    return _positive_electrum_height(status.get("block_height"))


def _esplora_bitcoin_utxo_record(raw_utxo, target, sync_state, tip_height=None):
    status = raw_utxo.get("status") or {}
    block_height = _block_height_from_status(status)
    confirmed = bool(status.get("confirmed")) if isinstance(status, dict) else bool(block_height)
    return {
        "txid": raw_utxo.get("txid"),
        "vout": raw_utxo.get("vout"),
        "asset": "BTC",
        "amount_sats": int(raw_utxo.get("value") or 0),
        "confirmation_status": "confirmed" if confirmed else "mempool",
        "confirmations": _confirmations_from_heights(block_height, tip_height),
        "block_height": block_height,
        "block_time": _block_time_from_status(status),
        "chain": sync_state.chain,
        "network": sync_state.network,
        **_target_output_metadata(target),
        "raw": {
            "source": "esplora_scripthash_utxo",
            "confirmed": confirmed,
        },
    }


def _liquid_utxo_record_from_output(
    txid,
    vout,
    status,
    decoded_tx,
    target,
    sync_state,
    *,
    source,
    tip_height=None,
):
    if vout is None or int(vout) < 0 or int(vout) >= len(decoded_tx.vout):
        raise AppError(f"Liquid UTXO output index {vout} is out of range for transaction {txid}")
    output = decoded_tx.vout[int(vout)]
    script_hex = output.script_pubkey.data.hex()
    if script_hex != target["script_pubkey"]:
        raise AppError(f"Liquid UTXO {txid}:{vout} did not match the tracked script")
    value_sats, asset_id = liquid_output_amount_asset_id(
        output,
        sync_state.descriptor_plan,
        target=target,
    )
    block_height = _block_height_from_status(status)
    confirmed = bool(status.get("confirmed")) if isinstance(status, dict) else bool(block_height)
    return {
        "txid": txid,
        "vout": int(vout),
        "asset": liquid_asset_code(asset_id, sync_state.policy_asset_id),
        "amount_sats": value_sats,
        "confirmation_status": "confirmed" if confirmed else "mempool",
        "confirmations": _confirmations_from_heights(block_height, tip_height),
        "block_height": block_height,
        "block_time": _block_time_from_status(status),
        "chain": sync_state.chain,
        "network": sync_state.network,
        **_target_output_metadata(target),
        "raw": {
            "source": source,
            "asset_id": asset_id,
            "confirmed": confirmed,
        },
    }


def compatibility_esplora_utxos_for_wallet(backend, sync_state: WalletSyncState):
    timeout = backend_timeout(backend)
    worker_count = _bounded_http_workers(backend)
    headers = _esplora_auth_headers(backend)
    proxy_url = _backend_proxy_url(backend)
    tip_height = _esplora_tip_height(
        backend["url"],
        **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
    )

    # Phase 1: fetch each tracked script's UTXO set concurrently, bounded by the
    # same per-wallet worker budget used for stats/history. This phase runs after
    # those phases finish, so peak concurrency against the host does not rise.
    def fetch_target_utxos(target):
        raw_utxos = fetch_esplora_scripthash_utxos(
            backend["url"],
            target["script_pubkey"],
            **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
        )
        return target, list(raw_utxos or [])

    raw_seen = {"count": 0}

    def utxo_fetch_progress(index, result, total):
        raw_seen["count"] += len(result[1])
        if index % max(1, worker_count) == 0 or index == total:
            _emit_backend_progress(
                "backend_fetch",
                target_count=total,
                targets_checked=index,
                utxos_seen=raw_seen["count"],
            )

    fetched = _map_bounded(
        sync_state.targets,
        fetch_target_utxos,
        worker_count,
        on_result=utxo_fetch_progress,
    )

    # Phase 2 (Liquid only): pre-fetch and decode each referenced raw transaction
    # concurrently into a cache before the serial record build. Populating the
    # cache from _map_bounded's main-thread result iteration keeps it race-free.
    decoded_by_txid = {}
    if sync_state.chain == "liquid":
        needed_txids = []
        seen_txids = set()
        for _target, raw_utxos in fetched:
            for raw_utxo in raw_utxos:
                txid = raw_utxo.get("txid")
                if txid and txid not in seen_txids:
                    seen_txids.add(txid)
                    needed_txids.append(txid)

        def fetch_liquid_decode(txid):
            raw_hex = http_get_text(
                append_url_path(backend["url"], f"tx/{txid}/hex"),
                **_esplora_call_kwargs(
                    timeout=timeout,
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            ).strip()
            return txid, decode_liquid_transaction(raw_hex)

        for txid, decoded in _map_bounded(needed_txids, fetch_liquid_decode, worker_count):
            decoded_by_txid[txid] = decoded

    # Phase 3: build records serially in tracked-script order (executor.map keeps
    # `fetched` in input order, and each UTXO list keeps the backend's order).
    outputs = []
    for target, raw_utxos in fetched:
        for raw_utxo in raw_utxos:
            if sync_state.chain == "liquid":
                outputs.append(
                    _liquid_utxo_record_from_output(
                        raw_utxo.get("txid"),
                        raw_utxo.get("vout"),
                        raw_utxo.get("status") or {},
                        decoded_by_txid[raw_utxo.get("txid")],
                        target,
                        sync_state,
                        source="liquid_esplora_scripthash_utxo",
                        tip_height=tip_height,
                    )
                )
            else:
                outputs.append(
                    _esplora_bitcoin_utxo_record(
                        raw_utxo,
                        target,
                        sync_state,
                        tip_height=tip_height,
                    )
                )
    return outputs


def sats_to_btc(value):
    return dec(value) / SATS_PER_BTC


def _checkpoint_mapping(sync_state: WalletSyncState):
    return dict(sync_state.checkpoint or {})


def _backend_identity(backend, sync_state: WalletSyncState):
    return {
        "name": backend.get("name"),
        "kind": normalize_backend_kind(backend.get("kind")),
        "chain": sync_state.chain,
        "network": sync_state.network,
        "batch_size": backend_batch_size(backend),
    }


def _raise_silent_payment_scan_file_read_error(exc: OSError):
    raise AppError(
        "Silent Payments local scanner output could not be read",
        code="silent_payment_scanner_unavailable",
        hint=(
            "Check the backend's silent_payment_scan_file path and keep it as "
            "a private regular JSON file."
        ),
        retryable=True,
    ) from exc


def _validate_private_silent_payment_scan_file(file_stat):
    if os.name != "posix":
        return
    if not stat.S_ISREG(file_stat.st_mode):
        raise AppError(
            "Silent Payments local scanner output must be a regular file",
            code="silent_payment_scanner_unavailable",
            hint="Point silent_payment_scan_file at a private regular JSON file.",
            retryable=False,
        )
    current_uid = os.getuid()
    if file_stat.st_uid != current_uid:
        raise AppError(
            "Silent Payments local scanner output must be owned by the current OS user",
            code="silent_payment_scanner_unavailable",
            hint="Move the scanner JSON to a file owned by the user running Kassiber.",
            retryable=False,
        )
    if file_stat.st_mode & 0o077:
        raise AppError(
            "Silent Payments local scanner output is readable or writable by other users",
            code="silent_payment_scanner_unavailable",
            hint="Set the scanner JSON file permissions to 0600 before syncing.",
            details={"mode": oct(stat.S_IMODE(file_stat.st_mode))},
            retryable=False,
        )


def _open_private_silent_payment_scan_file(path: Path):
    if os.name != "posix":
        return path.open("r", encoding="utf-8")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as exc:
        _raise_silent_payment_scan_file_read_error(exc)
    try:
        _validate_private_silent_payment_scan_file(os.fstat(fd))
        return os.fdopen(fd, "r", encoding="utf-8")
    except Exception:
        os.close(fd)
        raise


def _silent_payment_scan_payload(backend, wallet, plan: silent_payments.SilentPaymentPlan):
    config = json.loads(wallet["config_json"] or "{}")
    scan_file = backend_value(backend, *silent_payments.BACKEND_SCAN_FILE_FIELDS)
    scan_path = backend_value(backend, *silent_payments.BACKEND_SCAN_PATH_FIELDS)
    if plan.scan_mode == silent_payments.SCAN_MODE_SERVER:
        if not scan_path:
            raise AppError(
                "Silent Payments server-assisted scanner is not configured for this backend",
                code="silent_payment_scanner_unavailable",
                hint="Configure silent_payment_scan_path on the selected SP-capable backend.",
                retryable=False,
            )
        payload = {
            "descriptor": config.get(silent_payments.CONFIG_DESCRIPTOR),
            "chain": plan.chain,
            "network": plan.network,
            "start_height": plan.start_height,
            "start_date": plan.start_date,
            "full_history": plan.full_history,
        }
        return http_post_json(
            append_url_path(backend["url"], scan_path),
            payload,
            timeout=backend_timeout(backend),
            proxy_url=_backend_proxy_url(backend),
        )
    if scan_file:
        path = Path(scan_file).expanduser()
        try:
            with _open_private_silent_payment_scan_file(path) as handle:
                return json.load(handle)
        except OSError as exc:
            _raise_silent_payment_scan_file_read_error(exc)
        except json.JSONDecodeError as exc:
            raise AppError(
                "Silent Payments local scanner output is not valid JSON",
                code="silent_payment_scanner_invalid",
                retryable=False,
            ) from exc
    raise AppError(
        "Silent Payments scanner is not configured for this backend",
        code="silent_payment_scanner_unavailable",
        hint=(
            "Configure a local scanner output file or an explicit server-assisted "
            "scan path on an SP-capable backend."
        ),
        retryable=False,
    )


def _silent_payment_sync_adapter(backend, wallet, sync_state: WalletSyncState):
    plan = sync_state.descriptor_plan
    if not silent_payments.is_silent_payment_plan(plan):
        raise AppError("Wallet sync state is not for Silent Payments", code="validation")
    kind = normalize_backend_kind(backend.get("kind"))
    payload = _silent_payment_scan_payload(backend, wallet, plan)
    records, meta = silent_payments.normalize_scan_payload(
        payload,
        backend_name=str(backend.get("name") or ""),
        backend_kind=kind,
        plan=plan,
        checkpoint=_checkpoint_mapping(sync_state),
        wallet_id=str(wallet["id"]) if "id" in wallet.keys() else None,
        wallet_label=str(wallet["label"]) if "label" in wallet.keys() else None,
    )
    return records, {**dict(meta or {}), "observer_route": "silent_payments"}


def _skip_unchanged_utxo_refresh(meta, sync_state: WalletSyncState):
    if not sync_state.checkpoint:
        return False
    return (
        int(meta.get("scripts_changed") or 0) == 0
        and int(meta.get("scripts_unchanged") or 0) > 0
    )


def _merge_highest_used(previous, target, used):
    highest = dict(previous or {})
    if not used:
        return highest
    branch = str(target.get("branch_index", 0))
    address_index = int(target.get("address_index") or 0)
    highest[branch] = max(int(highest.get(branch) or -1), address_index)
    return highest


def _extract_payment_hash_from_witnesses(witness_lists):
    """Opportunistically recover a Lightning payment_hash from spend witnesses.

    ``witness_lists`` is an iterable where each element is the witness item
    sequence for one transaction input (bytes-like values). Returns the
    recovered ``payment_hash`` only when exactly one input reveals a known
    Boltz HTLC claim. A batched claim is not whole-row evidence for either
    payment and therefore returns ``None``.
    """
    matches = []
    for witness_items in witness_lists:
        if not witness_items:
            continue
        extraction = htlc_parser.extract_from_claim_witness(witness_items)
        if extraction is not None and extraction.payment_hash:
            matches.append(extraction.payment_hash)
    return matches[0] if len(matches) == 1 else None


def _extract_unique_claim_payment_hash_outpoint(
    vins,
    witness_items_fn,
    prev_txid_fn=None,
    prev_vout_fn=None,
):
    """Return ``(payment_hash, funding_txid, funding_vout)`` for one claim.

    Exact payment-hash matching consumes a whole transaction row. It is safe
    only when the transaction has exactly one economic input and that input
    both reveals the HTLC preimage and names a canonical funding outpoint.
    Missing outpoint identity, batched claims, and a claim mixed with an
    ordinary wallet input fail closed instead of assigning an aggregate row to
    one witness.
    """

    vins = list(vins)
    if len(vins) != 1:
        return None
    if prev_txid_fn is None:
        prev_txid_fn = _vin_prev_txid
    if prev_vout_fn is None:
        prev_vout_fn = _vin_prev_vout
    matches = []
    for vin in vins:
        items = witness_items_fn(vin)
        if not items:
            continue
        extraction = htlc_parser.extract_from_claim_witness(items)
        if extraction is None or not extraction.payment_hash:
            continue
        funding_txid = canonical_txid(prev_txid_fn(vin))
        funding_vout = prev_vout_fn(vin)
        if funding_txid is None or type(funding_vout) is not int or funding_vout < 0:
            return None
        matches.append((extraction.payment_hash, funding_txid, funding_vout))
    return matches[0] if len(matches) == 1 else None


def _payment_hash_fields(claim_evidence):
    """Build the payment-hash entries that should appear on a sync record.

    Returns an empty dict unless the hash came from exactly one canonical claim
    outpoint, so callers can splat the result without promoting ambiguous
    whole-row evidence.
    """
    if not claim_evidence:
        return {}
    payment_hash, _funding_txid, _funding_vout = claim_evidence
    return {
        "payment_hash": payment_hash,
        "payment_hash_source": "chain_script_unique_outpoint",
    }


def _vin_prev_txid(vin):
    """Prevout txid of a decoded vin (esplora / electrum dict shape)."""
    if isinstance(vin, dict):
        txid = vin.get("txid")
        return str(txid) if txid else None
    return None


def _vin_prev_vout(vin):
    """Prevout index of a decoded dict-shaped vin."""
    if not isinstance(vin, dict):
        return None
    value = vin.get("vout")
    return value if type(value) is int and value >= 0 else None


def _extract_refund_funding_outpoint(
    vins,
    witness_items_fn,
    prev_txid_fn=_vin_prev_txid,
    prev_vout_fn=_vin_prev_vout,
):
    """Exact funding outpoint when exactly one HTLC refund vin is present.

    A transaction can batch multiple timeout sweeps. A single transaction row
    cannot represent that as one exact link, so return ``None`` rather than
    silently selecting the first refund input.
    """
    matches = []
    for vin in vins:
        items = witness_items_fn(vin)
        if not items:
            continue
        extraction = htlc_parser.extract_from_refund_witness(items)
        if extraction is None:
            continue
        prev_txid = prev_txid_fn(vin)
        prev_vout = prev_vout_fn(vin)
        if prev_txid and prev_vout is not None:
            matches.append((prev_txid, prev_vout))
    unique = list(dict.fromkeys(matches))
    return unique[0] if len(unique) == 1 else None


def _extract_refund_funding_txid(vins, witness_items_fn, prev_txid_fn=_vin_prev_txid):
    """Funding txid of the first vin whose witness is an HTLC refund spend.

    A failed swap is swept via the HTLC's CLTV timeout branch, so the vin
    carrying that refund witness spends the swap's funding (lockup) output.
    That vin's prevout txid is therefore the on-chain funding transaction the
    inbound refund should be linked back to. ``witness_items_fn`` decodes one
    vin's witness items (``_esplora_witness_items`` for dict-shaped BTC/Electrum
    vins, ``_liquid_witness_items`` for embit Liquid vins) and ``prev_txid_fn``
    resolves that vin's prevout txid (``_vin_prev_txid`` for dict vins,
    ``liquid_input_txid`` for embit Liquid vins). Returns ``None`` when no input
    reveals an HTLC refund or the prevout txid is unavailable.

    Links only the first refund input — a Boltz refund spends a single HTLC, so
    a tx batch-sweeping multiple failed swaps (rare) links one lockup and the
    rest fall back to the heuristic / manual pairing.
    """
    for vin in vins:
        items = witness_items_fn(vin)
        if not items:
            continue
        extraction = htlc_parser.extract_from_refund_witness(items)
        if extraction is None:
            continue
        prev_txid = prev_txid_fn(vin)
        if prev_txid:
            return prev_txid
    return None


def _swap_refund_fields(funding_txid, funding_vout=None):
    """Swap-refund link entries for a sync record, splattable with ``**``.

    Empty dict when there is no link so records that are not HTLC refunds stay
    unpolluted.
    """
    if not funding_txid:
        return {}
    fields = {"swap_refund_funding_txid": funding_txid}
    if type(funding_vout) is int and funding_vout >= 0:
        fields["swap_refund_funding_vout"] = funding_vout
    return fields


def _esplora_witness_items(vin_entry):
    """Decode an esplora vin's ``witness`` array of hex strings into bytes."""
    witness = vin_entry.get("witness") if isinstance(vin_entry, dict) else None
    if not witness:
        return []
    items = []
    for entry in witness:
        if isinstance(entry, str):
            try:
                items.append(bytes.fromhex(entry))
            except ValueError:
                continue
        elif isinstance(entry, (bytes, bytearray)):
            items.append(bytes(entry))
    return items


def _liquid_witness_items(vin):
    """Pull the ``script_witness`` items from an embit Liquid input.

    Returns a list of bytes-like items, or an empty list when the witness
    container is missing or empty. Liquid inherits Bitcoin Script for HTLC
    redeem scripts, so the same parser handles both chains.
    """
    witness = getattr(vin, "witness", None)
    script_witness = getattr(witness, "script_witness", None) if witness is not None else None
    items = getattr(script_witness, "items", None) if script_witness is not None else None
    if not items:
        return []
    return [bytes(item) for item in items]


def _record_from_bitcoin_graph(
    tx,
    tracked_scripts,
    backend_name,
    *,
    txid,
    occurred_at,
    confirmed_at,
    explicit_fee_sats=None,
):
    tracked = {str(value).strip().lower() for value in tracked_scripts if value}
    received_sats = sum(
        Decimal(output_value_sats(vout) or 0)
        for vout in tx.get("vout", [])
        if isinstance(vout, dict)
        and str(output_script(vout) or "").strip().lower() in tracked
    )
    sent_sats = Decimal("0")
    total_input_sats = Decimal("0")
    for vin in tx.get("vin", []):
        if not isinstance(vin, dict):
            continue
        value_sats = input_value_sats(vin)
        if value_sats is None:
            continue
        total_input_sats += value_sats
        if str(input_script(vin) or "").strip().lower() in tracked:
            sent_sats += value_sats
    if received_sats == 0 and sent_sats == 0:
        return None
    if explicit_fee_sats is None:
        total_output_sats = sum(
            Decimal(output_value_sats(vout) or 0)
            for vout in tx.get("vout", [])
            if isinstance(vout, dict)
        )
        fee_sats = max(total_input_sats - total_output_sats, Decimal("0"))
    else:
        fee_sats = max(dec(explicit_fee_sats, "0"), Decimal("0"))
    if received_sats > sent_sats:
        direction = "inbound"
        amount = sats_to_btc(received_sats - sent_sats)
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out_sats = sent_sats - received_sats
        amount_sats = gross_out_sats - fee_sats
        if amount_sats < 0:
            amount_sats = Decimal("0")
        amount = sats_to_btc(amount_sats)
        fee = sats_to_btc(fee_sats)
        kind = "withdrawal" if amount > 0 else "fee"
    claim_evidence = _extract_unique_claim_payment_hash_outpoint(
        tx.get("vin", []), _esplora_witness_items
    )
    swap_refund_funding_outpoint = _extract_refund_funding_outpoint(
        tx.get("vin", []), _esplora_witness_items
    )
    return {
        "txid": txid,
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(tx, sort_keys=True),
        **_payment_hash_fields(claim_evidence),
        **_swap_refund_fields(*(swap_refund_funding_outpoint or (None, None))),
    }


def record_from_bitcoin_esplora_tx(tx, tracked_scripts, backend_name):
    status = tx.get("status") or {}
    block_time = status.get("block_time")
    occurred_at = timestamp_to_iso(block_time or tx.get("observed_at"))
    confirmed_at = (
        timestamp_to_iso(block_time, default=None)
        if status.get("confirmed") is True or block_time is not None
        else None
    )
    return _record_from_bitcoin_graph(
        tx,
        tracked_scripts,
        backend_name,
        txid=tx.get("txid"),
        occurred_at=occurred_at,
        confirmed_at=confirmed_at,
        explicit_fee_sats=tx.get("fee"),
    )


def liquid_asset_id_from_bytes(asset_bytes):
    if not isinstance(asset_bytes, (bytes, bytearray)) or len(asset_bytes) != 32:
        raise AppError("Unsupported Liquid asset encoding while decoding transaction")
    return bytes(reversed(bytes(asset_bytes))).hex()


def liquid_output_amount_asset_id(output, plan, target=None):
    try:
        if getattr(output, "is_blinded", False):
            if target is None:
                raise AppError("Unable to unblind Liquid output without wallet descriptor context")
            secret, _ = liquid_blinding_secret(plan, target["branch_index"], target["address_index"])
            value, asset_bytes, *_ = output.unblind(secret)
        else:
            value = output.value
            asset_bytes = output.asset
    except AppError:
        raise
    except Exception as exc:
        label = target["address"] if target and target.get("address") else target["script_pubkey"] if target else "fee output"
        raise AppError(f"Failed to decode Liquid output for {label}") from exc
    if not isinstance(value, int):
        raise AppError("Unsupported confidential Liquid value encoding while decoding transaction")
    return value, liquid_asset_id_from_bytes(asset_bytes)


def liquid_input_txid(vin):
    txid = getattr(vin, "txid", None)
    if isinstance(txid, str):
        return txid.lower()
    if isinstance(txid, (bytes, bytearray)):
        return bytes(txid).hex()
    raise AppError("Unsupported Liquid input txid encoding while decoding transaction")


def liquid_input_vout(vin):
    value = getattr(vin, "vout", None)
    if type(value) is int and value >= 0:
        return value
    return None


def record_components_from_liquid_tx(
    txid,
    occurred_at,
    tx,
    descriptor_plan,
    tracked_scripts,
    backend_name,
    policy_asset_id,
    prev_tx_lookup,
    raw_json_context=None,
    confirmed_at=None,
    network=None,
):
    net_sats = defaultdict(int)
    fee_sats = defaultdict(int)
    stored_vout = []
    for output_index, output in enumerate(tx.vout):
        script_hex = output.script_pubkey.data.hex()
        stored_output = {
            "n": output_index,
            "scriptpubkey": script_hex,
        }
        if script_hex == "":
            value_sats, asset_id = liquid_output_amount_asset_id(output, descriptor_plan, target=None)
            fee_sats[asset_id] += value_sats
            stored_output.update(
                {
                    "value_sats": value_sats,
                    "asset_id": asset_id,
                    "asset": liquid_asset_code(asset_id, policy_asset_id),
                    "role": "fee",
                }
            )
            stored_vout.append(stored_output)
            continue
        target = tracked_scripts.get(script_hex)
        if not target:
            # An explicit/unconfidential foreign output is safe to retain.  A
            # confidential foreign output cannot be unblinded with this
            # wallet's key and deliberately remains script-only; another owned
            # wallet may contribute its value when profile observations merge.
            try:
                value_sats, asset_id = liquid_output_amount_asset_id(
                    output, descriptor_plan, target=None
                )
            except AppError:
                pass
            else:
                stored_output.update(
                    {
                        "value_sats": value_sats,
                        "asset_id": asset_id,
                        "asset": liquid_asset_code(asset_id, policy_asset_id),
                        "role": "external",
                    }
                )
            stored_vout.append(stored_output)
            continue
        value_sats, asset_id = liquid_output_amount_asset_id(output, descriptor_plan, target=target)
        net_sats[asset_id] += value_sats
        stored_output.update(
            {
                "value_sats": value_sats,
                "asset_id": asset_id,
                "asset": liquid_asset_code(asset_id, policy_asset_id),
                "role": "owned",
            }
        )
        stored_vout.append(stored_output)
    # Persist valued historical prevouts, not just today's UTXO inventory.
    # This evidence survives after the input is spent and is sufficient for an
    # offline journal replay without descriptor/blinding secrets.
    stored_vin = []
    has_owned_input = False
    for vin in tx.vin:
        prev_txid = liquid_input_txid(vin)
        prev_vout = getattr(vin, "vout", None)
        if prev_vout is None:
            continue
        stored_input = {"txid": prev_txid, "vout": prev_vout}
        prev_tx = prev_tx_lookup(prev_txid)
        if prev_vout >= len(prev_tx.vout):
            raise AppError(f"Liquid prevout index {prev_vout} is out of range for transaction {prev_txid}")
        prev_output = prev_tx.vout[prev_vout]
        script_hex = prev_output.script_pubkey.data.hex()
        target = tracked_scripts.get(script_hex)
        if not target:
            stored_prevout = {"scriptpubkey": script_hex, "role": "external"}
            try:
                value_sats, asset_id = liquid_output_amount_asset_id(
                    prev_output, descriptor_plan, target=None
                )
            except AppError:
                pass
            else:
                stored_prevout.update(
                    {
                        "value_sats": value_sats,
                        "asset_id": asset_id,
                        "asset": liquid_asset_code(asset_id, policy_asset_id),
                    }
                )
            stored_input["prevout"] = stored_prevout
            stored_vin.append(stored_input)
            continue
        value_sats, asset_id = liquid_output_amount_asset_id(prev_output, descriptor_plan, target=target)
        has_owned_input = True
        net_sats[asset_id] -= value_sats
        stored_input["prevout"] = {
            "scriptpubkey": script_hex,
            "value_sats": value_sats,
            "asset_id": asset_id,
            "asset": liquid_asset_code(asset_id, policy_asset_id),
            "role": "owned",
        }
        stored_vin.append(stored_input)
    claim_evidence = _extract_unique_claim_payment_hash_outpoint(
        tx.vin,
        _liquid_witness_items,
        prev_txid_fn=liquid_input_txid,
        prev_vout_fn=liquid_input_vout,
    )
    payment_hash_fields = _payment_hash_fields(claim_evidence)
    # Liquid vins are embit objects, so the refund link needs the embit-aware
    # witness + prevout-txid helpers rather than the dict-shaped defaults.
    swap_refund_outpoint = _extract_refund_funding_outpoint(
        tx.vin,
        _liquid_witness_items,
        prev_txid_fn=liquid_input_txid,
        prev_vout_fn=liquid_input_vout,
    )
    swap_refund_fields = _swap_refund_fields(
        *(swap_refund_outpoint or (None, None))
    )
    records = []
    # The explicit fee output is transaction-global evidence, but only wallets
    # that own a funding input paid it. A destination-only observer must not get
    # a synthetic policy-asset fee row (especially on issued-asset receipts).
    accounted_fee_sats = fee_sats if has_owned_input else {}
    all_assets = sorted(set(net_sats) | set(accounted_fee_sats))
    for asset_id in all_assets:
        asset_code = liquid_asset_code(asset_id, policy_asset_id)
        net_value = dec(net_sats.get(asset_id, 0), default="0")
        fee_value = dec(accounted_fee_sats.get(asset_id, 0), default="0")
        if net_value == 0 and fee_value == 0:
            continue
        if net_value > 0:
            direction = "inbound"
            amount = sats_to_btc(net_value)
            fee = Decimal("0")
            kind = "deposit"
        else:
            direction = "outbound"
            gross_out_sats = abs(net_value)
            amount_sats = gross_out_sats - fee_value
            if amount_sats < 0:
                amount_sats = Decimal("0")
            amount = sats_to_btc(amount_sats)
            fee = sats_to_btc(fee_value)
            kind = "withdrawal" if amount > 0 else "fee"
        records.append(
            {
                "txid": txid,
                "occurred_at": occurred_at,
                "confirmed_at": (
                    confirmed_at
                    if confirmed_at is not None
                    else (None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at)
                ),
                "direction": direction,
                "asset": asset_code,
                "amount": amount,
                "fee": fee,
                "fiat_rate": None,
                "fiat_value": None,
                "kind": kind,
                "description": f"Synced from {backend_name}",
                "counterparty": None,
                "raw_json": json.dumps(
                    json_ready(
                        {
                            **(raw_json_context or {}),
                            "txid": txid,
                            "chain": "liquid",
                            "network": network or "",
                            "ownership_graph_version": 1,
                            "vin": stored_vin,
                            "vout": stored_vout,
                            "component": {
                                "asset_id": asset_id,
                                "asset": asset_code,
                                "net_sats": int(net_value),
                                "fee_sats": int(fee_value),
                            },
                        }
                    ),
                    sort_keys=True,
                ),
                **payment_hash_fields,
                **swap_refund_fields,
            }
        )
    return records


def compatibility_esplora_records_for_wallet(backend, sync_state: WalletSyncState):
    max_pages = parse_int(backend_value(backend, "maxpages"), default=0) or None
    timeout = backend_timeout(backend)
    worker_count = _bounded_http_workers(backend)
    headers = _esplora_auth_headers(backend)
    proxy_url = _backend_proxy_url(backend)
    checkpoint = _checkpoint_mapping(sync_state)
    previous_stats = checkpoint.get("esplora_scripthashes") or {}
    next_stats = {}
    highest_used = dict(checkpoint.get("highest_used") or {})
    changed_targets = []
    unchanged_scripts = 0

    def fetch_stats(target):
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        stats = esplora_scripthash_stats(
            backend["url"],
            target["script_pubkey"],
            **_esplora_call_kwargs(timeout=timeout, headers=headers, proxy_url=proxy_url),
        )
        return target, scripthash, stats

    def stats_fetch_progress(index, _result, total):
        if index % max(1, worker_count) == 0 or index == total:
            _emit_backend_progress(
                "backend_fetch",
                target_count=total,
                targets_checked=index,
            )

    for stats_index, (target, scripthash, stats) in enumerate(
        _map_bounded(
            sync_state.targets,
            fetch_stats,
            worker_count,
            on_result=stats_fetch_progress,
        ),
        start=1,
    ):
        chain_stats = stats.get("chain_stats") or {}
        mempool_stats = stats.get("mempool_stats") or {}
        tx_count = int(chain_stats.get("tx_count") or 0) + int(mempool_stats.get("tx_count") or 0)
        mempool_tx_count = int(mempool_stats.get("tx_count") or 0)
        fingerprint = esplora_stats_fingerprint(stats)
        previous = previous_stats.get(scripthash) if isinstance(previous_stats, dict) else None
        previous_dirty = isinstance(previous, dict) and bool(previous.get("mempool_dirty"))
        unchanged = (
            isinstance(previous, dict)
            and previous.get("fingerprint") == fingerprint
            and not previous_dirty
        )
        next_stats[scripthash] = {
            "fingerprint": fingerprint,
            "tx_count": tx_count,
            "mempool_tx_count": mempool_tx_count,
            "mempool_dirty": mempool_tx_count > 0,
        }
        highest_used = _merge_highest_used(highest_used, target, tx_count > 0)
        if unchanged:
            unchanged_scripts += 1
        else:
            changed_targets.append((target, scripthash))
        if stats_index % max(1, worker_count) == 0 or stats_index == len(sync_state.targets):
            _emit_backend_progress(
                "backend_fetch",
                target_count=len(sync_state.targets),
                targets_checked=stats_index,
                scripts_changed=len(changed_targets),
                scripts_unchanged=unchanged_scripts,
            )
    transactions_by_txid = {}
    def fetch_target_transactions(item):
        target, scripthash = item
        target_txs = []
        for tx in fetch_esplora_scripthash_transactions(
            backend["url"],
            target["script_pubkey"],
            **_esplora_call_kwargs(
                timeout=timeout,
                headers=headers,
                proxy_url=proxy_url,
                max_pages=max_pages,
            ),
            ):
            target_txs.append(tx)
        return scripthash, target_txs

    def history_fetch_progress(index, _result, total):
        if index % max(1, worker_count) == 0 or index == total:
            _emit_backend_progress(
                "backend_fetch",
                target_count=total,
                targets_checked=index,
            )

    for history_index, (_scripthash, target_txs) in enumerate(
        _map_bounded(
            changed_targets,
            fetch_target_transactions,
            worker_count,
            on_result=history_fetch_progress,
        ),
        start=1,
    ):
        for tx in target_txs:
            transactions_by_txid[tx["txid"]] = tx
        if history_index % max(1, worker_count) == 0 or history_index == len(changed_targets):
            _emit_backend_progress(
                "backend_fetch",
                target_count=len(changed_targets),
                targets_checked=history_index,
                known_txids=len(transactions_by_txid),
            )
    records = []
    raw_tx_cache = {}

    def liquid_tx_lookup(txid):
        if txid not in raw_tx_cache:
            raw_hex = http_get_text(
                append_url_path(backend["url"], f"tx/{txid}/hex"),
                **_esplora_call_kwargs(
                    timeout=backend_timeout(backend),
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            ).strip()
            raw_tx_cache[txid] = {
                "raw_hex": raw_hex,
                "decoded": decode_liquid_transaction(raw_hex),
            }
        return raw_tx_cache[txid]["decoded"]

    sorted_transactions = sorted(
        transactions_by_txid.values(),
        key=lambda item: (((item.get("status") or {}).get("block_time") or 0), item.get("txid", "")),
    )
    if sync_state.chain == "liquid":
        # Pre-fetch and decode each wallet transaction's raw hex concurrently
        # (bounded by the per-wallet worker budget) before the serial record
        # build. Populating the shared cache from _map_bounded's main-thread
        # result iteration keeps it race-free; prevout lookups during record
        # building still fall back to the same cache serially on the main thread.
        main_txids = [
            tx["txid"] for tx in sorted_transactions if tx["txid"] not in raw_tx_cache
        ]

        def fetch_liquid_main_tx(txid):
            raw_hex = http_get_text(
                append_url_path(backend["url"], f"tx/{txid}/hex"),
                **_esplora_call_kwargs(
                    timeout=timeout,
                    headers=headers,
                    proxy_url=proxy_url,
                ),
            ).strip()
            return txid, raw_hex

        for txid, raw_hex in _map_bounded(main_txids, fetch_liquid_main_tx, worker_count):
            raw_tx_cache[txid] = {
                "raw_hex": raw_hex,
                "decoded": decode_liquid_transaction(raw_hex),
            }
    for tx_index, tx in enumerate(sorted_transactions, start=1):
        if sync_state.chain == "liquid":
            cached_tx = raw_tx_cache[tx["txid"]]
            raw_hex = cached_tx["raw_hex"]
            decoded_tx = cached_tx["decoded"]
            records.extend(
                record_components_from_liquid_tx(
                    tx["txid"],
                    timestamp_to_iso((tx.get("status") or {}).get("block_time")),
                    decoded_tx,
                    sync_state.descriptor_plan,
                    sync_state.tracked_scripts,
                    backend["name"],
                    sync_state.policy_asset_id,
                    liquid_tx_lookup,
                    {"tx": tx, "raw_hex": raw_hex},
                    confirmed_at=timestamp_to_iso((tx.get("status") or {}).get("block_time"), default=None),
                    network=sync_state.network,
                )
            )
        else:
            normalized = record_from_bitcoin_esplora_tx(tx, sync_state.tracked_scripts, backend["name"])
            if normalized:
                records.append(normalized)
        if tx_index % 100 == 0 or tx_index == len(sorted_transactions):
            _emit_backend_progress(
                "decode_enrich",
                transactions_seen=tx_index,
                transactions_total=len(sorted_transactions),
                records=len(records),
            )
    checkpoint.update(
        {
            "backend": _backend_identity(backend, sync_state),
            "esplora_scripthashes": dict(sorted(next_stats.items())),
            "highest_used": dict(sorted(highest_used.items())),
        }
    )
    return records, {
        "freshness_checkpoint": checkpoint,
        "scripts_changed": len(changed_targets),
        "scripts_unchanged": unchanged_scripts,
        "known_txids": len(transactions_by_txid),
    }


def compatibility_esplora_sync_adapter(backend, wallet, sync_state):
    if silent_payments.is_silent_payment_plan(sync_state.descriptor_plan):
        return _silent_payment_sync_adapter(backend, wallet, sync_state)
    records, meta = compatibility_esplora_records_for_wallet(backend, sync_state)
    if _skip_unchanged_utxo_refresh(meta, sync_state):
        meta["utxos_skipped_unchanged"] = True
    else:
        meta["utxos"] = compatibility_esplora_utxos_for_wallet(backend, sync_state)
    return records, meta


def bitcoinrpc_auth_headers(backend):
    cookie_path = backend_value(backend, "cookiefile", "cookie_file")
    if cookie_path:
        token = Path(cookie_path).expanduser().read_text(encoding="utf-8").strip()
    else:
        username = backend_value(backend, "username", "rpcuser", "rpc_user")
        password = backend_value(backend, "password", "rpcpassword", "rpc_password")
        if not username or password is None:
            raise AppError(
                f"Bitcoin Core backend '{backend['name']}' requires USERNAME/PASSWORD or COOKIEFILE configuration"
            )
        token = f"{username}:{password}"
    encoded = base64.b64encode(token.encode("utf-8")).decode("ascii")
    return {"Authorization": f"Basic {encoded}"}


def bitcoinrpc_url(backend, wallet_name=None):
    if wallet_name:
        return append_url_path(backend["url"], f"wallet/{urlparse.quote(wallet_name, safe='')}")
    return backend["url"]


def bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
    payload = {
        "jsonrpc": "1.0",
        "id": f"{APP_NAME}-{method}",
        "method": method,
        "params": [] if params is None else params,
    }
    response = http_post_json(
        bitcoinrpc_url(backend, wallet_name=wallet_name),
        payload,
        headers=bitcoinrpc_auth_headers(backend),
        timeout=backend_timeout(backend) if timeout is None else timeout,
        proxy_url=_backend_proxy_url(backend),
    )
    if response.get("error"):
        error = response["error"]
        raise AppError(
            f"Bitcoin Core RPC {method} failed"
            f" ({error.get('code', 'unknown')}): {error.get('message', error)}"
        )
    return response.get("result")


def bitcoinrpc_import_timeout(backend):
    return max(backend_timeout(backend), 1800)


def bitcoinrpc_wallet_name(backend, wallet):
    explicit_name = backend_value(backend, "wallet")
    if explicit_name:
        return explicit_name
    prefix = backend_value(backend, "walletprefix", "wallet_prefix") or APP_NAME
    return f"{sanitize_wallet_segment(prefix)}-{sanitize_wallet_segment(wallet['id'])}"


def bitcoinrpc_ensure_watchonly_wallet(backend, wallet):
    wallet_name = bitcoinrpc_wallet_name(backend, wallet)
    loaded_wallets = bitcoinrpc_call(backend, "listwallets")
    if wallet_name in loaded_wallets:
        return wallet_name
    try:
        bitcoinrpc_call(backend, "loadwallet", [wallet_name, True])
        return wallet_name
    except AppError as load_error:
        create_attempts = [
            [wallet_name, True, True, "", False, True, True],
            [wallet_name, True, True, "", False, True],
            [wallet_name, True, True],
        ]
        for params in create_attempts:
            try:
                bitcoinrpc_call(backend, "createwallet", params)
                return wallet_name
            except AppError:
                continue
        raise AppError(
            f"Unable to load or create Bitcoin Core wallet '{wallet_name}' for backend '{backend['name']}'. "
            f"Last load error: {load_error}"
        ) from load_error


def bitcoinrpc_import_addresses(backend, wallet_name, wallet, addresses, birthday_ts=0):
    label = f"{APP_NAME}:{wallet['id']}"
    missing_addresses = []
    descriptors = []
    for address in addresses:
        info = bitcoinrpc_call(backend, "getaddressinfo", [address], wallet_name=wallet_name)
        if info.get("iswatchonly") or info.get("ismine"):
            continue
        descriptor = bitcoinrpc_call(backend, "getdescriptorinfo", [f"addr({address})"])
        descriptors.append(
            {"desc": descriptor["descriptor"], "timestamp": birthday_ts, "label": label}
        )
        missing_addresses.append(address)
    if not missing_addresses:
        return 0
    try:
        results = bitcoinrpc_call(
            backend,
            "importdescriptors",
            [descriptors],
            wallet_name=wallet_name,
            timeout=bitcoinrpc_import_timeout(backend),
        )
        failures = [result for result in results if not result.get("success")]
        if failures:
            raise AppError(f"descriptor import failed: {failures[0]}")
    except AppError:
        requests = [
            {
                "scriptPubKey": {"address": address},
                "timestamp": birthday_ts,
                "watchonly": True,
                "label": label,
            }
            for address in missing_addresses
        ]
        options = {"rescan": True}
        results = bitcoinrpc_call(
            backend,
            "importmulti",
            [requests, options],
            wallet_name=wallet_name,
            timeout=bitcoinrpc_import_timeout(backend),
        )
        failures = [result for result in results if not result.get("success")]
        if failures:
            raise AppError(f"address import failed: {failures[0]}")
    return len(missing_addresses)


def _int_mapping(value):
    if not isinstance(value, dict):
        return {}
    output = {}
    for key, raw in value.items():
        try:
            parsed = int(raw)
        except (TypeError, ValueError):
            continue
        if parsed >= 0:
            output[str(key)] = parsed
    return output


def _bitcoinrpc_descriptor_end(plan, branch, highest_used, previous_end=None):
    descriptor = branch_descriptor(branch)
    if not getattr(descriptor, "is_wildcard", False):
        return 0
    known_highest = _highest_used_branch_index(highest_used, branch.branch_index)
    if known_highest is None:
        previous = int(previous_end) if previous_end is not None else -1
        return max(0, previous, plan.gap_limit - 1)
    previous = int(previous_end) if previous_end is not None else -1
    return max(0, previous, known_highest + plan.gap_limit)


def _bitcoinrpc_descriptor_targets_for_range_ends(plan, range_ends):
    range_ends = _int_mapping(range_ends)
    targets = []
    for branch in plan.branches:
        end = range_ends.get(str(branch.branch_index), 0)
        if end <= 0:
            targets.append(
                sync_target_from_derived(
                    derive_descriptor_target(plan, branch.branch_index, 0)
                )
            )
            continue
        targets.extend(
            sync_target_from_derived(target)
            for target in derive_descriptor_targets(
                plan,
                branch_index=branch.branch_index,
                start=0,
                end=end + 1,
            )
        )
    return targets


def _bitcoinrpc_descriptor_targets_for_checkpoint(plan, checkpoint):
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    previous_ends = _int_mapping(checkpoint.get("bitcoinrpc_descriptor_range_ends"))
    highest_used = checkpoint.get("highest_used")
    range_ends = {}
    for branch in plan.branches:
        branch_key = str(branch.branch_index)
        range_ends[branch_key] = _bitcoinrpc_descriptor_end(
            plan,
            branch,
            highest_used,
            previous_ends.get(branch_key),
        )
    return _bitcoinrpc_descriptor_targets_for_range_ends(plan, range_ends)


def bitcoinrpc_ranged_descriptor(backend, branch, end, birthday_ts):
    descriptor_template = branch_descriptor(branch)
    raw_descriptor = descriptor_template.to_string()
    descriptor = bitcoinrpc_call(backend, "getdescriptorinfo", [raw_descriptor])
    request = {
        "desc": descriptor["descriptor"],
        "timestamp": birthday_ts,
        "internal": branch.branch_index % 2 == 1,
        "active": False,
    }
    if getattr(descriptor_template, "is_wildcard", False):
        request["range"] = [0, int(end)]
    return request


def bitcoinrpc_import_ranged_descriptors(backend, wallet_name, plan, checkpoint, birthday_ts):
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    highest_used = checkpoint.get("highest_used")
    previous_ends = _int_mapping(checkpoint.get("bitcoinrpc_descriptor_range_ends"))
    next_ends = dict(previous_ends)
    descriptors = []
    for branch in plan.branches:
        branch_key = str(branch.branch_index)
        end = _bitcoinrpc_descriptor_end(
            plan,
            branch,
            highest_used,
            previous_ends.get(branch_key),
        )
        next_ends[branch_key] = max(int(next_ends.get(branch_key, -1)), end)
        if end <= int(previous_ends.get(branch_key, -1)):
            continue
        descriptors.append(bitcoinrpc_ranged_descriptor(backend, branch, end, birthday_ts))
    if not descriptors:
        return 0, next_ends
    results = bitcoinrpc_call(
        backend,
        "importdescriptors",
        [descriptors],
        wallet_name=wallet_name,
        timeout=bitcoinrpc_import_timeout(backend),
    )
    failures = [result for result in results if not result.get("success")]
    if failures:
        raise AppError(f"ranged descriptor import failed: {failures[0]}")
    return len(descriptors), next_ends


def _bitcoinrpc_checkpoint_block(checkpoint):
    if not isinstance(checkpoint, dict):
        return None
    value = checkpoint.get("bitcoinrpc_last_block")
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def _bitcoinrpc_txids_from_details(details):
    txids = set()
    for detail in details or []:
        if not isinstance(detail, dict):
            continue
        txid = str(detail.get("txid") or "").strip().lower()
        if txid:
            txids.add(txid)
    return txids


def _bitcoinrpc_detail_category(detail):
    return str(detail.get("category") or "").lower()


def _bitcoinrpc_detail_is_retracted(detail):
    if _bitcoinrpc_detail_category(detail) == "orphan":
        return True
    return int(detail.get("confirmations") or 0) < 0


def _bitcoinrpc_retracted_txids(details):
    return {
        str(detail.get("txid") or "").strip().lower()
        for detail in details or []
        if isinstance(detail, dict)
        and detail.get("txid")
        and _bitcoinrpc_detail_is_retracted(detail)
    }


def _bitcoinrpc_has_immature_details(details):
    return any(
        isinstance(detail, dict)
        and _bitcoinrpc_detail_category(detail) == "immature"
        for detail in details or []
    )


def fetch_bitcoinrpc_wallet_transactions(backend, wallet_name, page_size=1000, checkpoint=None):
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    pending_maturity = bool(checkpoint.get("bitcoinrpc_pending_maturity"))
    last_block = None if pending_maturity else _bitcoinrpc_checkpoint_block(checkpoint)
    fallback_reason = None
    removed_txids = set()
    if pending_maturity:
        fallback_reason = "Bitcoin Core wallet has immature transactions awaiting maturity"
    if last_block:
        try:
            payload = bitcoinrpc_call(
                backend,
                "listsinceblock",
                [last_block, 1, True, True],
                wallet_name=wallet_name,
            )
        except AppError as exc:
            if "Bitcoin Core RPC listsinceblock failed" not in str(exc):
                raise
            fallback_reason = str(exc)
        else:
            if not isinstance(payload, dict):
                fallback_reason = "Bitcoin Core RPC listsinceblock returned an unexpected payload"
            else:
                transactions = payload.get("transactions") or []
                removed = payload.get("removed") or []
                if not isinstance(transactions, list):
                    fallback_reason = "Bitcoin Core RPC listsinceblock returned unexpected transactions"
                elif removed:
                    removed_txids = _bitcoinrpc_txids_from_details(removed)
                    fallback_reason = "Bitcoin Core RPC listsinceblock reported removed transactions"
                else:
                    retracted_txids = _bitcoinrpc_retracted_txids(transactions)
                    return list(transactions), {
                        "bitcoinrpc_sync_mode": "sinceblock",
                        "bitcoinrpc_last_block": payload.get("lastblock") or last_block,
                        "bitcoinrpc_removed": 0,
                        "bitcoinrpc_pending_maturity": _bitcoinrpc_has_immature_details(
                            transactions
                        ),
                        **(
                            {"bitcoinrpc_retracted_txids": sorted(retracted_txids)}
                            if retracted_txids
                            else {}
                        ),
                    }

    transactions = []
    skip = 0
    while True:
        page = bitcoinrpc_call(backend, "listtransactions", ["*", page_size, skip, True], wallet_name=wallet_name)
        if not page:
            break
        transactions.extend(page)
        if len(page) < page_size:
            break
        skip += page_size
    retracted_txids = removed_txids | _bitcoinrpc_retracted_txids(transactions)
    meta = {
        "bitcoinrpc_sync_mode": "full_scan",
        "bitcoinrpc_pending_maturity": _bitcoinrpc_has_immature_details(transactions),
    }
    if retracted_txids:
        meta["bitcoinrpc_retracted_txids"] = sorted(retracted_txids)
    if fallback_reason:
        meta["bitcoinrpc_sinceblock_fallback"] = fallback_reason
    try:
        last_block = bitcoinrpc_call(backend, "getbestblockhash")
    except AppError:
        last_block = None
    if last_block:
        meta["bitcoinrpc_last_block"] = last_block
    return transactions, meta


def record_from_bitcoinrpc_details(
    txid,
    details,
    backend_name,
    raw_graph=None,
    tracked_scripts=None,
):
    amount_total = Decimal("0")
    fee_total = Decimal("0")
    occurred_at = UNKNOWN_OCCURRED_AT
    confirmed_at = None
    for detail in details:
        category = _bitcoinrpc_detail_category(detail)
        if category == "immature" or _bitcoinrpc_detail_is_retracted(detail):
            continue
        amount_total += dec(detail.get("amount"), "0")
        # Bitcoin Core stamps the SAME whole-tx fee on every `send`-category
        # detail of one transaction, so summing per detail double-counts it for a
        # multi-output send (inflating both the booked outflow and the taxable
        # fee disposal). Take it once — the shared fee is identical across send
        # details; receive details carry 0/none, so max() yields the real fee.
        detail_fee = abs(dec(detail.get("fee"), "0"))
        if detail_fee > fee_total:
            fee_total = detail_fee
        if detail.get("blocktime") not in (None, "", 0, "0"):
            confirmed_at = timestamp_to_iso(detail.get("blocktime"), default=None)
        occurred_at = timestamp_to_iso(detail.get("blocktime") or detail.get("time"), default=occurred_at)
    if amount_total == 0 and fee_total == 0:
        return None
    privacy_boundary = None
    if amount_total > 0:
        direction = "inbound"
        amount = amount_total
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out = abs(amount_total)
        graph_amount = _bitcoinrpc_graph_outbound_amount(raw_graph, tracked_scripts, fee_total)
        if graph_amount is not None:
            # Decode-backed Core sync can use the same amount model as the
            # Esplora adapter: sum outputs not paying this wallet's tracked
            # scripts, with the network fee kept separately.
            amount = graph_amount
        else:
            # Legacy fallback for older Core/tapes with only wallet details:
            # Core send details already report recipient value separately from
            # the fee. Keep that amount intact so downstream accounting consumes
            # recipient value plus the explicit fee exactly once.
            amount = gross_out
        fee = fee_total
        kind = "withdrawal" if amount > 0 else "fee"
        if _bitcoinrpc_graph_has_foreign_inputs(raw_graph, tracked_scripts):
            privacy_boundary = "payjoin"
    raw_payload = raw_graph if raw_graph is not None else details
    if privacy_boundary and isinstance(raw_payload, dict):
        raw_payload = {**raw_payload, "privacy_boundary": privacy_boundary}
    record = {
        "txid": txid,
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(json_ready(raw_payload), sort_keys=True),
    }
    if privacy_boundary:
        record["privacy_boundary"] = privacy_boundary
    return record


def _bitcoinrpc_prevout_from_vin(vin):
    prevout = vin.get("prevout")
    if not isinstance(prevout, dict):
        return {}
    script = normalized_script_hex(output_script(prevout))
    value_sats = output_value_sats(prevout)
    result = {}
    if script:
        result["scriptpubkey"] = script
    if value_sats is not None:
        result["value"] = value_sats
    return result


def _bitcoinrpc_graph_outbound_amount(raw_graph, tracked_scripts, fee_btc):
    if not isinstance(raw_graph, dict):
        return None
    tracked = {str(script).lower() for script in (tracked_scripts or set()) if script}
    if not tracked:
        return None
    outputs = raw_graph.get("vout")
    if not isinstance(outputs, list):
        return None
    received_sats = 0
    external_sats = 0
    for output in outputs:
        if not isinstance(output, dict):
            return None
        script = output.get("scriptpubkey")
        if not script:
            return None
        if str(script).lower() in tracked:
            try:
                received_sats += int(output.get("value"))
            except (TypeError, ValueError):
                return None
            continue
        try:
            external_sats += int(output.get("value"))
        except (TypeError, ValueError):
            return None
    inputs = raw_graph.get("vin")
    sent_sats = 0
    if isinstance(inputs, list):
        for item in inputs:
            if not isinstance(item, dict):
                continue
            prevout = item.get("prevout")
            if not isinstance(prevout, dict):
                continue
            script = prevout.get("scriptpubkey")
            if not script or str(script).lower() not in tracked:
                continue
            try:
                sent_sats += int(prevout.get("value"))
            except (TypeError, ValueError):
                return None
    if sent_sats > 0:
        fee_sats = int((dec(fee_btc, "0") * SATS_PER_BTC).to_integral_value())
        return sats_to_btc(max(sent_sats - received_sats - fee_sats, 0))
    return None


def _bitcoinrpc_graph_has_foreign_inputs(raw_graph, tracked_scripts) -> bool:
    if not isinstance(raw_graph, dict):
        return False
    tracked = {str(script).lower() for script in (tracked_scripts or set()) if script}
    if not tracked:
        return False
    inputs = raw_graph.get("vin")
    if not isinstance(inputs, list):
        return False
    has_tracked = False
    has_foreign = False
    for item in inputs:
        if not isinstance(item, dict):
            continue
        prevout = item.get("prevout")
        if not isinstance(prevout, dict):
            continue
        script = prevout.get("scriptpubkey")
        if not script:
            continue
        if str(script).lower() in tracked:
            has_tracked = True
        else:
            has_foreign = True
    return has_tracked and has_foreign


def _bitcoinrpc_normalized_graph(txid, payload):
    decoded = payload.get("decoded") if isinstance(payload, dict) else None
    if not isinstance(decoded, dict):
        return None
    vin = decoded.get("vin")
    vout = decoded.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None
    normalized_vin = []
    for entry in vin:
        if not isinstance(entry, dict):
            continue
        item = {}
        if entry.get("txid") is not None:
            item["txid"] = str(entry.get("txid")).lower()
        if entry.get("vout") is not None:
            try:
                item["vout"] = int(entry.get("vout"))
            except (TypeError, ValueError):
                # Ignore malformed prevout index from backend payload and
                # continue best-effort normalization without a `vout` field.
                item.pop("vout", None)
        prevout = _bitcoinrpc_prevout_from_vin(entry)
        if prevout:
            item["prevout"] = prevout
        normalized_vin.append(item)
    normalized_vout = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, dict):
            continue
        value_sats = output_value_sats(entry)
        script = normalized_script_hex(output_script(entry))
        if value_sats is None or not script:
            continue
        try:
            output_index = int(entry.get("n", position))
        except (TypeError, ValueError):
            output_index = position
        normalized_vout.append(
            {"n": output_index, "scriptpubkey": script, "value": value_sats}
        )
    if not normalized_vin or not normalized_vout:
        return None
    return {
        "txid": str((decoded.get("txid") or txid)).lower(),
        "vin": normalized_vin,
        "vout": normalized_vout,
        "source": "bitcoinrpc_gettransaction",
    }


def _bitcoinrpc_fetch_normalized_graph(backend, wallet_name, txid, tx_cache=None):
    try:
        cache_key = str(txid)
        payload = tx_cache.get(cache_key) if tx_cache is not None else None
        if not isinstance(payload, dict) or not isinstance(payload.get("decoded"), dict):
            payload = bitcoinrpc_call(
                backend,
                "gettransaction",
                [txid, True, True],
                wallet_name=wallet_name,
            )
            if tx_cache is not None:
                tx_cache[cache_key] = payload
    except AppError:
        return None
    return _bitcoinrpc_normalized_graph(txid, payload)


def _bitcoinrpc_highest_used_from_details(details, sync_state: WalletSyncState | None):
    if sync_state is None:
        return {}
    target_by_address = {
        target.get("address"): target
        for target in sync_state.targets
        if target.get("address")
    }
    highest_used = {}
    for detail in details:
        category = _bitcoinrpc_detail_category(detail)
        if category == "immature" or _bitcoinrpc_detail_is_retracted(detail):
            continue
        target = target_by_address.get(detail.get("address"))
        if target is None:
            continue
        highest_used = _merge_highest_used(highest_used, target, True)
    return highest_used


def bitcoinrpc_records_for_wallet(
    backend,
    wallet,
    addresses,
    wallet_name=None,
    imported_count=None,
    checkpoint=None,
    sync_state: WalletSyncState | None = None,
):
    wallet_name = wallet_name or bitcoinrpc_ensure_watchonly_wallet(backend, wallet)
    if imported_count is None:
        imported_count = bitcoinrpc_import_addresses(backend, wallet_name, wallet, addresses)
    transaction_checkpoint = checkpoint if not imported_count else None
    details, fetch_meta = fetch_bitcoinrpc_wallet_transactions(
        backend,
        wallet_name,
        checkpoint=transaction_checkpoint,
    )
    grouped = defaultdict(list)
    for detail in details:
        txid = detail.get("txid")
        if txid:
            grouped[txid].append(detail)
    records = []
    verbose_tx_cache = {}
    graph_unavailable_txids = []
    for txid, tx_details in sorted(
        grouped.items(),
        key=lambda item: (
            max(detail.get("blocktime") or detail.get("time") or 0 for detail in item[1]),
            item[0],
        ),
    ):
        normalized = record_from_bitcoinrpc_details(txid, tx_details, backend["name"])
        if normalized and normalized["direction"] == "outbound":
            raw_graph = _bitcoinrpc_fetch_normalized_graph(
                backend,
                wallet_name,
                txid,
                verbose_tx_cache,
            )
            if raw_graph is not None:
                tracked_scripts = set((sync_state.tracked_scripts or {}).keys()) if sync_state else set()
                normalized = record_from_bitcoinrpc_details(
                    txid,
                    tx_details,
                    backend["name"],
                    raw_graph=raw_graph,
                    tracked_scripts=tracked_scripts,
                )
            elif sync_state and (sync_state.tracked_scripts or {}):
                graph_unavailable_txids.append(str(txid).lower())
        if normalized:
            records.append(normalized)
    if graph_unavailable_txids:
        fetch_meta["bitcoinrpc_graph_unavailable_txids"] = sorted(
            set(graph_unavailable_txids)
        )
        fetch_meta.pop("bitcoinrpc_last_block", None)
    retracted_txids = set(fetch_meta.get("bitcoinrpc_retracted_txids") or [])
    if retracted_txids:
        active_txids = {str(record.get("txid") or "").lower() for record in records}
        fetch_meta["bitcoinrpc_retracted_txids"] = sorted(retracted_txids - active_txids)
        if not fetch_meta["bitcoinrpc_retracted_txids"]:
            fetch_meta.pop("bitcoinrpc_retracted_txids", None)
    return records, {
        "core_wallet": wallet_name,
        "imported_addresses": imported_count,
        "bitcoinrpc_highest_used": _bitcoinrpc_highest_used_from_details(
            details,
            sync_state,
        ),
        **fetch_meta,
        "_bitcoinrpc_verbose_tx_cache": verbose_tx_cache,
    }


def _bitcoinrpc_transaction_metadata(backend, wallet_name, txid, tx_cache=None):
    try:
        cache_key = str(txid)
        tx = tx_cache.get(cache_key) if tx_cache is not None else None
        if not isinstance(tx, dict):
            tx = bitcoinrpc_call(backend, "gettransaction", [txid, True], wallet_name=wallet_name)
            if tx_cache is not None:
                tx_cache[cache_key] = tx
    except AppError:
        return {"block_height": None, "block_time": None}
    block_time = timestamp_to_iso(tx.get("blocktime") or tx.get("time"), default=None)
    block_height = None
    block_hash = tx.get("blockhash")
    if block_hash:
        try:
            header = bitcoinrpc_call(backend, "getblockheader", [block_hash], wallet_name=None)
            block_height = header.get("height")
        except AppError:
            block_height = None
    return {"block_height": block_height, "block_time": block_time}


def bitcoinrpc_utxos_for_wallet_name(
    backend,
    wallet_name,
    addresses,
    sync_state: WalletSyncState,
    tx_cache=None,
):
    target_by_address = {
        target["address"]: target
        for target in sync_state.targets
        if target.get("address")
    }
    target_by_script = {
        target["script_pubkey"]: target
        for target in sync_state.targets
        if target.get("script_pubkey")
    }
    raw_utxos = bitcoinrpc_call(
        backend,
        "listunspent",
        [0, 9999999, addresses, True],
        wallet_name=wallet_name,
    )
    outputs = []
    metadata_cache = {}
    for raw_utxo in raw_utxos or []:
        target = target_by_address.get(raw_utxo.get("address"))
        if target is None:
            target = target_by_script.get(str(raw_utxo.get("scriptPubKey") or "").lower())
        if target is None:
            continue
        txid = raw_utxo.get("txid")
        confirmations = int(raw_utxo.get("confirmations") or 0)
        if txid not in metadata_cache:
            metadata_cache[txid] = _bitcoinrpc_transaction_metadata(
                backend,
                wallet_name,
                txid,
                tx_cache,
            )
        metadata = metadata_cache[txid]
        amount_sats = int((dec(raw_utxo.get("amount"), "0") * SATS_PER_BTC).to_integral_value())
        outputs.append(
            {
                "txid": txid,
                "vout": raw_utxo.get("vout"),
                "asset": "BTC",
                "amount_sats": amount_sats,
                "confirmation_status": "confirmed" if confirmations > 0 else "mempool",
                "confirmations": confirmations,
                "block_height": metadata.get("block_height"),
                "block_time": metadata.get("block_time"),
                "chain": sync_state.chain,
                "network": sync_state.network,
                **_target_output_metadata(target),
                "raw": {
                    "source": "bitcoinrpc_listunspent",
                    "confirmed": confirmations > 0,
                    "safe": bool(raw_utxo.get("safe", True)),
                },
            }
        )
    return outputs


def bitcoinrpc_sync_adapter(backend, wallet, sync_state: WalletSyncState):
    if silent_payments.is_silent_payment_plan(sync_state.descriptor_plan):
        return _silent_payment_sync_adapter(backend, wallet, sync_state)
    wallet_name = bitcoinrpc_ensure_watchonly_wallet(backend, wallet)
    checkpoint = _checkpoint_mapping(sync_state)
    config = json.loads(_mapping_get(wallet, "config_json", "{}") or "{}")
    birthday_ts = iso_to_unix(config.get("birthday"))
    descriptor_range_ends = None
    effective_sync_state = sync_state
    if sync_state.descriptor_plan is not None:
        imported_count, descriptor_range_ends = bitcoinrpc_import_ranged_descriptors(
            backend,
            wallet_name,
            sync_state.descriptor_plan,
            checkpoint,
            birthday_ts,
        )
        expanded_targets = _bitcoinrpc_descriptor_targets_for_range_ends(
            sync_state.descriptor_plan,
            descriptor_range_ends,
        )
        effective_sync_state = replace(
            sync_state,
            targets=expanded_targets,
            tracked_scripts={
                target["script_pubkey"]: target
                for target in expanded_targets
                if target.get("script_pubkey")
            },
        )
    else:
        addresses = [
            target["address"]
            for target in effective_sync_state.targets
            if target.get("address")
        ]
        imported_count = bitcoinrpc_import_addresses(
            backend,
            wallet_name,
            wallet,
            addresses,
            birthday_ts=birthday_ts,
        )
    addresses = [
        target["address"]
        for target in effective_sync_state.targets
        if target.get("address")
    ]
    records, meta = bitcoinrpc_records_for_wallet(
        backend,
        wallet,
        addresses,
        wallet_name=wallet_name,
        imported_count=imported_count,
        checkpoint=checkpoint,
        sync_state=effective_sync_state,
    )
    utxos = bitcoinrpc_utxos_for_wallet_name(
        backend,
        wallet_name,
        addresses,
        effective_sync_state,
        tx_cache=meta.get("_bitcoinrpc_verbose_tx_cache"),
    )
    meta["utxos"] = utxos
    meta["observer_route"] = "bitcoin_core_rpc"
    meta.pop("_bitcoinrpc_verbose_tx_cache", None)
    if sync_state.descriptor_plan is not None:
        highest_used = dict(checkpoint.get("highest_used") or {})
        for branch, index in (meta.get("bitcoinrpc_highest_used") or {}).items():
            previous = _highest_used_branch_index(highest_used, branch)
            if previous is None or int(index) > previous:
                highest_used[str(branch)] = int(index)
        for utxo in utxos:
            highest_used = _merge_highest_used(highest_used, utxo, True)
        meta["imported_descriptors"] = imported_count
    meta.pop("bitcoinrpc_highest_used", None)
    if meta.get("bitcoinrpc_last_block") or descriptor_range_ends is not None:
        next_checkpoint = dict(checkpoint)
        next_checkpoint["backend"] = _backend_identity(backend, sync_state)
        if meta.get("bitcoinrpc_last_block"):
            next_checkpoint["bitcoinrpc_last_block"] = meta["bitcoinrpc_last_block"]
        if meta.get("bitcoinrpc_pending_maturity"):
            next_checkpoint["bitcoinrpc_pending_maturity"] = True
        else:
            next_checkpoint.pop("bitcoinrpc_pending_maturity", None)
        if descriptor_range_ends is not None:
            next_checkpoint["bitcoinrpc_descriptor_range_ends"] = dict(
                sorted(descriptor_range_ends.items())
            )
            next_checkpoint["highest_used"] = dict(sorted(highest_used.items()))
        meta["freshness_checkpoint"] = next_checkpoint
    return records, meta


def read_varint(payload, offset):
    prefix = payload[offset]
    offset += 1
    if prefix < 0xFD:
        return prefix, offset
    if prefix == 0xFD:
        return int.from_bytes(payload[offset : offset + 2], "little"), offset + 2
    if prefix == 0xFE:
        return int.from_bytes(payload[offset : offset + 4], "little"), offset + 4
    return int.from_bytes(payload[offset : offset + 8], "little"), offset + 8


def decode_raw_transaction(raw_hex):
    payload = bytes.fromhex(raw_hex)
    offset = 0
    version = int.from_bytes(payload[offset : offset + 4], "little")
    offset += 4
    has_witness = len(payload) > offset + 1 and payload[offset] == 0 and payload[offset + 1] != 0
    if has_witness:
        offset += 2
    input_count, offset = read_varint(payload, offset)
    vin = []
    for _ in range(input_count):
        prev_txid = payload[offset : offset + 32][::-1].hex()
        offset += 32
        prev_vout = int.from_bytes(payload[offset : offset + 4], "little")
        offset += 4
        script_length, offset = read_varint(payload, offset)
        offset += script_length
        sequence = int.from_bytes(payload[offset : offset + 4], "little")
        offset += 4
        vin.append({"txid": prev_txid, "vout": prev_vout, "sequence": sequence})
    output_count, offset = read_varint(payload, offset)
    vout = []
    total_output_sats = Decimal("0")
    for index in range(output_count):
        value_sats = int.from_bytes(payload[offset : offset + 8], "little")
        offset += 8
        script_length, offset = read_varint(payload, offset)
        script = payload[offset : offset + script_length]
        offset += script_length
        value_decimal = dec(value_sats)
        total_output_sats += value_decimal
        vout.append(
            {
                "n": index,
                "value_sats": value_decimal,
                "value": sats_to_btc(value_decimal),
                "script_hex": script.hex(),
            }
        )
    if has_witness:
        for index in range(input_count):
            witness_count, offset = read_varint(payload, offset)
            items = []
            for _ in range(witness_count):
                item_length, offset = read_varint(payload, offset)
                items.append(payload[offset : offset + item_length].hex())
                offset += item_length
            vin[index]["witness"] = items
    locktime = int.from_bytes(payload[offset : offset + 4], "little")
    return {
        "version": version,
        "locktime": locktime,
        "vin": vin,
        "vout": vout,
        "total_output_sats": total_output_sats,
        "raw_hex": raw_hex,
    }


def block_header_timestamp(header_hex):
    header = bytes.fromhex(header_hex)
    return int.from_bytes(header[68:72], "little")


def _positive_electrum_height(value):
    if value in (None, "", 0, "0"):
        return None
    try:
        height = int(value)
    except (TypeError, ValueError):
        return None
    return height if height > 0 else None


def _backend_time_to_iso(value, default=UNKNOWN_OCCURRED_AT):
    parsed = parse_iso_datetime_or_none(value)
    if parsed is not None:
        return (
            parsed.replace(microsecond=0)
            .isoformat()
            .replace("+00:00", "Z")
        )
    return timestamp_to_iso(value, default=default)


def _history_explicit_time_iso(history):
    for key in ("confirmed_at", "timestamp", "time", "blocktime", "height"):
        value = history.get(key)
        parsed = parse_iso_datetime_or_none(value)
        if parsed is not None:
            return (
                parsed.replace(microsecond=0)
                .isoformat()
                .replace("+00:00", "Z")
            )
        if key != "height" and value not in (None, "", 0, "0"):
            try:
                return timestamp_to_iso(value)
            except (TypeError, ValueError, OSError, OverflowError):
                continue
    return None


def _history_needs_recheck(history):
    if _positive_electrum_height(history.get("height")) is not None:
        return False
    if _history_explicit_time_iso(history) is not None:
        return False
    return True


def _history_occurred_at(history, height_to_timestamp):
    height = _positive_electrum_height(history.get("height"))
    if height is not None:
        return timestamp_to_iso(height_to_timestamp(height))
    return _history_explicit_time_iso(history) or UNKNOWN_OCCURRED_AT


def _history_sort_key(txid, history):
    height = _positive_electrum_height(history.get("height"))
    if height is not None:
        return (0, height, txid)
    return (1, _history_explicit_time_iso(history) or UNKNOWN_OCCURRED_AT, txid)


def electrum_output_at_index(tx, index):
    outputs = tx.get("vout") or []
    if index is None or index < 0 or index >= len(outputs):
        return None
    return outputs[index]


def _electrum_output_value_sats(output):
    if not isinstance(output, dict):
        return None
    value_sats = output.get("value_sats")
    if value_sats is None:
        value = output.get("value")
        if value is None:
            return None
        if isinstance(value, bool):
            return None
        if isinstance(value, int):
            return dec(value, "0").to_integral_value()
        if isinstance(value, str) and "." not in value:
            return dec(value, "0").to_integral_value()
        return (dec(value, "0") * SATS_PER_BTC).to_integral_value()
    return dec(value_sats, "0").to_integral_value()


def _electrum_graph_output_payload(output):
    if not isinstance(output, dict):
        return None
    script = output.get("scriptpubkey") or output.get("script_hex")
    value_sats = _electrum_output_value_sats(output)
    if script is None or value_sats is None:
        return None
    return {
        "scriptpubkey": script,
        "value": int(value_sats),
    }


def _normalize_electrum_bitcoin_graph_for_storage(tx, tx_lookup):
    """Persist the public graph shape that local consumers expect.

    Fulcrum/Electrum returns raw transaction hex, so the current transaction
    decodes to input outpoints plus outputs shaped as ``script_hex`` /
    ``value_sats``. Fetching each distinct previous tx once and normalizing
    current outputs to ``scriptpubkey`` / integer-sat ``value`` makes the stored
    row usable for local-only graph consumers after sync, matching Esplora's
    inline-prevout convention.
    """
    if not isinstance(tx, dict):
        return tx
    tx[ELECTRUM_STORED_GRAPH_MARKER] = {
        "kind": "bitcoin_electrum",
        "version": ELECTRUM_STORED_GRAPH_VERSION,
    }
    vout = tx.get("vout")
    if isinstance(vout, list):
        for entry in vout:
            payload = _electrum_graph_output_payload(entry)
            if payload is not None:
                entry.update(payload)
    vin = tx.get("vin")
    if not isinstance(vin, list):
        return tx
    for entry in vin:
        if not isinstance(entry, dict):
            continue
        if isinstance(entry.get("prevout"), dict):
            continue
        prev_txid = entry.get("txid")
        prev_index = entry.get("vout")
        if prev_txid is None or prev_index is None:
            continue
        prev_tx = tx_lookup(prev_txid)
        prevout = electrum_output_at_index(prev_tx, prev_index)
        payload = _electrum_graph_output_payload(prevout)
        if payload is not None:
            entry["prevout"] = payload
    return tx


def _electrum_utxo_status(raw_utxo, header_timestamps):
    height = _positive_electrum_height(raw_utxo.get("height"))
    if height is None:
        block_time = _history_explicit_time_iso(raw_utxo)
        return {
            "confirmed": block_time is not None,
            "block_height": None,
            "block_time": block_time,
        }
    return {
        "confirmed": True,
        "block_height": height,
        "block_time": timestamp_to_iso(header_timestamps.get(height), default=None),
    }


def _electrum_tip_height(client):
    try:
        header = client.call("blockchain.headers.subscribe", [])
    except AppError:
        return None
    if not isinstance(header, dict):
        return None
    try:
        return int(header.get("height"))
    except (TypeError, ValueError):
        return None


def _electrum_bitcoin_utxo_record(raw_utxo, target, sync_state, header_timestamps, tip_height=None):
    status = _electrum_utxo_status(raw_utxo, header_timestamps)
    block_height = status["block_height"]
    return {
        "txid": raw_utxo.get("tx_hash"),
        "vout": raw_utxo.get("tx_pos"),
        "asset": "BTC",
        "amount_sats": int(raw_utxo.get("value") or 0),
        "confirmation_status": "confirmed" if status["confirmed"] else "mempool",
        "confirmations": _confirmations_from_heights(block_height, tip_height),
        "block_height": block_height,
        "block_time": status["block_time"],
        "chain": sync_state.chain,
        "network": sync_state.network,
        **_target_output_metadata(target),
        "raw": {
            "source": "electrum_scripthash_listunspent",
            "confirmed": bool(status["confirmed"]),
        },
    }


def compatibility_electrum_utxos_for_wallet(backend, sync_state: WalletSyncState):
    outputs = []
    batch_size = backend_batch_size(backend)
    header_timestamps = {}
    with _electrum_client_context(backend) as client:
        tip_height = _electrum_tip_height(client)
        scripthashes = [scriptpubkey_scripthash(target["script_pubkey"]) for target in sync_state.targets]
        target_by_scripthash = dict(zip(scripthashes, sync_state.targets))
        raw_results = electrum_call_many(
            client,
            [("blockchain.scripthash.listunspent", [scripthash]) for scripthash in scripthashes],
            batch_size=batch_size,
        )
        raw_by_target = []
        heights = set()
        for scripthash, raw_utxos in zip(scripthashes, raw_results):
            target = target_by_scripthash[scripthash]
            for raw_utxo in raw_utxos or []:
                raw_by_target.append((target, raw_utxo))
                height = _positive_electrum_height(raw_utxo.get("height"))
                if height is not None:
                    heights.add(height)
        if heights:
            header_hexes = electrum_call_many(
                client,
                [("blockchain.block.header", [height]) for height in sorted(heights)],
                batch_size=batch_size,
            )
            for height, header_hex in zip(sorted(heights), header_hexes):
                header_timestamps[height] = block_header_timestamp(header_hex)
        liquid_tx_cache = {}

        def liquid_tx(txid):
            if txid not in liquid_tx_cache:
                raw_tx = client.call("blockchain.transaction.get", [txid])
                liquid_tx_cache[txid] = decode_liquid_transaction(raw_tx)
            return liquid_tx_cache[txid]

        for target, raw_utxo in raw_by_target:
            if sync_state.chain == "liquid":
                status = _electrum_utxo_status(raw_utxo, header_timestamps)
                outputs.append(
                    _liquid_utxo_record_from_output(
                        raw_utxo.get("tx_hash"),
                        raw_utxo.get("tx_pos"),
                        {
                            "confirmed": bool(status["confirmed"]),
                            "block_height": status["block_height"],
                            "block_time": status["block_time"],
                        },
                        liquid_tx(raw_utxo.get("tx_hash")),
                        target,
                        sync_state,
                        source="liquid_electrum_scripthash_listunspent",
                        tip_height=tip_height,
                    )
                )
                continue
            outputs.append(
                _electrum_bitcoin_utxo_record(
                    raw_utxo,
                    target,
                    sync_state,
                    header_timestamps,
                    tip_height=tip_height,
                )
            )
    return outputs


def record_from_electrum_tx(txid, tx, height, tracked_scripts, backend_name, tx_lookup):
    stored_graph = json_ready(tx)
    _normalize_electrum_bitcoin_graph_for_storage(stored_graph, tx_lookup)
    occurred_at = _backend_time_to_iso(height)
    confirmed_at = None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at
    stored_graph["status"] = {"confirmed": confirmed_at is not None}
    return _record_from_bitcoin_graph(
        stored_graph,
        tracked_scripts,
        backend_name,
        txid=txid,
        occurred_at=occurred_at,
        confirmed_at=confirmed_at,
    )


def compatibility_electrum_records_for_wallet(backend, sync_state: WalletSyncState):
    transactions = {}
    stored_transactions = (
        dict(sync_state.history_cache or {}) if sync_state.chain == "bitcoin" else {}
    )

    def stored_transaction(txid):
        stored = stored_transactions.get(str(txid))
        if isinstance(stored, str):
            try:
                stored = json.loads(stored)
            except ValueError:
                return None
        if not isinstance(stored, dict):
            return None
        marker = stored.get(ELECTRUM_STORED_GRAPH_MARKER)
        if not isinstance(marker, dict):
            return None
        if marker.get("kind") != "bitcoin_electrum":
            return None
        try:
            marker_version = int(marker.get("version") or 0)
        except (TypeError, ValueError):
            return None
        if marker_version != ELECTRUM_STORED_GRAPH_VERSION:
            return None
        if str(stored.get("txid") or "").lower() != str(txid).lower():
            return None
        if not isinstance(stored.get("vin"), list) or not isinstance(
            stored.get("vout"), list
        ):
            return None
        return stored
    records = []
    batch_size = backend_batch_size(backend)
    tracked_scripts = (
        set(sync_state.tracked_scripts)
        if sync_state.chain == "bitcoin"
        else sync_state.tracked_scripts
    )
    checkpoint = _checkpoint_mapping(sync_state)
    previous_statuses = checkpoint.get("electrum_scripthash_statuses") or {}
    previous_dirty = set(checkpoint.get("electrum_dirty_scripthashes") or [])
    previous_history_entries = checkpoint.get("electrum_history_entries") or {}
    stored_graph_current = (
        int(checkpoint.get("electrum_stored_graph_version") or 0)
        >= ELECTRUM_STORED_GRAPH_VERSION
    )
    highest_used = dict(checkpoint.get("highest_used") or {})
    header_timestamps = {
        int(height): timestamp
        for height, timestamp in (checkpoint.get("electrum_headers") or {}).items()
        if str(height).isdigit()
    }
    next_statuses = {}
    next_history_entries = {}
    dirty_scripthashes = set()
    unchanged_scripts = 0
    changed_scripts = 0
    header_cache_hits = 0
    with _electrum_client_context(backend) as client:
        histories = []
        target_by_scripthash = {}
        scripthashes = []
        for target in sync_state.targets:
            scripthash = scriptpubkey_scripthash(target["script_pubkey"])
            target_by_scripthash[scripthash] = target
            scripthashes.append(scripthash)
        statuses = electrum_call_many(
            client,
            [("blockchain.scripthash.subscribe", [scripthash]) for scripthash in scripthashes],
            batch_size=batch_size,
        )
        history_scripthashes = []
        total_scripts = len(scripthashes)
        for status_index, (scripthash, status) in enumerate(
            zip(scripthashes, statuses),
            start=1,
        ):
            next_statuses[scripthash] = status
            target = target_by_scripthash[scripthash]
            highest_used = _merge_highest_used(highest_used, target, status is not None)
            if status is None and previous_statuses.get(scripthash) is None:
                next_history_entries[scripthash] = {}
                unchanged_scripts += 1
                if status_index % max(1, batch_size) == 0 or status_index == total_scripts:
                    _emit_backend_progress(
                        "backend_fetch",
                        target_count=total_scripts,
                        targets_checked=status_index,
                        scripts_changed=len(history_scripthashes),
                        scripts_unchanged=unchanged_scripts,
                    )
                continue
            if (
                stored_graph_current
                and status == previous_statuses.get(scripthash)
                and scripthash not in previous_dirty
            ):
                next_history_entries[scripthash] = dict(
                    previous_history_entries.get(scripthash) or {}
                )
                unchanged_scripts += 1
                if status_index % max(1, batch_size) == 0 or status_index == total_scripts:
                    _emit_backend_progress(
                        "backend_fetch",
                        target_count=total_scripts,
                        targets_checked=status_index,
                        scripts_changed=len(history_scripthashes),
                        scripts_unchanged=unchanged_scripts,
                    )
                continue
            history_scripthashes.append(scripthash)
            if status_index % max(1, batch_size) == 0 or status_index == total_scripts:
                _emit_backend_progress(
                    "backend_fetch",
                    target_count=total_scripts,
                    targets_checked=status_index,
                    scripts_changed=len(history_scripthashes),
                    scripts_unchanged=unchanged_scripts,
                )
        if history_scripthashes:
            changed_scripts = len(history_scripthashes)
            fetched_histories = electrum_call_many(
                client,
                [
                    ("blockchain.scripthash.get_history", [scripthash])
                    for scripthash in history_scripthashes
                ],
                batch_size=batch_size,
            )
            for history_index, (scripthash, history) in enumerate(
                zip(history_scripthashes, fetched_histories),
                start=1,
            ):
                normalized_history = history or []
                current_entries = {
                    str(item["tx_hash"]): json_ready(item)
                    for item in normalized_history
                    if isinstance(item, dict) and item.get("tx_hash")
                }
                prior_entries = previous_history_entries.get(scripthash) or {}
                histories.extend(
                    item
                    for item in normalized_history
                    if isinstance(item, dict)
                    and item.get("tx_hash")
                    and prior_entries.get(str(item["tx_hash"])) != json_ready(item)
                )
                next_history_entries[scripthash] = current_entries
                if any(_history_needs_recheck(item) for item in normalized_history):
                    dirty_scripthashes.add(scripthash)
                if history_index % max(1, batch_size) == 0 or history_index == changed_scripts:
                    _emit_backend_progress(
                        "backend_fetch",
                        target_count=changed_scripts,
                        targets_checked=history_index,
                        known_txids=len(
                            {
                                item.get("tx_hash")
                                for item in histories
                                if isinstance(item, dict) and item.get("tx_hash")
                            }
                        ),
                    )

        def lookup(txid):
            if txid not in transactions:
                cached = stored_transaction(txid)
                if cached is not None:
                    transactions[txid] = cached
                else:
                    raw_tx = client.call("blockchain.transaction.get", [txid])
                    if sync_state.chain == "liquid":
                        transactions[txid] = {
                            "raw_hex": raw_tx,
                            "decoded": decode_liquid_transaction(raw_tx),
                        }
                    else:
                        transactions[txid] = decode_raw_transaction(raw_tx)
            return transactions[txid]

        def height_to_timestamp(height):
            normalized_height = _positive_electrum_height(height)
            if normalized_height is None:
                return None
            if normalized_height not in header_timestamps:
                header_hex = client.call("blockchain.block.header", [normalized_height])
                header_timestamps[normalized_height] = block_header_timestamp(header_hex)
            return header_timestamps[normalized_height]

        txids = {}
        for history in histories:
            txids[history["tx_hash"]] = history
        ordered_histories = sorted(
            txids.items(),
            key=lambda item: _history_sort_key(item[0], item[1]),
        )
        ordered_txids = [txid for txid, _ in ordered_histories]
        for txid in ordered_txids:
            cached = stored_transaction(txid)
            if cached is not None:
                transactions[txid] = cached
        missing_ordered_txids = [txid for txid in ordered_txids if txid not in transactions]
        fetched_transaction_count = len(missing_ordered_txids)
        if missing_ordered_txids:
            raw_transactions = electrum_call_many(
                client,
                [("blockchain.transaction.get", [txid]) for txid in missing_ordered_txids],
                batch_size=batch_size,
            )
            for txid, raw_tx in zip(missing_ordered_txids, raw_transactions):
                if sync_state.chain == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
        seen_txids = set(transactions)
        prev_txids = []
        for txid in ordered_txids:
            current_tx = transactions[txid]["decoded"] if sync_state.chain == "liquid" else transactions[txid]
            vins = current_tx.vin if sync_state.chain == "liquid" else current_tx.get("vin", [])
            for vin in vins:
                prev_txid = liquid_input_txid(vin) if sync_state.chain == "liquid" else vin.get("txid")
                if not prev_txid or prev_txid in seen_txids:
                    continue
                cached = stored_transaction(prev_txid)
                if cached is not None:
                    transactions[prev_txid] = cached
                    seen_txids.add(prev_txid)
                    continue
                seen_txids.add(prev_txid)
                prev_txids.append(prev_txid)
        if prev_txids:
            fetched_transaction_count += len(prev_txids)
            raw_prev_transactions = electrum_call_many(
                client,
                [("blockchain.transaction.get", [txid]) for txid in prev_txids],
                batch_size=batch_size,
            )
            for txid, raw_tx in zip(prev_txids, raw_prev_transactions):
                if sync_state.chain == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
        heights = sorted(
            {
                height
                for history in txids.values()
                for height in [_positive_electrum_height(history.get("height"))]
                if height is not None
            }
        )
        if heights:
            header_cache_hits = len([height for height in heights if height in header_timestamps])
            missing_heights = [height for height in heights if height not in header_timestamps]
            if missing_heights:
                header_hexes = electrum_call_many(
                    client,
                    [("blockchain.block.header", [height]) for height in missing_heights],
                    batch_size=batch_size,
                )
                for height, header_hex in zip(missing_heights, header_hexes):
                    header_timestamps[height] = block_header_timestamp(header_hex)
        for tx_index, (txid, history) in enumerate(ordered_histories, start=1):
            occurred_at = _history_occurred_at(history, height_to_timestamp)
            if sync_state.chain == "liquid":
                current_tx = lookup(txid)
                records.extend(
                    record_components_from_liquid_tx(
                        txid,
                        occurred_at,
                        current_tx["decoded"],
                        sync_state.descriptor_plan,
                        tracked_scripts,
                        backend["name"],
                        sync_state.policy_asset_id,
                        lambda prev_txid: lookup(prev_txid)["decoded"],
                        {
                            "history": history,
                            "raw_hex": current_tx["raw_hex"],
                            "status": {
                                "confirmed": occurred_at != UNKNOWN_OCCURRED_AT
                            },
                        },
                        confirmed_at=None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at,
                        network=sync_state.network,
                    )
                )
            else:
                tx = lookup(txid)
                _normalize_electrum_bitcoin_graph_for_storage(tx, lookup)
                normalized = record_from_electrum_tx(
                    txid,
                    tx,
                    occurred_at,
                    tracked_scripts,
                    backend["name"],
                    lookup,
                )
                if normalized:
                    records.append(normalized)
            if tx_index % 100 == 0 or tx_index == len(ordered_histories):
                _emit_backend_progress(
                    "decode_enrich",
                    transactions_seen=tx_index,
                    transactions_total=len(ordered_histories),
                    records=len(records),
                )
    # The write-only per-wallet txid ledger is retired; purge it from
    # checkpoints written by earlier versions.
    checkpoint.pop("electrum_known_txids", None)
    checkpoint.update(
        {
            "backend": _backend_identity(backend, sync_state),
            "electrum_dirty_scripthashes": sorted(dirty_scripthashes),
            "electrum_headers": {
                str(height): header_timestamps[height] for height in sorted(header_timestamps)
            },
            "electrum_stored_graph_version": ELECTRUM_STORED_GRAPH_VERSION,
            "electrum_history_entries": dict(sorted(next_history_entries.items())),
            "electrum_scripthash_statuses": dict(sorted(next_statuses.items())),
            "highest_used": dict(sorted(highest_used.items())),
        }
    )
    return records, {
        "freshness_checkpoint": checkpoint,
        "scripts_changed": changed_scripts,
        "scripts_unchanged": unchanged_scripts,
        "header_cache_hits": header_cache_hits,
        "transactions_fetched": fetched_transaction_count,
    }


def compatibility_electrum_sync_adapter(backend, wallet, sync_state):
    if silent_payments.is_silent_payment_plan(sync_state.descriptor_plan):
        return _silent_payment_sync_adapter(backend, wallet, sync_state)
    records, meta = compatibility_electrum_records_for_wallet(backend, sync_state)
    if _skip_unchanged_utxo_refresh(meta, sync_state):
        meta["utxos_skipped_unchanged"] = True
    else:
        meta["utxos"] = compatibility_electrum_utxos_for_wallet(backend, sync_state)
    return records, meta


def custom_sync_adapter(backend, wallet, sync_state):
    if silent_payments.is_silent_payment_plan(sync_state.descriptor_plan):
        return _silent_payment_sync_adapter(backend, wallet, sync_state)
    raise AppError(
        "Custom backends can only sync wallets through an explicit Silent Payments scanner",
        code="validation",
        retryable=False,
    )


COMPATIBILITY_SYNC_BACKEND_ADAPTERS = MappingProxyType(
    {
        "esplora": compatibility_esplora_sync_adapter,
        "electrum": compatibility_electrum_sync_adapter,
    }
)


SYNC_BACKEND_ADAPTERS = MappingProxyType(
    {
        **COMPATIBILITY_SYNC_BACKEND_ADAPTERS,
        "bitcoinrpc": bitcoinrpc_sync_adapter,
        "custom": custom_sync_adapter,
    }
)


__all__ = [
    "SYNC_BACKEND_ADAPTERS",
    "address_to_scriptpubkey",
    "bitcoinrpc_sync_adapter",
    "bitcoinrpc_utxos_for_wallet_name",
    "custom_sync_adapter",
    "decode_raw_transaction",
    "compatibility_electrum_sync_adapter",
    "compatibility_electrum_utxos_for_wallet",
    "compatibility_esplora_sync_adapter",
    "compatibility_esplora_utxos_for_wallet",
    "fetch_esplora_transaction",
    "fetch_transaction_legs",
    "resolve_wallet_sync_targets",
    "resolve_verify_backend",
    "verify_session",
    "sync_target_from_address",
    "sync_target_from_derived",
]

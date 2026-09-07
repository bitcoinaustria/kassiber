"""Explicit, bounded reference acquisition. No wallet imports or public fallback.

Plans are recomputed before any egress. Data is sanitized before its atomic
profile-scoped save. A backend attests observations; it does not grant ownership.
"""
from __future__ import annotations

from collections import deque
from contextlib import nullcontext
import json
import os
import re
import time
from urllib import error as urlerror, request as urlrequest, parse as urlparse

from ..backends import get_db_backend, backend_timeout
from ..errors import AppError
from ..time_utils import now_iso
from ..wallet_descriptors import normalize_network
from .chain_analysis import build_index
from .chain_analysis.query import resolve_subject
from .chain_analysis_cases import arguments, atomic, canonical, digest, invalid, text_value, validate_domain
from . import sync_backends as transport
from . import transaction_graph as graph
from .onchain import input_outpoint, normalized_script_hex


# Consensus constants from Bitcoin Core v31 kernel/chainparams.cpp and Elements
# kernel/chainparams.cpp. Custom Liquid chains require an explicit genesis pin.
GENESIS = {
    ("bitcoin", "main"): "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    ("bitcoin", "test"): "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
    ("bitcoin", "signet"): "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
    ("bitcoin", "regtest"): "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
    ("liquid", "main"): "1466275836220db2944ca059a3a10ef6fd2ea684b0688d2c379296888a206003",
}
TXID = re.compile(r"^[a-f0-9]{64}$")
MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_TOTAL_BYTES = 64 * 1024 * 1024


def _txid(value):
    return isinstance(value, str) and TXID.fullmatch(value) is not None


def _routing_identity(backend):
    def endpoint(value, *, path=False):
        if not value:
            return None
        try:
            parsed = urlparse.urlsplit(value)
            return [parsed.scheme.lower(), parsed.hostname, parsed.port, parsed.path if path else None]
        except (ValueError, TypeError):
            invalid("Invalid backend routing configuration")
    # Userinfo, query credentials and tokens do not define a new destination.
    # Bind routing/TLS choices without exposing either their values or stable
    # credential fingerprints in the public plan.
    return [endpoint(backend.get("url"), path=True), endpoint(transport._backend_proxy_url(backend)), transport.backend_value(backend, "certificate"), transport.backend_value(backend, "insecure"), backend_timeout(backend)]


def _prepare_acquisition(conn, profile_id, args):
    arguments(args, ("backend", "subject", "chain", "network", "direction", "depth", "max_transactions", "genesis_hash"), ("backend", "subject", "chain", "network", "direction"))
    validate_domain(args["chain"], args["network"])
    value = {**args, "backend": text_value(args["backend"], "backend", 128).lower(), "subject": text_value(args["subject"], "subject", 256), "depth": args.get("depth", 3), "max_transactions": args.get("max_transactions", 50)}
    if value["direction"] not in {"backward", "forward", "both"}:
        invalid("Invalid acquisition direction")
    for field, maximum in (("depth", 10), ("max_transactions", 200)):
        if type(value[field]) is not int or not 1 <= value[field] <= maximum:
            invalid(f"{field} must be between 1 and {maximum}")
    consensus_genesis = GENESIS.get((value["chain"], value["network"]))
    if consensus_genesis is not None and value.get("genesis_hash") not in (None, consensus_genesis):
        invalid("The supplied genesis hash conflicts with the selected standard network")
    expected_genesis = consensus_genesis or value.get("genesis_hash")
    if not _txid(expected_genesis):
        invalid("This custom network requires an explicit genesis_hash pin")
    value["genesis_hash"] = expected_genesis
    backend = get_db_backend(conn, value["backend"])
    kind = backend["kind"]
    if kind not in {"bitcoinrpc", "esplora", "liquid-esplora", "electrum"}:
        invalid("Selected backend cannot supply chain observations", "capability_unavailable")
    if backend.get("chain") != value["chain"] or normalize_network(value["chain"], backend.get("network")) != normalize_network(value["chain"], value["network"]):
        invalid("Selected backend belongs to a different chain or network")
    if kind == "bitcoinrpc" and value["chain"] != "bitcoin":
        invalid("The Bitcoin Core observer supports Bitcoin only", "capability_unavailable")
    from .book_network import resolve_book_environment, require_chain_domain
    book = resolve_book_environment(conn, profile_id)
    domain = require_chain_domain(conn, profile_id, value["chain"], value["network"], operation="acquisition")
    if backend.get("chain_instance_id") and backend["chain_instance_id"] != domain["chain_instance_id"]:
        invalid("This backend is assigned to a different local chain instance")
    index = build_index(conn, profile_id)
    subject = value["subject"].lower()
    bare = subject.split(":tx:", 1)[-1].split(":out:", 1)[-1].split(":", 1)[0]
    if _txid(bare) and (subject == bare or subject.startswith(f"{value['chain']}:{normalize_network(value['chain'], value['network'])}:") or re.fullmatch(r"[a-f0-9]{64}:[0-9]+", subject)):
        seeds = [bare]
    else:
        ids = resolve_subject(index, value["subject"], value)
        seeds = sorted({index.nodes[node].get("txid") for node in ids if _txid(index.nodes[node].get("txid"))})
    if not seeds:
        invalid("Acquisition needs a txid, outpoint or a locally resolved wallet/address", "not_found")
    if len(seeds) > value["max_transactions"]:
        invalid("Subject has more transactions than this acquisition budget; narrow the subject")
    safe_backend = {key: backend.get(key) for key in ("name", "kind", "chain", "network")}
    result = {"args": value, "snapshot_id": index.snapshot_id, "backend": safe_backend, "effects": {"egresses": True, "max_transactions": value["max_transactions"], "max_requests": 6 + 2 * value["max_transactions"], "scheduling_deadline_seconds": 45, "request_timeout_seconds": 8, "max_response_bytes": MAX_RESPONSE_BYTES, "max_total_response_bytes": MAX_TOTAL_BYTES}, "limitations": ["Reference observations do not grant ownership or custody.", "Only the chosen backend is contacted; missing or pruned history remains a frontier.", "Confirmation and spend status are observations at acquisition time.", "The 45 second deadline stops new requests and response chunks; a pending network read has an inactivity timeout of at most 8 seconds."]}
    if kind == "electrum":
        result["limitations"].append("Electrum supplies ancestors, but no universal historical spender index or confirmation proof in this query.")
    if kind == "bitcoinrpc":
        result["limitations"].append("Historical forward expansion requires a synchronized Core 31 txospenderindex; arbitrary raw transaction reads may require txindex.")
    result["book_environment"] = {"environment_id": book["environment_id"], "revision": book["revision"], "chain_instance_id": book.get("chain_instance_id")}
    if book.get("chain_instance_id"):
        result["limitations"].append("Approval assigns the selected source to this local chain instance. Its genesis hash verifies the network, not the identity of a particular local chain history.")
    # A configuration change invalidates consent without exposing a hash of
    # secrets. Backend CRUD maintains updated_at; bind every safe plan field.
    revision = conn.execute("SELECT updated_at FROM backends WHERE name=?", (backend["name"],)).fetchone()[0]
    result["plan_id"] = digest([profile_id, result, revision, seeds, _routing_identity(backend)])
    return result, backend, seeds


def plan_acquisition(conn, profile_id, args):
    plan, _backend, _seeds = _prepare_acquisition(conn, profile_id, args)
    return plan


class _Budget:
    def __init__(self, maximum, seconds=45):
        self.maximum, self.count = maximum, 0
        self.bytes_read = 0
        self.deadline = time.monotonic() + seconds

    def request(self):
        # This boundary covers HTTP requests and the initial Electrum handshake,
        # before either transport resolves or connects to even a loopback host.
        if str(os.environ.get("KASSIBER_NO_EGRESS") or "").strip().lower() in {"1", "true", "yes", "on"}:
            raise AppError(
                "Outbound chain acquisition is disabled by KASSIBER_NO_EGRESS",
                code="network_egress_disabled",
                retryable=False,
            )
        if self.count >= self.maximum or self.bytes_read >= MAX_TOTAL_BYTES or time.monotonic() >= self.deadline:
            invalid("Acquisition budget reached", "acquisition_budget")
        self.count += 1
        return max(0.05, min(8, self.deadline - time.monotonic()))

    def received(self, size):
        self.bytes_read += size
        if self.bytes_read > MAX_TOTAL_BYTES:
            invalid("Acquisition total response byte budget reached", "acquisition_budget")


class _BoundedElectrumClient(transport.ElectrumClient):
    """Acquisition-only framing bound, including the initial version handshake."""
    def __init__(self, backend, budget):
        super().__init__(backend)
        self.budget = budget
        self._response_buffer = b""

    def _call_locked(self, method, params=None):
        if self.socket is None:
            invalid("Electrum connection is unavailable", "history_unavailable")
        remaining = self.budget.deadline - time.monotonic()
        if remaining <= 0:
            invalid("Electrum request exceeded its deadline", "acquisition_budget")
        self.socket.settimeout(min(8, remaining))
        self.request_id += 1
        payload = canonical({"jsonrpc": "2.0", "id": self.request_id, "method": method, "params": params or []}).encode() + b"\n"
        self.socket.sendall(payload)
        transport.get_egress_ledger().record(subsystem="sync", host=self._egress_host, port=self._egress_port, scheme="electrum", operation="socket.write", method=method, bytes_out=len(payload))
        for _ in range(128):
            while b"\n" not in self._response_buffer:
                remaining = self.budget.deadline - time.monotonic()
                if remaining <= 0:
                    invalid("Electrum response exceeded its deadline", "acquisition_budget")
                self.socket.settimeout(min(8, remaining))
                chunk = self.socket.recv(min(65536, MAX_RESPONSE_BYTES + 1 - len(self._response_buffer)))
                if not chunk:
                    invalid("Electrum connection closed before its response", "history_unavailable")
                self.budget.received(len(chunk))
                self._response_buffer += chunk
                if len(self._response_buffer) > MAX_RESPONSE_BYTES:
                    invalid("Electrum response exceeds the byte limit", "invalid_observation")
            line, self._response_buffer = self._response_buffer.split(b"\n", 1)
            message = json.loads(line)
            if not isinstance(message, dict):
                invalid("Malformed Electrum response", "invalid_observation")
            if message.get("id") != self.request_id:
                continue
            if message.get("error"):
                invalid("Electrum could not supply this observation", "history_unavailable")
            return message.get("result")
        invalid("Electrum response notification limit reached", "acquisition_budget")


class _Reader:
    def __init__(self, backend, budget, client=None):
        self.backend, self.budget, self.client = backend, budget, client

    def rpc(self, method, params=None):
        request_id = f"chain-analysis-{method}"
        payload = {"jsonrpc": "1.0", "id": request_id, "method": method, "params": params or []}
        response = self._http(transport.bitcoinrpc_url(self.backend), headers=transport.bitcoinrpc_auth_headers(self.backend), payload=payload)
        if not isinstance(response, dict) or response.get("id") != request_id:
            invalid("Invalid Bitcoin Core response identity", "invalid_observation")
        if response.get("error"):
            invalid("Bitcoin Core could not supply the requested observation", "history_unavailable")
        return response.get("result")

    def electrum(self, method, params=None):
        timeout = self.budget.request()
        if self.client.socket is not None:
            self.client.socket.settimeout(timeout)
        return self.client.call(method, params)

    def http(self, path, *, text=False):
        return self._http(str(self.backend["url"]).rstrip("/") + path, headers=graph._esplora_auth_headers(self.backend), text=text)

    def _http(self, url, *, headers, text=False, payload=None):
        timeout = self.budget.request()
        request = urlrequest.Request(url, data=canonical(payload).encode() if payload is not None else None, headers={"Accept": "text/plain" if text else "application/json", "Content-Type": "application/json", "User-Agent": "Kassiber-chain-analysis", **(headers or {})}, method="POST" if payload is not None else "GET")
        try:
            response = transport.urlopen_with_proxy(request, url, timeout, proxy_url=transport._backend_proxy_url(self.backend), source_label="backend", ssl_context=transport._backend_ssl_context(self.backend), follow_redirects=False, raise_http_errors=False)
        except urlerror.HTTPError as error:
            # urllib's HTTPError owns a live response stream. All statuses,
            # including refused redirects, must close it and charge error-body
            # reads to the same deadline and byte budgets as successful reads.
            response = error
        with response as response:
            status = getattr(response, "status", None)
            chunks, size = [], 0
            read = getattr(response, "read1", response.read)
            while True:
                if time.monotonic() >= self.budget.deadline:
                    invalid("Acquisition response exceeded its scheduling deadline", "acquisition_budget")
                chunk = read(min(65536, MAX_RESPONSE_BYTES + 1 - size))
                if not chunk:
                    break
                chunks.append(chunk)
                size += len(chunk)
                self.budget.received(len(chunk))
                if size > MAX_RESPONSE_BYTES:
                    invalid("Acquisition response exceeds the byte limit", "invalid_observation")
        # Core's typed RPC envelope may arrive with HTTP400/404/500. Other
        # errors are categorical; never include server prose in the result.
        if isinstance(status, int) and status >= 300 and not (payload is not None and status in {400, 404, 500}):
            invalid("Backend could not supply the requested observation", "history_unavailable")
        try:
            body = b"".join(chunks).decode("utf-8")
            return body if text else json.loads(body)
        except (ValueError, RecursionError) as error:
            raise AppError("Acquisition returned malformed response data", code="invalid_observation") from error

    def genesis(self):
        if self.backend["kind"] == "bitcoinrpc":
            return self.rpc("getblockhash", [0])
        if self.client is not None:
            features = self.electrum("server.features")
            return features.get("genesis_hash") if isinstance(features, dict) else None
        return self.http("/block-height/0", text=True).strip()

    def transaction(self, txid, chain):
        status = {}
        feature_raw = None
        if self.backend["kind"] == "bitcoinrpc":
            # Verbosity 2 includes prevout values/scripts when Core has undo
            # data. Verbosity 1 silently discarded these available facts and
            # made input characteristics and entropy unnecessarily unknown.
            decoded = self.rpc("getrawtransaction", [txid, 2])
            if not isinstance(decoded, dict) or decoded.get("txid") != txid:
                invalid("Backend returned a different transaction", "invalid_observation")
            confirmations = decoded.get("confirmations")
            status = {"confirmed": type(confirmations) is int and confirmations > 0, "conflicted": type(confirmations) is int and confirmations < 0}
            if _txid(decoded.get("blockhash")):
                status["block_hash"] = decoded["blockhash"]
            raw = graph._bitcoinrpc_decoded_to_graph_raw(decoded)
            for original, target in zip(decoded.get("vin", ()), raw["vin"]):
                if isinstance(original, dict) and isinstance(original.get("coinbase"), str) and normalized_script_hex(original["coinbase"]):
                    target["is_coinbase"] = True
                if isinstance(original, dict):
                    for key in ("scriptSig", "txinwitness"):
                        if key in original:
                            target[key] = original[key]
        elif self.client is not None:
            raw_hex = self.electrum("blockchain.transaction.get", [txid])
            if not isinstance(raw_hex, str) or len(raw_hex) > 8_000_000:
                invalid("Invalid transaction bytes", "invalid_observation")
            if chain == "bitcoin":
                from embit.transaction import Transaction
                tx = Transaction.parse(bytes.fromhex(raw_hex))
                actual = tx.txid().hex()
                raw = graph._bitcoin_electrum_decoded_to_graph_raw(txid, transport.decode_raw_transaction(raw_hex), raw_hex)
                from .chain_analysis.features import feature_raw_from_transaction
                feature_raw = feature_raw_from_transaction(tx)
            else:
                tx = transport.decode_liquid_transaction(raw_hex)
                actual = tx.txid().hex()
                raw = graph._liquid_electrum_decoded_to_graph_raw(txid, tx, raw_hex)
                for original, target in zip(tx.vin, raw["vin"]):
                    if getattr(original, "is_pegin", False) is True:
                        target["is_pegin"] = True
            if actual != txid:
                invalid("Backend returned different transaction bytes", "invalid_observation")
        else:
            raw = self.http(f"/tx/{txid}")
            if not isinstance(raw, dict) or raw.get("txid") != txid:
                invalid("Backend returned a different transaction", "invalid_observation")
            source_status = raw.get("status", {})
            if isinstance(source_status, dict):
                status = {key: source_status[key] for key in ("confirmed", "block_height", "block_hash", "block_time") if key in source_status and (type(source_status[key]) in {bool, int} or key == "block_hash" and _txid(source_status[key]))}
        if not isinstance(raw.get("vin"), list) or not isinstance(raw.get("vout"), list) or not raw["vin"] or not raw["vout"] or len(raw["vin"]) + len(raw["vout"]) > 20000:
            invalid("Backend returned an incomplete or oversized transaction", "invalid_observation")
        clean = graph._sanitize_graph_lookup_raw(raw, chain, txid)
        if chain == "bitcoin":
            from .chain_analysis.features import extract_transaction_features, PERSISTED_FEATURE_KEY
            clean[PERSISTED_FEATURE_KEY] = extract_transaction_features(feature_raw or raw, source="local_acquisition", subject_id=f"tx:{txid}")
        if len(clean["vin"]) != len(raw["vin"]) or len(clean["vout"]) != len(raw["vout"]):
            invalid("Backend returned malformed transaction legs", "invalid_observation")
        # Sequence is public transaction structure; witness stacks and preimages
        # are intentionally discarded before storage.
        for original, target in zip(raw["vin"], clean["vin"]):
            if isinstance(original.get("coinbase"), str) and normalized_script_hex(original["coinbase"]):
                target["is_coinbase"] = True
            if type(original.get("sequence")) is int and 0 <= original["sequence"] <= 0xFFFFFFFF:
                target["sequence"] = original["sequence"]
            if not target.get("is_coinbase") and not target.get("is_pegin") and (input_outpoint(target) is None or type(target.get("vout")) is not int):
                invalid("Backend returned an invalid input outpoint", "invalid_observation")
        seen_outputs = set()
        for output in clean["vout"]:
            number = output.get("n")
            script = output.get("scriptpubkey")
            value = output.get("value")
            if type(number) is not int or not 0 <= number < len(clean["vout"]) or number in seen_outputs or script is not None and (len(script) > 20000 or normalized_script_hex(script) is None) or value is not None and (type(value) is not int or value < 0) or value is None and script is None and output.get("value_state") != "confidential":
                invalid("Backend returned an invalid output", "invalid_observation")
            seen_outputs.add(number)
        return clean, status

    def spenders(self, txid, raw):
        if self.backend["kind"] == "bitcoinrpc":
            refs = [{"txid": txid, "vout": i} for i in range(len(raw["vout"]))]
            rows = self.rpc("gettxspendingprevout", [refs, {"mempool_only": False}])
            if not isinstance(rows, list) or len(rows) != len(refs) or any(not isinstance(row, dict) or row.get("txid") != txid or type(row.get("vout")) is not int or row.get("vout") != i or "spendingtxid" in row and not _txid(row["spendingtxid"]) for i, row in enumerate(rows)):
                invalid("Invalid spent-output response", "invalid_observation")
            claimed = {}
            for row in rows:
                if row.get("spendingtxid"):
                    claimed.setdefault(row["spendingtxid"], []).append((txid, row["vout"]))
            return claimed
        rows = self.http(f"/tx/{txid}/outspends")
        if not isinstance(rows, list) or len(rows) != len(raw["vout"]) or any(not isinstance(row, dict) or type(row.get("spent")) is not bool or row["spent"] and not _txid(row.get("txid")) for row in rows):
            invalid("Invalid spent-output response", "invalid_observation")
        claimed = {}
        for number, row in enumerate(rows):
            if row["spent"]:
                claimed.setdefault(row["txid"], []).append((txid, number))
        return claimed


def apply_acquisition(conn, profile_id, args):
    arguments(args, ("plan",), ("plan",))
    supplied = args["plan"]
    if not isinstance(supplied, dict) or not isinstance(supplied.get("args"), dict):
        invalid("An acquisition plan is required")
    expected, backend, seeds = _prepare_acquisition(conn, profile_id, supplied["args"])
    if canonical(expected) != canonical(supplied):
        invalid("Acquisition plan changed; review a new plan before fetching", "chain_analysis_stale")
    value, safe_backend = expected["args"], expected["backend"]
    # Use the exact backend and seed snapshots bound to the recomputed plan.
    # Re-reading could use a concurrently changed endpoint or new wallet txids.
    backend["timeout"] = min(8, backend_timeout(backend))
    budget = _Budget(expected["effects"]["max_requests"])
    queue, seen, observations, frontier = deque((seed, 0) for seed in seeds), set(), {}, []
    spend_claims = {}
    def stop(txid, reason):
        frontier.append({"txid": txid, "reason": reason})
    manager = _BoundedElectrumClient(backend, budget) if backend["kind"] == "electrum" else nullcontext(None)
    if backend["kind"] == "electrum":
        budget.request()  # server.version handshake is a real protocol request.
    with manager as client:
        reader = _Reader(backend, budget, client)
        if reader.genesis() != value["genesis_hash"]:
            invalid("Backend genesis does not match the selected network", "backend_network_mismatch")
        forward = backend["kind"] != "electrum"
        if backend["kind"] == "bitcoinrpc" and value["direction"] != "backward":
            try:
                indexes = reader.rpc("getindexinfo", ["txospenderindex"])
                forward = isinstance(indexes, dict) and indexes.get("txospenderindex", {}).get("synced") is True
            except AppError:
                forward = False
        while queue:
            txid, depth = queue.popleft()
            if txid in seen:
                continue
            if len(seen) >= value["max_transactions"]:
                stop(txid, "transaction_limit")
                continue
            seen.add(txid)
            try:
                raw, status = reader.transaction(txid, value["chain"])
            except (AppError, OSError, ValueError, TypeError) as error:
                stop(txid, "acquisition_budget" if isinstance(error, AppError) and error.code == "acquisition_budget" else "budget_or_history_unavailable" if budget.count >= budget.maximum or time.monotonic() >= budget.deadline else "transaction_unavailable")
                continue
            observations[txid] = (raw, status)
            neighbors = set()
            if value["direction"] != "forward":
                neighbors.update(row["txid"] for row in raw["vin"] if _txid(row.get("txid")) and not row.get("is_coinbase") and not row.get("is_pegin"))
            if value["direction"] != "backward":
                if not forward:
                    stop(txid, "historical_spender_index_unavailable")
                else:
                    try:
                        claimed = reader.spenders(txid, raw)
                        neighbors.update(claimed)
                        for spender, points in claimed.items():
                            spend_claims.setdefault(spender, set()).update(points)
                        status["spenders_checked"] = True
                    except (AppError, OSError, ValueError, TypeError):
                        stop(txid, "spender_history_unavailable")
            for neighbor in sorted(neighbors - seen):
                if depth >= value["depth"]:
                    stop(neighbor, "depth_limit")
                else:
                    queue.append((neighbor, depth + 1))
    for spender, points in spend_claims.items():
        if spender not in observations:
            continue  # Already represented by a history/depth/budget frontier.
        spent = {input_outpoint(item) for item in observations[spender][0]["vin"] if not item.get("is_pegin")}
        for source, number in points - spent:
            stop(source, "spender_claim_not_corroborated")
            if source in observations:
                observations[source][1]["spenders_checked"] = False
    # Fetching occurs outside a write transaction. Revalidate book and backend
    # before the atomic local save so concurrent edits cannot silently retarget.
    with atomic(conn):
        if plan_acquisition(conn, profile_id, value)["plan_id"] != expected["plan_id"]:
            invalid("Local inputs changed during acquisition; observations were not saved", "chain_analysis_stale")
        observed_at = now_iso()
        network = normalize_network(value["chain"], value["network"])
        for txid, (raw, status) in observations.items():
            conn.execute("INSERT INTO chain_analysis_observations VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(profile_id,chain,network,txid) DO UPDATE SET payload_json=excluded.payload_json,status_json=excluded.status_json,source_name=excluded.source_name,observed_at=excluded.observed_at", (profile_id, value["chain"], network, txid, canonical(raw), canonical(status), backend["name"], observed_at))
    return {"acquired_count": len(observations), "request_count": budget.count, "response_bytes": budget.bytes_read, "transaction_ids": sorted(observations), "frontier": [{"txid": txid, "reason": reason} for txid, reason in sorted({(item["txid"], item["reason"]) for item in frontier})], "complete": not frontier, "backend": safe_backend, "snapshot_id": build_index(conn, profile_id).snapshot_id}

"""Revocable recurring acquisition and resumable Bitcoin Core block observation.

An authorization pins the book, source configuration, domain and lifetime quota.
Requests reserve quota durably before transport; a crash can waste quota, never
replenish it. Worker leases and commit fencing prevent overlapping publication.
Only sanitized public facts are retained, independently of wallet observations.
"""
from __future__ import annotations

import hashlib
import io
import json
import time
import uuid

from ..backends import get_db_backend
from ..db import database_instance_id
from ..errors import AppError
from ..time_utils import now_iso
from . import chain_analysis_acquisition as acquisition
from .chain_analysis_cases import arguments, atomic, canonical, digest, invalid, text_value

READ_OPERATIONS = frozenset({"sources.plan", "sources.list"})
WRITE_OPERATIONS = frozenset({"sources.authorize", "sources.revoke", "sources.run"})


def _domain(conn, profile_id, spec):
    from .book_network import require_chain_domain
    return require_chain_domain(conn, profile_id, spec["chain"], spec["network"],
                                chain_instance_id=spec.get("chain_instance_id"), operation="recurring_acquisition")


def _source(conn, spec):
    backend = get_db_backend(conn, spec["backend"])
    if backend["kind"] != "bitcoinrpc" or backend.get("chain") != "bitcoin" or backend.get("network") != spec["network"]:
        invalid("Recurring acquisition requires a matching Bitcoin Core connection", "capability_unavailable")
    revision = conn.execute("SELECT updated_at FROM backends WHERE name=?", (backend["name"],)).fetchone()[0]
    return backend, digest([revision, acquisition._routing_identity(backend), backend["kind"], backend.get("chain"), backend.get("network")])


def plan(conn, profile_id, args):
    fields = ("backend", "chain", "network", "chain_instance_id", "mode", "subject", "start_height", "end_height", "interval_seconds", "duration_days", "max_requests", "max_bytes", "blocks_per_run")
    arguments(args, fields, ("backend", "network", "mode", "max_requests", "max_bytes"))
    spec = {"chain": "bitcoin", "interval_seconds": 300, "duration_days": 30, "blocks_per_run": 4, **args}
    spec["backend"] = text_value(spec["backend"], "backend", 128).lower()
    if spec["chain"] != "bitcoin" or spec["network"] not in {"main", "test", "signet", "regtest"} or spec["mode"] not in {"subject", "blocks"}:
        invalid("Choose a Bitcoin network and subject or blocks acquisition")
    for name, low, high in (("interval_seconds", 30, 86400), ("duration_days", 1, 365), ("max_requests", 4, 100000000), ("max_bytes", acquisition.MAX_RESPONSE_BYTES, 10**13), ("blocks_per_run", 1, 16)):
        if type(spec[name]) is not int or not low <= spec[name] <= high:
            invalid(f"{name} must be between {low} and {high}")
    if spec["mode"] == "subject":
        if not acquisition._txid(spec.get("subject")) or "start_height" in spec or "end_height" in spec:
            invalid("Subject acquisition requires one canonical txid and no block range")
    else:
        if "subject" in spec or type(spec.get("start_height")) is not int or spec["start_height"] < 0:
            invalid("Block acquisition requires an explicit nonnegative start height")
        if "end_height" in spec and (type(spec["end_height"]) is not int or spec["end_height"] < spec["start_height"]):
            invalid("End height must be at or above the start height")
    binding = _domain(conn, profile_id, spec)
    backend, fingerprint = _source(conn, spec)
    result = {"spec": spec, "binding": binding, "backend": {key: backend.get(key) for key in ("name", "kind", "chain", "network")},
              "effects": {"recurring_egress": True, "max_requests": spec["max_requests"], "max_response_bytes": acquisition.MAX_RESPONSE_BYTES,
                          "max_total_bytes": spec["max_bytes"], "duration_days": spec["duration_days"], "interval_seconds": spec["interval_seconds"],
                          "requires_running_unlocked_daemon": True},
              "limitations": ["Only the selected source is contacted. Pruned history remains incomplete.",
                              "Source, book or network changes invalidate this authorization.",
                              "Observations do not prove ownership or authorize accounting."]}
    result["plan_id"] = digest([profile_id, database_instance_id(conn), fingerprint, result])
    return result


def _require_protected_book(conn):
    from .chain_analysis_watches import require_encrypted
    require_encrypted(conn)


def authorize(conn, profile_id, args):
    _require_protected_book(conn)
    arguments(args, ("plan",), ("plan",))
    submitted = args["plan"]
    if not isinstance(submitted, dict):
        invalid("An unchanged source plan is required")
    with atomic(conn):
        expected = plan(conn, profile_id, submitted.get("spec", {}))
        if canonical(submitted) != canonical(expected):
            invalid("Source authorization changed; review a new plan", "chain_analysis_stale")
        spec = expected["spec"]
        _, fingerprint = _source(conn, spec)
        ident, now = uuid.uuid4().hex, int(time.time())
        conn.execute("""INSERT INTO chain_analysis_acquisition_grants
            (id,profile_id,spec_json,binding_json,source_fingerprint,database_identity,status,next_run_at,expires_at,created_at)
            VALUES(?,?,?,?,?,?,'active',?,?,?)""",
            (ident, profile_id, canonical(spec), canonical(expected["binding"]), fingerprint, database_instance_id(conn), now, now + spec["duration_days"] * 86400, now_iso()))
    return get(conn, profile_id, ident)


def _row(conn, profile_id, ident):
    text_value(ident, "id", 64)
    row = conn.execute("SELECT * FROM chain_analysis_acquisition_grants WHERE profile_id=? AND id=?", (profile_id, ident)).fetchone()
    if row is None:
        invalid("Source authorization not found in this book", "not_found")
    return dict(row)


def get(conn, profile_id, ident):
    row = _row(conn, profile_id, ident)
    # Do not expose endpoint fingerprints or internal lease identifiers.
    return {key: row[key] for key in ("id", "status", "revision", "requests_used", "bytes_used", "next_run_at", "expires_at", "cursor_height", "last_code", "created_at")} | {"spec": json.loads(row["spec_json"]), "binding": json.loads(row["binding_json"])}


def list_sources(conn, profile_id, args):
    arguments(args, ("limit", "after_id"))
    limit = args.get("limit", 50)
    if type(limit) is not int or not 1 <= limit <= 100:
        invalid("limit must be between 1 and 100")
    after = args.get("after_id", "")
    if not isinstance(after, str) or len(after) > 64:
        invalid("Invalid source cursor")
    ids = [row[0] for row in conn.execute("SELECT id FROM chain_analysis_acquisition_grants WHERE profile_id=? AND id>? ORDER BY id LIMIT ?", (profile_id, after, limit + 1))]
    return {"items": [get(conn, profile_id, ident) for ident in ids[:limit]], "next_cursor": ids[limit - 1] if len(ids) > limit else None}


def revoke(conn, profile_id, args):
    arguments(args, ("id", "expected_revision"), ("id", "expected_revision"))
    with atomic(conn):
        row = _row(conn, profile_id, args["id"])
        if type(args["expected_revision"]) is not int or row["revision"] != args["expected_revision"]:
            invalid("Source authorization changed", "chain_analysis_stale")
        conn.execute("UPDATE chain_analysis_acquisition_grants SET status='revoked',revision=revision+1,lease_token=NULL,lease_until=NULL WHERE id=?", (row["id"],))
    return get(conn, profile_id, row["id"])


def _validate(conn, profile_id, row, *, lease=None, cancelled=lambda: False):
    if cancelled():
        invalid("Acquisition cancelled", "cancelled")
    _require_protected_book(conn)
    live = _row(conn, profile_id, row["id"])
    if live["status"] != "active" or live["revision"] != row["revision"] or lease is not None and live["lease_token"] != lease:
        invalid("Source authorization is no longer active", "acquisition_revoked")
    if live["expires_at"] <= time.time():
        invalid("Source authorization expired", "acquisition_expired")
    if live["database_identity"] != database_instance_id(conn):
        invalid("Source authorization belongs to a different database", "scope_changed")
    spec = json.loads(live["spec_json"])
    if canonical(_domain(conn, profile_id, spec)) != live["binding_json"]:
        invalid("Book environment changed", "scope_changed")
    backend, fingerprint = _source(conn, spec)
    if fingerprint != live["source_fingerprint"]:
        invalid("Source configuration changed; authorize it again", "acquisition_source_changed")
    return live, backend


class _GrantBudget(acquisition._Budget):
    def __init__(self, conn, profile_id, row, token, cancelled):
        super().__init__(maximum=64, seconds=45)
        self.conn, self.profile_id, self.row, self.token, self.cancelled = conn, profile_id, row, token, cancelled
        self.reserved = False
        self.response_bytes = 0

    def request(self):
        # A previous successful response can refund its unused reservation. If a
        # request failed, the runner retains the reservation conservatively.
        self.settle()
        timeout = super().request()
        with atomic(self.conn):
            live, _ = _validate(self.conn, self.profile_id, self.row, lease=self.token, cancelled=self.cancelled)
            spec = json.loads(live["spec_json"])
            if live["requests_used"] >= spec["max_requests"] or live["bytes_used"] + acquisition.MAX_RESPONSE_BYTES > spec["max_bytes"]:
                invalid("Source lifetime quota reached; review a new authorization", "acquisition_quota")
            self.conn.execute("UPDATE chain_analysis_acquisition_grants SET requests_used=requests_used+1,bytes_used=bytes_used+? WHERE id=?", (acquisition.MAX_RESPONSE_BYTES, self.row["id"]))
        self.conn.commit()
        self.reserved, self.response_bytes = True, 0
        return timeout

    def received(self, size):
        self.response_bytes += size
        if self.cancelled():
            invalid("Acquisition cancelled", "cancelled")
        super().received(size)

    def settle(self):
        if self.reserved:
            with atomic(self.conn):
                self.conn.execute("UPDATE chain_analysis_acquisition_grants SET bytes_used=bytes_used-? WHERE id=?", (max(0, acquisition.MAX_RESPONSE_BYTES - self.response_bytes), self.row["id"]))
            self.conn.commit()
            self.reserved = False


def _clean_block(raw_hex, expected_hash):
    """Use embit's transaction parser; raw witnesses never enter stored rows."""
    from embit import compact
    from embit.transaction import Transaction
    from .chain_analysis.features import extract_transaction_features, feature_raw_from_transaction, PERSISTED_FEATURE_KEY
    if not isinstance(raw_hex, str) or len(raw_hex) > 8_000_000:
        invalid("Invalid block bytes", "invalid_observation")
    try:
        stream = io.BytesIO(bytes.fromhex(raw_hex))
        header = stream.read(80)
        if len(header) != 80 or hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex() != expected_hash:
            invalid("Block hash does not match its header", "invalid_observation")
        count = compact.read_from(stream)
        if not 1 <= count <= 100000:
            invalid("Invalid block transaction count", "invalid_observation")
        transactions, leaves = [], []
        for number in range(count):
            tx = Transaction.read_from(stream)
            txid = tx.txid().hex()
            leaves.append(bytes.fromhex(txid)[::-1])
            raw = feature_raw_from_transaction(tx)
            raw["txid"] = txid
            for item in raw["vin"]:
                if item.get("txid") == "0" * 64 and item.get("vout") == 0xFFFFFFFF:
                    item["is_coinbase"] = True
            clean = acquisition.graph._sanitize_graph_lookup_raw(raw, "bitcoin", txid)
            clean[PERSISTED_FEATURE_KEY] = extract_transaction_features(raw, source="local_acquisition", subject_id=f"tx:{txid}")
            for original, target in zip(raw["vin"], clean["vin"]):
                if "sequence" in original:
                    target["sequence"] = original["sequence"]
                if original.get("is_coinbase"):
                    target["is_coinbase"] = True
            transactions.append((number, txid, clean))
        while len(leaves) > 1:
            if len(leaves) % 2:
                leaves.append(leaves[-1])
            leaves = [hashlib.sha256(hashlib.sha256(leaves[i] + leaves[i + 1]).digest()).digest() for i in range(0, len(leaves), 2)]
        if leaves[0] != header[36:68]:
            invalid("Block transactions do not match the header commitment", "invalid_observation")
        if stream.read(1):
            invalid("Trailing block bytes", "invalid_observation")
        return header[4:36][::-1].hex(), transactions
    except (ValueError, IndexError, EOFError, TypeError) as exc:
        raise AppError("Invalid block structure", code="invalid_observation") from exc


def _assertions(conn, profile_id, row, observations):
    spec, binding = json.loads(row["spec_json"]), json.loads(row["binding_json"])
    for occurrence, txid, raw, status in observations:
        conn.execute("""INSERT INTO chain_analysis_reference_assertions
            VALUES(?,?,?,?,?,?,?,?,?,1,?) ON CONFLICT(grant_id,occurrence_id) DO UPDATE SET
            payload_json=excluded.payload_json,status_json=excluded.status_json,active=1,observed_at=excluded.observed_at""",
            (row["id"], profile_id, binding["domain_id"], spec["chain"], spec["network"], occurrence, txid, canonical(raw), canonical(status), now_iso()))


def _blocks(conn, profile_id, row, reader, validate):
    spec = json.loads(row["spec_json"])
    tip = reader.rpc("getblockcount")
    if type(tip) is not int or tip < 0:
        invalid("Invalid chain height", "invalid_observation")
    cursor = row["cursor_height"]
    # Disconnect one bounded suffix per run. Membership history is retained and
    # will be reactivated when the same block reappears.
    if cursor is not None and (cursor > tip or reader.rpc("getblockhash", [cursor]) != row["cursor_hash"]):
        with atomic(conn):
            validate()
            # Withhold the whole uncertain source until a common ancestor has
            # been checked. Old branch evidence must not trigger watch clears.
            conn.execute("UPDATE chain_analysis_reference_blocks SET active=2 WHERE grant_id=? AND active=1", (row["id"],))
            conn.execute("UPDATE chain_analysis_reference_assertions SET active=2 WHERE grant_id=? AND active=1", (row["id"],))
            conn.execute("UPDATE chain_analysis_reference_blocks SET active=0 WHERE grant_id=? AND block_hash=?", (row["id"], row["cursor_hash"]))
            conn.execute("UPDATE chain_analysis_reference_assertions SET active=0 WHERE grant_id=? AND occurrence_id LIKE ?", (row["id"], row["cursor_hash"] + ":%"))
            previous = conn.execute("SELECT height,block_hash FROM chain_analysis_reference_blocks WHERE grant_id=? AND active=2 ORDER BY height DESC LIMIT 1", (row["id"],)).fetchone()
            conn.execute("UPDATE chain_analysis_acquisition_grants SET cursor_height=?,cursor_hash=?,last_code='reorg_reconciling' WHERE id=?", (previous[0] if previous else None, previous[1] if previous else None, row["id"]))
        conn.commit()
        return "reorg_reconciling"
    with atomic(conn):
        validate()
        conn.execute("UPDATE chain_analysis_reference_blocks SET active=1 WHERE grant_id=? AND active=2", (row["id"],))
        conn.execute("UPDATE chain_analysis_reference_assertions SET active=1 WHERE grant_id=? AND active=2", (row["id"],))
    conn.commit()
    start = spec["start_height"] if cursor is None else cursor + 1
    end = min(tip, spec.get("end_height", tip), start + spec["blocks_per_run"] - 1)
    parent = row["cursor_hash"]
    for height in range(start, end + 1):
        block_hash = reader.rpc("getblockhash", [height])
        if not acquisition._txid(block_hash):
            invalid("Invalid block identity", "invalid_observation")
        previous, transactions = _clean_block(reader.rpc("getblock", [block_hash, 0]), block_hash)
        if parent is not None and previous != parent or reader.rpc("getblockhash", [height]) != block_hash:
            invalid("Chain changed during block acquisition", "chain_analysis_stale")
        with atomic(conn):
            validate()
            conn.execute("""INSERT INTO chain_analysis_reference_blocks VALUES(?,?,?,?,1)
                ON CONFLICT(grant_id,block_hash) DO UPDATE SET active=1""", (row["id"], block_hash, height, previous))
            status = {"confirmed": True, "block_hash": block_hash, "block_height": height}
            for number, txid, raw in transactions:
                for item in raw["vin"]:
                    if item.get("is_coinbase"):
                        continue
                    prior = conn.execute("""SELECT occurrence_id,payload_json FROM chain_analysis_reference_assertions
                        WHERE grant_id=? AND active=1 AND txid=?
                        ORDER BY CAST(json_extract(status_json,'$.block_height') AS INTEGER) DESC LIMIT 1""",
                        (row["id"], item.get("txid"))).fetchone()
                    if prior:
                        outputs = json.loads(prior["payload_json"])["vout"]
                        vout = item.get("vout")
                        if type(vout) is int and 0 <= vout < len(outputs):
                            item.update(reference_occurrence_id=prior["occurrence_id"], reference_grant_id=row["id"], prevout=outputs[vout])
                _assertions(conn, profile_id, row, [(f"{block_hash}:{number}", txid, raw, status)])
            conn.execute("UPDATE chain_analysis_acquisition_grants SET cursor_height=?,cursor_hash=?,last_code=NULL WHERE id=?", (height, block_hash, row["id"]))
        conn.commit()
        parent = block_hash
    return "range_complete" if spec.get("end_height") is not None and end >= spec["end_height"] else "caught_up" if end >= tip else "backfilling"


def run(conn, profile_id, ident, *, cancelled=lambda: False):
    if conn.in_transaction:
        invalid("Commit pending edits before acquisition", "transaction_active")
    token = uuid.uuid4().hex
    with atomic(conn):
        row = _row(conn, profile_id, ident)
        row, backend = _validate(conn, profile_id, row, cancelled=cancelled)
        now = int(time.time())
        if row["lease_token"] and (row["lease_until"] or 0) > now:
            invalid("This source is already running", "acquisition_running")
        conn.execute("UPDATE chain_analysis_acquisition_grants SET lease_token=?,lease_until=? WHERE id=?", (token, now + 90, ident))
    conn.commit()
    budget = _GrantBudget(conn, profile_id, row, token, cancelled)
    reader = acquisition._Reader(backend, budget)
    def validate():
        return _validate(conn, profile_id, row, lease=token, cancelled=cancelled)
    code, status = None, "active"
    try:
        spec = json.loads(row["spec_json"])
        if reader.genesis() != acquisition.GENESIS[("bitcoin", spec["network"])]:
            invalid("Source genesis changed", "backend_network_mismatch")
        if spec["mode"] == "blocks":
            code = _blocks(conn, profile_id, row, reader, validate)
        else:
            raw, observed = reader.transaction(spec["subject"], "bitcoin")
            with atomic(conn):
                validate()
                conn.execute("UPDATE chain_analysis_reference_assertions SET active=0 WHERE grant_id=? AND active=1", (ident,))
                _assertions(conn, profile_id, row, [(token + ":" + spec["subject"], spec["subject"], raw, observed)])
            conn.commit()
            code = "observed"
        budget.settle()
    except (AppError, OSError, ValueError, TypeError) as exc:
        conn.rollback()
        code = exc.code if isinstance(exc, AppError) else "history_unavailable"
        if code in {"acquisition_quota", "acquisition_expired", "acquisition_source_changed", "scope_changed", "book_network_unbound", "book_network_mismatch", "watch_requires_encrypted_database", "backend_network_mismatch", "acquisition_revoked"}:
            status = "paused"
    finally:
        with atomic(conn):
            # Revocation or another worker's lease can never be undone here.
            conn.execute("""UPDATE chain_analysis_acquisition_grants SET status=?,last_code=?,next_run_at=?,lease_token=NULL,lease_until=NULL
                WHERE id=? AND status='active' AND revision=? AND lease_token=?""",
                (status, code, int(time.time()) + (1 if code in {"backfilling", "reorg_reconciling"} else json.loads(row["spec_json"])["interval_seconds"]), ident, row["revision"], token))
        conn.commit()
    return get(conn, profile_id, ident)


def run_due(conn, profile_id, *, cancelled=lambda: False):
    rows = conn.execute("SELECT id FROM chain_analysis_acquisition_grants WHERE profile_id=? AND status='active' AND next_run_at<=? ORDER BY next_run_at,id LIMIT 1", (profile_id, int(time.time()))).fetchall()
    results = []
    for row in rows:
        if cancelled():
            break
        try:
            results.append(run(conn, profile_id, row[0], cancelled=cancelled))
        except AppError as error:
            if error.code == "acquisition_running":
                continue
            if error.code in {"acquisition_expired", "acquisition_source_changed", "scope_changed", "book_network_unbound", "book_network_mismatch", "watch_requires_encrypted_database"}:
                with atomic(conn):
                    conn.execute("UPDATE chain_analysis_acquisition_grants SET status='paused',last_code=? WHERE id=? AND status='active'", (error.code, row[0]))
                conn.commit()
            else:
                raise
    return results


def has_pending_work(conn, profile_id):
    return conn.execute("SELECT 1 FROM chain_analysis_acquisition_grants WHERE profile_id=? AND status='active' LIMIT 1", (profile_id,)).fetchone() is not None


def dispatch(conn, profile_id, operation, args):
    if operation == "sources.plan":
        return plan(conn, profile_id, args)
    if operation == "sources.list":
        return list_sources(conn, profile_id, args)
    if operation == "sources.authorize":
        return authorize(conn, profile_id, args)
    if operation == "sources.revoke":
        return revoke(conn, profile_id, args)
    arguments(args, ("id",), ("id",))
    row = _row(conn, profile_id, args["id"])
    _validate(conn, profile_id, row)
    conn.execute("UPDATE chain_analysis_acquisition_grants SET next_run_at=? WHERE id=?", (int(time.time()), row["id"]))
    return {**get(conn, profile_id, row["id"]), "queued": True, "requires_running_unlocked_daemon": True}

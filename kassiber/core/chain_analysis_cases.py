"""Immutable investigation snapshots and local, revisioned attribution claims.

These records never authorize custody, tax treatment or wallet ownership.
Callers own the commit; savepoints make multi-record operations atomic.
"""
from __future__ import annotations

from contextlib import contextmanager
import base64
import hashlib
import json
import re
import uuid

from ..errors import AppError
from ..time_utils import now_iso
from .chain_analysis import run_analysis


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def invalid(message, code="validation"):
    raise AppError(message, code=code, retryable=code == "chain_analysis_stale")


def text_value(value, field, limit=256):
    if not isinstance(value, str) or not value.strip() or len(value) > limit or any(ord(c) < 32 for c in value):
        invalid(f"{field} must be nonempty text up to {limit} characters")
    return value.strip()


def arguments(args, allowed, required=()):
    if not isinstance(args, dict) or set(args) - set(allowed) or set(required) - set(args):
        invalid("Unsupported or missing chain analysis arguments")


@contextmanager
def atomic(conn):
    name = "chain_analysis_" + uuid.uuid4().hex
    conn.execute(f"SAVEPOINT {name}")
    try:
        yield
    except BaseException:
        conn.execute(f"ROLLBACK TO SAVEPOINT {name}")
        raise
    finally:
        conn.execute(f"RELEASE SAVEPOINT {name}")


def _case(row, *, detail=True):
    try:
        result = json.loads(row["result_json"])
        query = json.loads(row["query_json"])
    except (ValueError, TypeError):
        invalid("Stored investigation failed its content integrity check", "integrity_error")
    if not isinstance(result, dict) or digest(result) != row["result_digest"] or query != result.get("query") or row["snapshot_id"] != result.get("snapshot_id"):
        invalid("Stored investigation failed its content integrity check", "integrity_error")
    value = {key: row[key] for key in ("id", "title", "created_at", "snapshot_id")}
    value.update(query=query, summary=result.get("summary", {}))
    if detail:
        value["result"] = result
    return value


def get_case(conn, profile_id, case_id):
    text_value(case_id, "id", 64)
    row = conn.execute("SELECT * FROM chain_analysis_cases WHERE profile_id=? AND id=?", (profile_id, case_id)).fetchone()
    if row is None:
        invalid("Investigation not found in this book", "not_found")
    return _case(row)


def list_cases(conn, profile_id, args):
    arguments(args, ("limit", "cursor"))
    limit = args.get("limit", 50)
    if type(limit) is not int or not 1 <= limit <= 100:
        invalid("limit must be between 1 and 100")
    params, clause = [profile_id], ""
    if args.get("cursor"):
        try:
            cursor = json.loads(base64.urlsafe_b64decode(text_value(args["cursor"], "cursor", 2048)))
            if not isinstance(cursor, list) or len(cursor) != 3 or cursor[0] != profile_id or not all(isinstance(v, str) for v in cursor):
                raise ValueError
        except (ValueError, TypeError):
            invalid("Invalid investigation cursor")
        clause = " AND (created_at, id) < (?, ?)"
        params.extend(cursor[1:])
    rows = conn.execute(f"SELECT * FROM chain_analysis_cases WHERE profile_id=?{clause} ORDER BY created_at DESC,id DESC LIMIT ?", (*params, limit + 1)).fetchall()
    items = [_case(row, detail=False) for row in rows[:limit]]
    cursor = None
    if len(rows) > limit:
        last = rows[limit - 1]
        cursor = base64.urlsafe_b64encode(canonical([profile_id, last["created_at"], last["id"]]).encode()).decode()
    return {"items": items, "next_cursor": cursor}


def save_case(conn, profile_id, args):
    arguments(args, ("title", "query", "expected_snapshot_id"), ("title", "query", "expected_snapshot_id"))
    title = text_value(args["title"], "title")
    with atomic(conn):
        result = run_analysis(conn, profile_id, args["query"])
        if result["snapshot_id"] != args["expected_snapshot_id"]:
            invalid("Analysis inputs changed; run the query again before saving", "chain_analysis_stale")
        payload = canonical(result)
        if len(payload.encode()) > 10_000_000:
            invalid("Investigation exceeds the 10 MB snapshot limit; narrow the query")
        case_id = str(uuid.uuid4())
        conn.execute("INSERT INTO chain_analysis_cases VALUES(?,?,?,?,?,?,?,?)", (case_id, profile_id, title, canonical(result["query"]), payload, result["snapshot_id"], digest(result), now_iso()))
        return get_case(conn, profile_id, case_id)


def delete_case(conn, profile_id, case_id):
    get_case(conn, profile_id, case_id)
    conn.execute("DELETE FROM chain_analysis_cases WHERE profile_id=? AND id=?", (profile_id, case_id))
    return {"id": case_id, "deleted": True}


def compare_case(conn, profile_id, args):
    arguments(args, ("id", "other_id"), ("id",))
    with atomic(conn):
        base = get_case(conn, profile_id, args["id"])
        current = get_case(conn, profile_id, args["other_id"])["result"] if args.get("other_id") else run_analysis(conn, profile_id, base["query"])
        result = {"base_id": base["id"], "base_snapshot_id": base["snapshot_id"], "current_snapshot_id": current["snapshot_id"], "changed": digest(base["result"]) != digest(current), "comparison": "saved_to_saved" if args.get("other_id") else "saved_to_current", "summary": {"base": base["summary"], "current": current.get("summary", {})}}
        for kind in ("nodes", "edges"):
            before = {row["id"]: row for row in base["result"][kind]}
            after = {row["id"]: row for row in current[kind]}
            result[f"added_{kind}"] = [after[key] for key in sorted(after.keys() - before.keys())]
            result[f"removed_{kind}"] = [before[key] for key in sorted(before.keys() - after.keys())]
            result[f"changed_{kind}"] = [{"id": key, "before": before[key], "after": after[key]} for key in sorted(before.keys() & after.keys()) if before[key] != after[key]]
        return result


def validate_domain(chain, network):
    if not isinstance(chain, str) or not isinstance(network, str) or chain not in {"bitcoin", "liquid"} or network not in {"main", "test", "signet", "regtest"} or (chain == "liquid" and network == "signet"):
        invalid("A valid explicit chain and network are required")


def validate_subject(subject, chain, network):
    subject = text_value(subject, "subject", 256)
    from ..wallet_descriptors import normalize_network
    prefix = f"{chain}:{normalize_network(chain, network)}:"
    physical = subject.removeprefix(prefix)
    if physical.startswith("tx:"):
        physical = physical[3:]
    elif physical.startswith("out:"):
        physical = physical[4:]
    if re.fullmatch(r"[0-9a-fA-F]{64}(?::(?:0|[1-9][0-9]{0,9}))?", physical):
        return subject.lower()
    # Validate addresses with the existing wallet libraries; never accept
    # descriptors, extended keys or arbitrary text as a chain subject.
    try:
        if chain == "bitcoin":
            from .address_scripts import address_to_scriptpubkey, base58check_decode
            address_to_scriptpubkey(subject)
            lower = subject.lower()
            if lower.startswith(("bc1", "tb1", "bcrt1")):
                hrp = lower.split("1", 1)[0]
                expected = "bc" if network == "main" else "bcrt" if network == "regtest" else "tb"
                if hrp != expected:
                    raise ValueError
                return lower
            if base58check_decode(subject)[0] not in ({0, 5} if network == "main" else {111, 196}):
                raise ValueError
        else:
            from embit.liquid.addresses import addr_decode
            addr_decode(subject)
            # embit decoding validates checksum. Network prefixes are distinct
            # for Liquid main/test/regtest, including confidential addresses.
            prefixes = {"main": ("ex1", "lq1", "V", "G", "H"), "test": ("tex1", "tlq1", "Q", "8", "v"), "regtest": ("ert1", "el1", "2", "X", "Az")}
            if not subject.startswith(prefixes[network]):
                raise ValueError
    except (AppError, ValueError, TypeError, ImportError, IndexError) as exc:
        raise AppError("Subject must be a transaction, outpoint or valid address in the selected domain", code="validation") from exc
    return subject


def _label(row):
    value = dict(row)
    value["cluster_defining"] = bool(value["cluster_defining"])
    value["deleted"] = bool(value["deleted"])
    value.pop("profile_id", None)
    return value


def list_labels(conn, profile_id):
    return {"items": [_label(row) for row in conn.execute("SELECT * FROM chain_analysis_labels WHERE profile_id=? AND deleted=0 ORDER BY chain,network,subject,id", (profile_id,))]}


def upsert_label(conn, profile_id, args):
    fields = ("subject", "chain", "network", "label", "category", "source", "confidence", "cluster_defining")
    arguments(args, (*fields, "id", "expected_revision"), fields[:-1])
    validate_domain(args["chain"], args["network"])
    value = {key: text_value(args[key], key, 512 if key == "source" else 256) for key in fields[:-1]}
    value["subject"] = validate_subject(value["subject"], value["chain"], value["network"])
    if value["category"] not in {"exchange", "merchant", "mixer", "service", "self", "other"} or value["confidence"] not in {"user_confirmed", "imported", "unverified"}:
        invalid("Unsupported label category or confidence")
    cluster = args.get("cluster_defining", False)
    if type(cluster) is not bool:
        invalid("cluster_defining must be a boolean")
    with atomic(conn):
        timestamp = now_iso()
        if args.get("id"):
            old = conn.execute("SELECT * FROM chain_analysis_labels WHERE id=? AND profile_id=? AND deleted=0", (args["id"], profile_id)).fetchone()
            if old is None:
                invalid("Label not found in this book", "not_found")
            if type(args.get("expected_revision")) is not int or old["revision"] != args["expected_revision"]:
                invalid("Label changed; reload before editing", "chain_analysis_stale")
            label_id, revision, created = old["id"], old["revision"] + 1, old["created_at"]
        else:
            if "expected_revision" in args:
                invalid("expected_revision is only valid when updating a label")
            label_id, revision, created = str(uuid.uuid4()), 1, timestamp
        conn.execute("INSERT INTO chain_analysis_labels VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET chain=excluded.chain,network=excluded.network,subject=excluded.subject,label=excluded.label,category=excluded.category,source=excluded.source,confidence=excluded.confidence,cluster_defining=excluded.cluster_defining,revision=excluded.revision,updated_at=excluded.updated_at", (label_id, profile_id, value["chain"], value["network"], value["subject"], value["label"], value["category"], value["source"], value["confidence"], int(cluster), revision, 0, created, timestamp))
        result = _label(conn.execute("SELECT * FROM chain_analysis_labels WHERE id=?", (label_id,)).fetchone())
        conn.execute("INSERT INTO chain_analysis_label_history VALUES(?,?,?,?)", (label_id, revision, canonical(result), timestamp))
        return result


def delete_label(conn, profile_id, args):
    arguments(args, ("id", "expected_revision"), ("id", "expected_revision"))
    with atomic(conn):
        row = conn.execute("SELECT * FROM chain_analysis_labels WHERE id=? AND profile_id=? AND deleted=0", (args["id"], profile_id)).fetchone()
        if row is None:
            invalid("Label not found in this book", "not_found")
        if type(args["expected_revision"]) is not int or args["expected_revision"] != row["revision"]:
            invalid("Label changed; reload before deleting", "chain_analysis_stale")
        revision, timestamp = row["revision"] + 1, now_iso()
        conn.execute("UPDATE chain_analysis_labels SET deleted=1,revision=?,updated_at=? WHERE id=?", (revision, timestamp, row["id"]))
        result = _label(conn.execute("SELECT * FROM chain_analysis_labels WHERE id=?", (row["id"],)).fetchone())
        conn.execute("INSERT INTO chain_analysis_label_history VALUES(?,?,?,?)", (row["id"], revision, canonical(result), timestamp))
        return result


def import_labels(conn, profile_id, args):
    arguments(args, ("items",), ("items",))
    items = args["items"]
    if not isinstance(items, list) or not 1 <= len(items) <= 5000:
        invalid("A label import requires between 1 and 5000 structured labels")
    with atomic(conn):
        results = []
        for item in items:
            if not isinstance(item, dict) or {"id", "expected_revision"} & set(item):
                invalid("Label imports create new claims; use revisioned edits for existing labels")
            results.append(upsert_label(conn, profile_id, item))
        return {"imported_count": len(results), "items": results}

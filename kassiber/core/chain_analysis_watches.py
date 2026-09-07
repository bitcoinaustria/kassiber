"""Durable local evidence watches; no transports, model calls or accounting writes.

The canonical graph supplies facts and observer visibility. A watch compares
semantic observations, not graph hashes. Caller-owned transactions atomically
advance the baseline and append inbox events; delivery is a separate receipt.
"""
from __future__ import annotations

import json
import uuid

from ..db import database_instance_id
from ..errors import AppError
from ..time_utils import now_iso
from .chain_analysis import run_analysis
from .chain_analysis.query import normalize_query
from .chain_analysis_cases import arguments, atomic, canonical, digest, get_case, invalid, text_value


RULE_VERSION = 1
RULES = frozenset({"output_spent", "confirmations", "connection_supported", "attribution_changed", "findings_changed"})


def require_encrypted(conn):
    # The driver capability alone is insufficient: plaintext SQLite opened by
    # SQLCipher still has cipher_version. cipher_integrity_check only applies to
    # keyed databases; cipher_provider + nonempty cipher page size is not proof.
    row = conn.execute("PRAGMA cipher_version").fetchone()
    keyed = conn.execute("PRAGMA cipher_salt").fetchone() if row else None
    if not keyed or not keyed[0]:
        raise AppError("Encrypt this book before enabling background evidence work", code="watch_requires_encrypted_database", hint="Open Settings > Lock & encryption, or run secrets init for this book")


def domain_for(conn, profile_id, query):
    from .book_network import require_chain_domain
    return require_chain_domain(conn, profile_id, query["chain"], query["network"], operation="analysis")


def revision_for(conn, profile_id):
    from .chain_analysis.projection import index_revision
    return index_revision(conn, profile_id)


def normalize_definition(conn, profile_id, args):
    arguments(args, ("rule", "query", "case_id", "threshold"), ("rule",))
    if not isinstance(args["rule"], str) or args["rule"] not in RULES:
        invalid("Unsupported watch rule")
    if bool(args.get("case_id")) == bool(args.get("query")):
        invalid("Choose one investigation or query")
    query = get_case(conn, profile_id, args["case_id"])["query"] if args.get("case_id") else args["query"]
    query = normalize_query(query)
    if not query.get("chain") or not query.get("network"):
        invalid("Select an explicit chain and network before watching")
    rule = args["rule"]
    if rule in {"output_spent", "confirmations", "attribution_changed"} and not query.get("subject"):
        invalid("This watch requires a subject")
    if rule == "connection_supported" and query["mode"] != "path":
        invalid("A connection watch requires a path query")
    if rule in {"output_spent", "confirmations"}:
        query.update(mode="trace", direction="both", depth=2, include_relations=False, include_hypotheses=False)
        query.pop("target", None)
    definition = {"rule": rule, "rule_version": RULE_VERSION, "query": query}
    if args.get("case_id"):
        definition["case_id"] = text_value(args["case_id"], "case_id", 64)
    if rule == "confirmations":
        threshold = args.get("threshold", 1)
        if type(threshold) is not int or not 1 <= threshold <= 10000:
            invalid("Confirmation threshold must be between 1 and 10000")
        definition["threshold"] = threshold
    elif "threshold" in args:
        invalid("Only confirmation watches accept a threshold")
    return definition


def _subject_nodes(result, subject):
    return [node for node in result["nodes"] if subject in {node["id"], node.get("txid") if node["kind"] == "transaction" else None, node.get("outpoint"), node.get("address")}]


def observe(conn, profile_id, definition):
    """Evidence disappearance is unknown, never evidence of absence."""
    if definition["rule_version"] != RULE_VERSION:
        return {"status": "unavailable", "reason": "rule_version_changed", "values": []}
    try:
        result = run_analysis(conn, profile_id, definition["query"])
    except AppError as error:
        if error.code in {"not_found", "subject_ambiguous"}:
            return {"status": "unavailable", "reason": "subject_unavailable", "values": []}
        raise
    rule = definition["rule"]
    nodes = _subject_nodes(result, definition["query"].get("subject"))
    trusted = {node["id"] for node in result["nodes"] if node["status"] not in {"missing", "stale", "conflicting", "retracted"}}
    complete = result["coverage"].get("complete") is True
    if rule == "output_spent":
        if len(nodes) != 1 or nodes[0]["kind"] != "output":
            return {"status": "unavailable", "reason": "output_required", "values": []}
        # A wallet inventory spend can name a yet-unfetched successor. The
        # spends edge itself is positive evidence, but stale source is not.
        invalid_nodes = {node["id"] for node in result["nodes"] if node["status"] in {"stale", "conflicting", "retracted"}}
        for edge in result["edges"]:
            if edge["kind"] == "creates" and edge["source"] in invalid_nodes:
                invalid_nodes.add(edge["target"])
        values = sorted({edge["target"] for edge in result["edges"] if edge["target"] not in invalid_nodes and edge["source"] not in invalid_nodes and edge["kind"] == "spends" and edge["source"] == nodes[0]["id"] and edge["source"] in trusted and edge.get("status") not in {"stale", "conflicting"}})
        return {"status": "observed" if values else "unknown", "values": values}
    if rule == "confirmations":
        values = [node.get("confirmations") for node in nodes if node["id"] in trusted]
        known = len(values) == 1 and type(values[0]) is int and values[0] >= 0
        return {"status": "observed" if known else "unknown", "values": [values[0] >= definition["threshold"]] if known else []}
    if rule == "connection_supported":
        return {"status": "observed" if result.get("paths") else "unknown", "values": [True] if result.get("paths") else []}
    if rule == "attribution_changed":
        # Canonical analytics exposes direct observer-visible label matches;
        # only semantic claim fields count, never ingestion timestamps.
        values = sorted({canonical({key: label.get(key) for key in ("subject", "label", "category", "confidence", "source", "dataset_id", "visibility")}) for exposure in result.get("exposure", []) if exposure["kind"] == "direct_label_match" for label in [exposure["claim"]]})
    else:
        values = sorted({canonical({"code": item["code"], "node_ids": sorted(item.get("node_ids", [])), "rule_version": item.get("rule_version")}) for item in result["findings"]})
    return {"status": "observed" if complete else "partial", "values": values}


def preview(conn, profile_id, args):
    require_encrypted(conn)
    with atomic(conn):
        definition = normalize_definition(conn, profile_id, args)
        domain = domain_for(conn, profile_id, definition["query"])
        revision = revision_for(conn, profile_id)
        observation = observe(conn, profile_id, definition)
        plan = {"definition": definition, "domain": domain, "book_id": database_instance_id(conn), "profile_id": profile_id, "revision": revision, "baseline": observation}
        return {**plan, "plan_id": digest(plan), "local_only": True, "egress": "none"}


def create(conn, profile_id, args):
    arguments(args, ("plan", "definition", "expected_plan_id"))
    if "plan" not in args:
        arguments(args, ("definition", "expected_plan_id"), ("definition", "expected_plan_id"))
        with atomic(conn):
            plan = preview(conn, profile_id, args["definition"])
            if plan["plan_id"] != args["expected_plan_id"]:
                invalid("Evidence or book changed; preview this watch again", "chain_analysis_stale")
            return create(conn, profile_id, {"plan": plan})
    arguments(args, ("plan",), ("plan",))
    plan = args["plan"]
    if not isinstance(plan, dict) or not isinstance(plan.get("definition"), dict):
        invalid("A reviewed watch preview is required")
    supplied = plan["definition"]
    fields = {key: supplied[key] for key in ("rule", "threshold") if key in supplied}
    fields.update({"case_id": supplied["case_id"]} if supplied.get("case_id") else {"query": supplied.get("query")})
    with atomic(conn):
        current = preview(conn, profile_id, fields)
        if current != plan:
            invalid("Evidence or book changed; preview this watch again", "chain_analysis_stale")
        if conn.execute("SELECT COUNT(*) FROM chain_analysis_watches WHERE profile_id=?", (profile_id,)).fetchone()[0] >= 250:
            invalid("This book has reached its 250 watch limit")
        existing = conn.execute("SELECT id FROM chain_analysis_watches WHERE profile_id=? AND definition_json=? AND domain_json=?", (profile_id, canonical(current["definition"]), canonical(current["domain"]))).fetchone()
        if existing:
            existing_watch = get_watch(conn, profile_id, existing["id"])
            if not existing_watch["enabled"]:
                raise AppError("This watch already exists and is paused; resume it in Watches", code="watch_already_exists")
            return existing_watch
        ident, now = str(uuid.uuid4()), now_iso()
        conn.execute("INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES(?,?,?,?,?,?,?,?,?)", (ident, profile_id, current["book_id"], canonical(current["domain"]), canonical(current["definition"]), canonical(current["baseline"]), canonical(current["revision"]), now, now))
        return get_watch(conn, profile_id, ident)


def _watch(row):
    return {"id": row["id"], "definition": json.loads(row["definition_json"]), "enabled": bool(row["enabled"]), "revision": row["revision"], "baseline": json.loads(row["baseline_json"]), "checked_at": row["checked_at"], "created_at": row["created_at"]}


def get_watch(conn, profile_id, ident):
    row = conn.execute("SELECT * FROM chain_analysis_watches WHERE profile_id=? AND id=?", (profile_id, text_value(ident, "id", 64))).fetchone()
    if row is None:
        invalid("Watch not found in this book", "not_found")
    return _watch(row)


def list_watches(conn, profile_id, args):
    arguments(args, ())
    return {"items": [_watch(row) for row in conn.execute("SELECT * FROM chain_analysis_watches WHERE profile_id=? ORDER BY created_at,id", (profile_id,))], "local_only": True}


def configure(conn, profile_id, args):
    arguments(args, ("id", "expected_revision", "enabled"), ("id", "expected_revision", "enabled"))
    if type(args["enabled"]) is not bool:
        invalid("enabled must be boolean")
    with atomic(conn):
        old = get_watch(conn, profile_id, args["id"])
        if type(args["expected_revision"]) is not int or old["revision"] != args["expected_revision"]:
            invalid("Watch changed; reload it", "chain_analysis_stale")
        conn.execute("UPDATE chain_analysis_watches SET enabled=?,revision=revision+1,checkpoint='' WHERE id=?", (int(args["enabled"]), args["id"]))
        return get_watch(conn, profile_id, args["id"])


def delete(conn, profile_id, args):
    arguments(args, ("id", "expected_revision"), ("id", "expected_revision"))
    with atomic(conn):
        watch = get_watch(conn, profile_id, args["id"])
        if type(args["expected_revision"]) is not int or watch["revision"] != args["expected_revision"]:
            invalid("Watch changed; reload it", "chain_analysis_stale")
        # Explicit cleanup also removes local notification history. Other
        # evidence, saved investigations and accounting remain independent.
        conn.execute("DELETE FROM chain_analysis_watch_inbox WHERE watch_id=? AND profile_id=?", (args["id"], profile_id))
        conn.execute("DELETE FROM chain_analysis_watches WHERE id=? AND profile_id=?", (args["id"], profile_id))
        return {"id": args["id"], "deleted": True}


def transition(before, after, rule):
    if before == after:
        return None
    previous, current = set(map(canonical, before["values"])), set(map(canonical, after["values"]))
    if after["status"] in {"unavailable", "unknown"} and before["status"] not in {"unavailable", "unknown"}:
        return "coverage_lost"
    if rule == "confirmations" and after["status"] == "observed" and before["values"] != after["values"]:
        return "threshold_reached" if after["values"] == [True] else "threshold_reversed" if before["values"] == [True] else None
    if current - previous:
        return {"output_spent": "spend_observed", "connection_supported": "connection_supported", "attribution_changed": "attribution_changed", "findings_changed": "findings_changed"}.get(rule)
    if previous - current:
        return "evidence_changed" if after["status"] == "observed" else "coverage_lost"
    if before["status"] == "observed" and after["status"] == "partial":
        return "coverage_lost"
    return None


def evaluate_due(conn, profile_id, *, cancelled=lambda: False):
    require_encrypted(conn)
    emitted = 0
    with atomic(conn):
        revision = revision_for(conn, profile_id)
        for row in conn.execute("SELECT * FROM chain_analysis_watches WHERE profile_id=? AND enabled=1 ORDER BY id", (profile_id,)).fetchall():
            if cancelled():
                raise AppError("Local watch evaluation cancelled", code="cancelled")
            definition = json.loads(row["definition_json"])
            # Resolve on every tick, even without evidence changes: environment
            # rebinding must never continue a watch under another domain.
            try:
                domain = domain_for(conn, profile_id, definition["query"])
                valid = domain == json.loads(row["domain_json"]) and row["book_id"] == database_instance_id(conn)
            except AppError:
                valid = False
            if valid and row["checkpoint"] == canonical(revision):
                continue
            after = observe(conn, profile_id, definition) if valid else {"status": "unavailable", "reason": "domain_changed", "values": []}
            before = json.loads(row["baseline_json"])
            code = transition(before, after, definition["rule"])
            sequence = row["sequence"] + int(code is not None)
            if code:
                conn.execute("INSERT INTO chain_analysis_watch_inbox(id,watch_id,profile_id,sequence,code,observation_json,created_at) VALUES(?,?,?,?,?,?,?)", (str(uuid.uuid4()), row["id"], profile_id, sequence, code, canonical({"before": before, "after": after}), now_iso()))
                emitted += 1
            conn.execute("UPDATE chain_analysis_watches SET baseline_json=?,checkpoint=?,sequence=?,checked_at=? WHERE id=?", (canonical(after), canonical(revision) if valid else "", sequence, now_iso(), row["id"]))
        if cancelled():
            raise AppError("Local watch evaluation cancelled", code="cancelled")
    return {"evaluated": True, "event_count": emitted, "local_only": True}


def inbox(conn, profile_id, args):
    arguments(args, ("limit", "before"))
    limit = args.get("limit", 50)
    if type(limit) is not int or not 1 <= limit <= 100:
        invalid("limit must be between 1 and 100")
    # Monotonic SQLite rowid survives equal timestamps, UUID order and clock jumps.
    before = args.get("before", 9223372036854775807)
    if type(before) is not int or before < 1:
        invalid("before must be a positive inbox cursor")
    rows = conn.execute("SELECT rowid AS cursor,* FROM chain_analysis_watch_inbox WHERE profile_id=? AND rowid<? ORDER BY rowid DESC LIMIT ?", (profile_id, before, limit + 1)).fetchall()
    items = [{key: row[key] for key in ("id", "watch_id", "code", "created_at", "acknowledged_at", "cursor")} | {"observation": json.loads(row["observation_json"])} for row in rows[:limit]]
    unread = conn.execute("SELECT COUNT(*) FROM chain_analysis_watch_inbox WHERE profile_id=? AND acknowledged_at IS NULL", (profile_id,)).fetchone()[0]
    return {"items": items, "next_cursor": items[-1]["cursor"] if len(rows) > limit else None, "unread_count": unread}


def acknowledge(conn, profile_id, args):
    arguments(args, ("id",), ("id",))
    ident = text_value(args["id"], "id", 64)
    result = conn.execute("UPDATE chain_analysis_watch_inbox SET acknowledged_at=COALESCE(acknowledged_at,?) WHERE profile_id=? AND id=?", (now_iso(), profile_id, ident))
    if not result.rowcount:
        invalid("Inbox event not found in this book", "not_found")
    return {"id": ident, "acknowledged": True}


def dispatch(conn, profile_id, operation, args):
    handlers = {"preview": preview, "create": create, "list": list_watches, "configure": configure, "delete": delete, "inbox": inbox, "acknowledge": acknowledge}
    if operation == "evaluate":
        arguments(args, ())
        return evaluate_due(conn, profile_id)
    return handlers[operation](conn, profile_id, args)

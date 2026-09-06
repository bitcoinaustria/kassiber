"""Curated investigation schemas; execution lives at the shared daemon seam."""
from __future__ import annotations


def obj(properties, required=()):
    return {"type": "object", "additionalProperties": False, "properties": properties, "required": list(required)}


def string(maximum=256):
    return {"type": "string", "minLength": 1, "maxLength": maximum}


def integer(low, high):
    return {"type": "integer", "minimum": low, "maximum": high}


def enum(*values):
    return {"type": "string", "enum": list(values)}


QUERY = obj({
    "mode": enum("overview", "trace", "path"), "subject": string(1024), "target": string(1024),
    "chain": enum("bitcoin", "liquid"), "network": enum("main", "test", "signet", "regtest"),
    "direction": enum("backward", "forward", "both"), "depth": integer(1, 50),
    "node_limit": integer(25, 2000), "edge_limit": integer(50, 6000),
    "include_relations": {"type": "boolean"}, "include_hypotheses": {"type": "boolean"},
    "observer": enum("public", "owner", "disclosed"),
    "min_amount_msat": {"type": "string", "pattern": "^(0|[1-9][0-9]*)$"},
    "start": string(40), "end": string(40),
})
LABEL = obj({
    "id": string(64), "expected_revision": integer(1, 2147483647),
    "subject": string(), "chain": enum("bitcoin", "liquid"),
    "network": enum("main", "test", "signet", "regtest"), "label": string(),
    "category": enum("exchange", "merchant", "mixer", "service", "self", "other"),
    "source": string(512), "confidence": enum("user_confirmed", "imported", "unverified"),
    "cluster_defining": {"type": "boolean"},
}, ("subject", "chain", "network", "label", "category", "source", "confidence"))
ACQUIRE = obj({
    "backend": string(128), "subject": string(), "chain": enum("bitcoin", "liquid"),
    "network": enum("main", "test", "signet", "regtest"),
    "direction": enum("backward", "forward", "both"), "depth": integer(1, 10),
    "max_transactions": integer(1, 200), "genesis_hash": {"type": "string", "pattern": "^[a-f0-9]{64}$"},
}, ("backend", "subject", "chain", "network", "direction"))


def tool_specs():
    entries = [
        ("query", "Investigate locally stored Bitcoin/Liquid transaction connectivity, current custody relations across rails, reversible clustering hypotheses and label exposure. Use trace or path after overview; inspect frontier and evidence, and treat connectivity as distinct from ownership or taint. No network lookup occurs. Remote-provider results use opaque references valid only in this process and book; reuse them as subject/target. Hypotheses overlay the observed graph, never prove a spend or move tax basis.", QUERY, False),
        ("entropy", "Enumerate compatible input/output partitions for one Bitcoin transaction under explicit assumptions. Inspect status, limits and model assumptions before interpreting counts. Payjoin/joint payments, missing or confidential amounts are unsupported. This is model uncertainty, not ownership probabilities or a privacy score.", obj({"subject": string(), "chain": enum("bitcoin", "liquid"), "network": enum("main", "test", "signet", "regtest"), "max_states": integer(1, 200000)}, ("subject",)), False),
        ("cases.list", "List immutable investigations saved in the current book.", obj({"limit": integer(1, 100), "cursor": string(2048)}), False),
        ("cases.get", "Read a saved historical investigation, including its original query and evidence graph. A saved snapshot is not current chain status.", obj({"id": string(64)}, ("id",)), False),
        ("cases.compare", "Compare a saved investigation with another saved case or a fresh local execution of its original query. Returns actual added, removed and changed observations.", obj({"id": string(64), "other_id": string(64)}, ("id",)), False),
        ("cases.save", "Save an immutable local investigation after consent. Reuses the query and snapshot_id from query; server recomputes and refuses changed inputs. Never changes accounting.", obj({"title": string(), "query": QUERY, "expected_snapshot_id": string(64)}, ("title", "query", "expected_snapshot_id")), True),
        ("cases.delete", "Delete one saved local investigation after consent. Does not remove observations or alter accounting.", obj({"id": string(64)}, ("id",)), True),
        ("labels.list", "Read local structured attribution claims with source and revision. Labels are claims, never authority for ownership, tax treatment, guilt or taint.", obj({}), False),
        ("labels.upsert", "Create or revise a local attribution claim after consent. Include the actual evidence source; never invent entity attribution. Update using expected_revision. cluster_defining defaults false and may be true only when the user explicitly wants this claim to define a cluster.", LABEL, True),
        ("labels.delete", "Retract a local attribution claim after consent using its current revision; retains immutable label history.", obj({"id": string(64), "expected_revision": integer(1, 2147483647)}, ("id", "expected_revision")), True),
        ("acquire.plan", "On-device providers only: prepare a bounded reference acquisition from an explicitly selected configured backend. This is local preview only; no network or save occurs. Present the effects and limitations before apply.", ACQUIRE, False),
        ("acquire.apply", "On-device providers only: after once-only consent, fetch the unchanged reviewed plan from its selected backend and save sanitized reference observations locally. Egress is explicit; partial history remains a frontier. Does not sync wallets or authorize accounting.", obj({"plan": {"type": "object"}}, ("plan",)), True),
    ]
    return [{"name": f"ui.chain_analysis.{name}", "wire_name": "ui_chain_analysis_" + name.replace(".", "_"), "daemon_kind": f"ui.chain_analysis.{name}", "description": description, "parameters": schema, "kind_class": "mutating" if write else "read_only", "summary_template": "Chain analysis: " + name.replace(".", " "), "egresses": name == "acquire.apply"} for name, description, schema, write in entries]

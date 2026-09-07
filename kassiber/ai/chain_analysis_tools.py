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

SCENARIO = obj({
    "kind": enum("independent", "coinjoin_intrafees"),
    "protocol": enum("generic", "whirlpool", "joinmarket", "wabisabi"),
    "max_received_fee_msat": {"type": "string", "pattern": "^(0|[1-9][0-9]*)$"},
    "max_paid_fee_msat": {"type": "string", "pattern": "^(0|[1-9][0-9]*)$"},
}, ("kind",))
COMPUTATION = {"max_states": integer(1, 2000000), "max_duration_ms": integer(1, 30000), "scenario": SCENARIO}
ENTROPY = obj({"subject": string(1024), "chain": enum("bitcoin", "liquid"), "network": enum("main", "test", "signet", "regtest"), "observer": enum("owner", "public", "disclosed"), **COMPUTATION}, ("subject",))
PSBT = {"psbt_token": string(128), "network": enum("main", "test", "signet", "regtest")}
PAYJOIN = obj({
    "payment_output_index": integer(0, 100000), "additional_fee_output_index": integer(0, 100000),
    "max_additional_fee_contribution_msat": {"type": "string", "pattern": "^(0|[1-9][0-9]*)$"},
    "allow_output_substitution": {"type": "boolean"},
    "minimum_fee_rate_sat_vb": {"type": "string", "pattern": "^[0-9]+(?:\\.[0-9]+)?$"},
}, ("payment_output_index",))
MANIFEST = obj({
    "dataset_key": string(128), "name": string(), "version": string(128),
    "chain": enum("bitcoin", "liquid"), "network": enum("main", "test", "signet", "regtest"),
    "source": string(512), "license": string(512), "attribution_method": string(),
    "visibility": enum("public", "private"), "source_url": string(1024),
    "observed_at": string(40), "valid_from": string(40), "valid_until": string(40),
    "category": enum("exchange", "merchant", "mixer", "service", "self", "other"),
    "expected_active_id": string(64),
}, ("dataset_key", "name", "version", "chain", "network", "source", "license", "attribution_method", "visibility"))
DATASET_SOURCE = {"source_token": string(128), "manifest": MANIFEST, "format": enum("csv", "jsonl"), "adapter": enum("generic", "am_i_exposed", "maru92")}


def tool_specs():
    entries = [
        ("query", "Investigate locally stored Bitcoin/Liquid transaction connectivity, current custody relations across rails, reversible clustering hypotheses and label exposure. Use trace or path after overview; inspect frontier and evidence, and treat connectivity as distinct from ownership or taint. No network lookup occurs. Remote-provider results use opaque references valid only in this process and book; reuse them as subject/target. Hypotheses overlay the observed graph, never prove a spend or move tax basis.", QUERY, False),
        ("entropy", "Compute exact conditional Bitcoin flow partitions, optionally with explicit participant-fee bounds. Protocol names describe a scenario and never prove a protocol or its fees. Inspect status, assumptions and limits; partial computations withhold exact linkage. Payjoin/joint payments require PSBT comparison, not this independent-flow model.", ENTROPY, False),
        ("entropy.start", "Start a bounded local computation on immutable observer-visible inputs. Use jobs.get to read progress/result and jobs.cancel to stop. The receipt remains tied to its submitted snapshot. Never infer progress percentage or exact links from partial counts.", ENTROPY, False),
        ("jobs.get", "Read a process/book-scoped analysis computation receipt and its actual progress or terminal result.", obj({"job_id": string(128)}, ("job_id",)), False),
        ("jobs.cancel", "Cancel a running local analysis computation. Does not mutate observations or accounting.", obj({"job_id": string(128)}, ("job_id",)), False),
        ("psbt.analyze", "Analyze a PSBT the user explicitly selected in the local investigation workspace. Reuse only its opaque psbt_token; no file paths or PSBT text. Validates v0/v2 and UTXO evidence and returns safe public structural features. Missing values remain unknown; this never signs or broadcasts.", obj(PSBT, ("psbt_token", "network")), False),
        ("psbt.compare", "Compare explicitly selected original/proposal PSBT source tokens. Optional payjoin parameters must come from the actual negotiation/user, not guessed. Reports changed inputs, outputs, amounts, features and BIP78 checks; it never authorizes signing.", obj({"before_token": string(128), "after_token": string(128), "network": enum("main", "test", "signet", "regtest"), "payjoin": PAYJOIN}, ("before_token", "after_token", "network")), False),
        ("psbt.entropy.start", "Start the same conditional flow-partition computation for a locally selected PSBT. Source bytes and private maps remain local; results are bound to the submitted proposal. Read/cancel through jobs.get/jobs.cancel.", obj({**PSBT, **COMPUTATION}, ("psbt_token", "network")), False),
        ("datasets.list", "List locally imported attribution pack versions and lifecycle states. Packs are sourced claims, never identity proof or custody authority. No dataset download occurs.", obj({"limit": integer(1, 100), "cursor": string(4096)}), False),
        ("datasets.get", "Read one local attribution pack's manifest, version, content digest and state.", obj({"id": string(64)}, ("id",)), False),
        ("datasets.query", "Query exact local attribution claims by subject and chain/network, or exact label within a dataset_id. Inspect conflicting claims, source methodology, validity and historical dataset_status. Absence does not prove anonymity.", obj({"subject": string(1024), "chain": enum("bitcoin", "liquid"), "network": enum("main", "test", "signet", "regtest"), "dataset_id": string(64), "label": string(), "observer": enum("owner", "public", "disclosed"), "limit": integer(1, 100), "cursor": string(4096)}), False),
        ("datasets.preview", "On-device providers only: validate the selected local dataset token and user-supplied provenance manifest. Streams the entire source, returns its hash/counts/sample and performs no import. Never invent source, methodology or license claims.", obj(DATASET_SOURCE, ("source_token", "manifest")), False),
        ("datasets.import", "On-device providers only: after consent, import the explicitly previewed local source, manifest and expected_sha256. Replacing an active version requires its expected_active_id. The hash is checked before atomic activation; prior evidence remains versioned.", obj({**DATASET_SOURCE, "expected_sha256": {"type": "string", "pattern": "^[a-f0-9]{64}$"}}, ("source_token", "manifest", "expected_sha256")), True),
        ("datasets.revoke", "Retract one local attribution pack after consent with its expected_revision. Keeps historical versions; future matching no longer uses the revoked pack.", obj({"id": string(64), "expected_revision": integer(1, 2147483647)}, ("id", "expected_revision")), True),
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
    watch = obj({"rule": enum("output_spent", "confirmations", "connection_supported", "attribution_changed", "findings_changed"), "query": QUERY, "case_id": string(64), "threshold": integer(1, 10000)}, ("rule",))
    entries.extend([
        ("watches.preview", "Preview a typed local evidence watch. Choose a scoped query or saved case. Records a baseline only on create; no network or model runs in background.", watch, False),
        ("watches.create", "After consent, create the unchanged reviewed local watch preview in the encrypted book. Re-preview after evidence or book changes.", obj({"definition": obj({**watch["properties"], "rule_version": integer(1, 1)}, ("rule",)), "expected_plan_id": string(64)}, ("definition", "expected_plan_id")), True),
        ("watches.list", "List local evidence watches in this book.", obj({}), False),
        ("watches.inbox", "Read durable local evidence changes; lost coverage is not resolution.", obj({"limit": integer(1, 100), "before": integer(1, 9007199254740991)}), False),
        ("watches.configure", "After consent, pause or resume a watch using its current revision. Resume catches up from its saved baseline.", obj({"id": string(64), "expected_revision": integer(1, 2147483647), "enabled": {"type": "boolean"}}, ("id", "expected_revision", "enabled")), True),
        ("watches.delete", "After consent, delete a local watch and its inbox history. Leaves evidence and accounting unchanged.", obj({"id": string(64), "expected_revision": integer(1, 2147483647)}, ("id", "expected_revision")), True),
        ("watches.acknowledge", "After consent, acknowledge one inbox event; this never changes evidence or accounting.", obj({"id": string(64)}, ("id",)), True),
        ("watches.evaluate", "After consent, evaluate enabled local watches against committed local evidence now; no acquisition or model call.", obj({}), True),
    ])
    entries.append(("datasets.discard", "After consent, remove only incomplete failed/staging dataset imports in bounded batches. Complete versions retain their provenance; revoke them instead.", obj({"id": string(64)}, ("id",)), True))
    for operation in ("datasets.preview", "datasets.import", "datasets.discard"):
        name, description, schema, write = next(entry for entry in entries if entry[0] == operation)
        entries.append((name + ".start", "Start cancellable background work; poll jobs.get and use jobs.cancel to stop. Preferred for large files. " + description, schema, write))
    return [{"name": f"ui.chain_analysis.{name}", "wire_name": "ui_chain_analysis_" + name.replace(".", "_"), "daemon_kind": f"ui.chain_analysis.{name}", "description": description, "parameters": schema, "kind_class": "mutating" if write else "read_only", "summary_template": "Chain analysis: " + name.replace(".", " "), "egresses": name == "acquire.apply"} for name, description, schema, write in entries]

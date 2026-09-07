"""On-device-only recurring acquisition tools; no implicit source selection."""
from .chain_analysis_tools import obj, string, integer, enum


def source_tool_specs():
    recipe = obj({
        "backend": string(128), "chain": enum("bitcoin"), "network": enum("main", "test", "signet", "regtest"),
        "chain_instance_id": string(128), "mode": enum("subject", "blocks"), "subject": string(64),
        "start_height": integer(0, 2147483647), "end_height": integer(0, 2147483647),
        "interval_seconds": integer(30, 86400), "duration_days": integer(1, 365),
        "max_requests": integer(4, 100000000), "max_bytes": integer(8388608, 10000000000000), "blocks_per_run": integer(1, 16),
    }, ("backend", "network", "mode", "max_requests", "max_bytes"))
    entries = [
        ("plan", "On-device only. Preview recurring acquisition using the user's chosen Core connection, network, block range or exact txid and lifetime request/byte budgets. Offline; never invent permissions or silently choose a public source.", recipe, False),
        ("authorize", "On-device only. After once-only consent to server-recomputed effects, authorize this exact plan for recurring requests while the book is unlocked and the daemon runs. This is a new persistent revocable permission, not reuse of an earlier one-shot approval.", obj({"plan": {"type": "object"}}, ("plan",)), True),
        ("list", "On-device only. List source authorizations, progress, lifetime quota consumption, expiry and pause reasons in this book. No network request.", obj({"limit": integer(1, 100), "after_id": string(64)}), False),
        ("revoke", "On-device only. Revoke a recurring source authorization after consent. Preserve acquired observations; stop future requests and fence in-flight publication.", obj({"id": string(64), "expected_revision": integer(1, 2147483647)}, ("id", "expected_revision")), True),
        ("run", "On-device only. Queue the next bounded batch of an already authorized source. Requires a running unlocked daemon. Does not widen the approved source, lifetime or budgets.", obj({"id": string(64)}, ("id",)), True),
    ]
    return [{"name": f"ui.chain_analysis.sources.{name}", "wire_name": f"ui_chain_analysis_sources_{name}", "daemon_kind": f"ui.chain_analysis.sources.{name}",
             "description": description, "parameters": schema, "kind_class": "mutating" if write else "read_only",
             "summary_template": f"Recurring source: {name}", "egresses": name == "run"} for name, description, schema, write in entries]

"""CLI grammar for the shared investigation API; no accounting interpretation."""
from __future__ import annotations

import json
from pathlib import Path

from ..core import chain_analysis_api
from ..errors import AppError


def add_parser(sub):
    root = sub.add_parser("chain-analysis", help="Local transaction investigations, tracing and evidence")
    commands = root.add_subparsers(dest="chain_analysis_command", required=True)
    for mode in ("overview", "trace", "path"):
        parser = commands.add_parser(mode)
        if mode != "overview":
            parser.add_argument("subject")
        if mode == "path":
            parser.add_argument("target")
        parser.add_argument("--chain", choices=("bitcoin", "liquid"))
        parser.add_argument("--network", choices=("main", "test", "signet", "regtest"))
        parser.add_argument("--direction", choices=("backward", "forward", "both"), default="both")
        parser.add_argument("--depth", type=int, default=4)
        parser.add_argument("--node-limit", type=int, default=400)
        parser.add_argument("--edge-limit", type=int, default=1200)
        parser.add_argument("--observer", choices=("public", "owner", "disclosed"), default="owner")
        parser.add_argument("--no-relations", action="store_true")
        parser.add_argument("--include-hypotheses", action="store_true")
        parser.add_argument("--min-amount-msat")
        parser.add_argument("--start")
        parser.add_argument("--end")
        _scope(parser)
    watches = commands.add_parser("watches", help="Local-only evidence watches and inbox").add_subparsers(dest="chain_analysis_action", required=True)
    for action in ("preview", "create", "list", "configure", "delete", "inbox", "acknowledge", "evaluate"):
        parser = watches.add_parser(action)
        _scope(parser)
        if action in {"preview", "configure", "delete"}:
            parser.add_argument("--document", required=True, help="Typed watch definition/configuration JSON or @path")
        elif action == "create":
            parser.add_argument("--plan", required=True, help="Unchanged reviewed preview JSON or @path")
        elif action == "acknowledge":
            parser.add_argument("id")
        elif action == "inbox":
            parser.add_argument("--limit", type=int, default=50)
            parser.add_argument("--before", type=int)
    entropy = commands.add_parser("entropy")
    entropy.add_argument("subject")
    entropy.add_argument("--chain", choices=("bitcoin", "liquid"))
    entropy.add_argument("--network", choices=("main", "test", "signet", "regtest"))
    entropy.add_argument("--max-states", type=int, default=200000)
    entropy.add_argument("--max-duration-ms", type=int, default=1000)
    entropy.add_argument("--observer", choices=("public", "owner", "disclosed"), default="owner")
    entropy.add_argument("--scenario", help="Explicit fee model JSON or @path; never inferred participant economics")
    _scope(entropy)
    psbt = commands.add_parser("psbt").add_subparsers(dest="chain_analysis_action", required=True)
    for action in ("analyze", "compare", "entropy"):
        parser = psbt.add_parser(action)
        _scope(parser)
        parser.add_argument("--network", choices=("main", "test", "signet", "regtest"), required=True)
        if action in {"analyze", "entropy"}:
            parser.add_argument("--file", required=True, help="Local PSBT v0/v2 file (binary, base64 or hex)")
            if action == "entropy":
                parser.add_argument("--scenario", help="Explicit fee scenario JSON or @path")
                parser.add_argument("--max-states", type=int, default=200000)
                parser.add_argument("--max-duration-ms", type=int, default=1000)
        else:
            parser.add_argument("--before", required=True, help="Original PSBT file")
            parser.add_argument("--after", required=True, help="Proposed PSBT file")
            parser.add_argument("--payjoin", help="Explicit BIP78 comparison constraints JSON or @path")
    datasets = commands.add_parser("datasets").add_subparsers(dest="chain_analysis_action", required=True)
    for action in ("list", "get", "query", "preview", "import", "revoke", "discard"):
        parser = datasets.add_parser(action)
        _scope(parser)
        if action == "list":
            parser.add_argument("--limit", type=int, default=50)
            parser.add_argument("--cursor")
        elif action == "query":
            parser.add_argument("--subject")
            parser.add_argument("--chain", choices=("bitcoin", "liquid"))
            parser.add_argument("--network", choices=("main", "test", "signet", "regtest"))
            parser.add_argument("--dataset-id")
            parser.add_argument("--label", help="Exact entity name within a selected dataset")
            parser.add_argument("--observer", choices=("owner", "public", "disclosed"), default="owner")
            parser.add_argument("--limit", type=int, default=50)
            parser.add_argument("--cursor")
        elif action in {"get", "revoke", "discard"}:
            parser.add_argument("id")
            if action == "revoke":
                parser.add_argument("--expected-revision", type=int, required=True)
        else:
            parser.add_argument("--file", required=True, help="Local attribution CSV or JSONL; streamed without loading the full dataset")
            parser.add_argument("--manifest", required=True, help="Dataset provenance JSON or @path")
            parser.add_argument("--source-format", choices=("csv", "jsonl"), default="csv")
            parser.add_argument("--adapter", choices=("generic", "am_i_exposed", "maru92"), default="generic")
            if action == "import":
                parser.add_argument("--expected-sha256", required=True, help="Source SHA256 returned by preview")
    for group, actions in (("cases", ("list", "get", "save", "compare", "delete")), ("labels", ("list", "upsert", "import", "delete")), ("acquire", ("plan", "apply"))):
        nested = commands.add_parser(group).add_subparsers(dest="chain_analysis_action", required=True)
        for action in actions:
            parser = nested.add_parser(action)
            _scope(parser)
            if group == "cases" and action in {"get", "compare", "delete"} or group == "labels" and action == "delete":
                parser.add_argument("id")
            if group == "cases" and action == "compare":
                parser.add_argument("--other-id")
            if group == "cases" and action == "list":
                parser.add_argument("--limit", type=int, default=50)
                parser.add_argument("--cursor")
            if group == "cases" and action == "save":
                parser.add_argument("--title", required=True)
                parser.add_argument("--query", required=True, help="Query JSON or @path to a query JSON file")
                parser.add_argument("--expected-snapshot-id", required=True)
            if group == "labels" and action in {"upsert", "import"}:
                parser.add_argument("--document", required=True, help="Label JSON (import: {items:[...]}) or @path")
            if group == "labels" and action == "delete":
                parser.add_argument("--expected-revision", type=int, required=True)
            if group == "acquire" and action == "plan":
                parser.add_argument("subject")
                parser.add_argument("--backend", required=True)
                parser.add_argument("--chain", choices=("bitcoin", "liquid"), required=True)
                parser.add_argument("--network", choices=("main", "test", "signet", "regtest"), required=True)
                parser.add_argument("--direction", choices=("backward", "forward", "both"), default="both")
                parser.add_argument("--depth", type=int, default=3)
                parser.add_argument("--max-transactions", type=int, default=50)
                parser.add_argument("--genesis-hash")
            if group == "acquire" and action == "apply":
                parser.add_argument("--plan", required=True, help="Unchanged reviewed plan JSON or @path; this contacts the selected backend")


def _scope(parser):
    parser.add_argument("--workspace")
    parser.add_argument("--profile")


def _json(value):
    try:
        if value.startswith("@"):
            path = Path(value[1:]).expanduser()
            if path.stat().st_size > 10_000_000:
                raise ValueError("JSON document exceeds 10 MB")
            value = path.read_text(encoding="utf-8")
        parsed = json.loads(value)
        # Machine output files can be fed directly into the next operation.
        if isinstance(parsed, dict) and set(parsed) >= {"kind", "schema_version", "data"}:
            parsed = parsed["data"]
        if not isinstance(parsed, dict):
            raise ValueError("JSON object required")
        return parsed
    except (OSError, ValueError) as exc:
        raise AppError("Invalid chain analysis JSON document", code="validation") from exc


def dispatch(conn, args):
    command = args.chain_analysis_command
    if command in {"overview", "trace", "path"}:
        fields = ("subject", "target", "chain", "network", "direction", "depth", "node_limit", "edge_limit", "observer", "include_hypotheses", "min_amount_msat", "start", "end")
        payload = {key: getattr(args, key) for key in fields if getattr(args, key, None) is not None}
        payload.update(mode=command, include_relations=not args.no_relations)
        operation = "query"
    elif command == "entropy":
        payload = {key: getattr(args, key) for key in ("subject", "chain", "network", "observer", "max_states", "max_duration_ms") if getattr(args, key, None) is not None}
        if args.scenario:
            payload["scenario"] = _json(args.scenario)
        operation = command
    elif command == "psbt":
        operation = f"psbt.{args.chain_analysis_action}"
        payload = {"network": args.network}
        fields = {"before": args.before, "after": args.after} if args.chain_analysis_action == "compare" else {"psbt": args.file}
        for key, filename in fields.items():
            try:
                with Path(filename).expanduser().open("rb") as stream:
                    payload[key] = stream.read(4 * 1024 * 1024 + 1)
            except OSError:
                raise AppError("Cannot read selected PSBT file", code="chain_analysis_source_unavailable") from None
        if getattr(args, "payjoin", None):
            payload["payjoin"] = _json(args.payjoin)
        if args.chain_analysis_action == "entropy":
            payload.update(max_states=args.max_states, max_duration_ms=args.max_duration_ms)
            if args.scenario:
                payload["scenario"] = _json(args.scenario)
    elif command == "datasets" and args.chain_analysis_action in {"preview", "import"}:
        payload = {"manifest": _json(args.manifest), "format": args.source_format, "adapter": args.adapter}
        if args.chain_analysis_action == "import":
            payload["expected_sha256"] = args.expected_sha256
        try:
            with Path(args.file).expanduser().open("rb") as stream:
                return chain_analysis_api.dispatch(conn, f"ui.chain_analysis.datasets.{args.chain_analysis_action}", payload, workspace=args.workspace, profile=args.profile, source_stream=stream)
        except OSError:
            raise AppError("Cannot read selected dataset file", code="chain_analysis_source_unavailable") from None
    else:
        action = args.chain_analysis_action
        operation = f"{command}.{action}"
        allowed = ("id", "other_id", "limit", "cursor", "title", "expected_snapshot_id", "expected_revision")
        if command == "watches" and action == "inbox":
            allowed = ("limit", "before")
        if command == "datasets" and action == "query":
            allowed = ("subject", "chain", "network", "dataset_id", "label", "observer", "limit", "cursor")
        if command == "acquire" and action == "plan":
            allowed = ("backend", "subject", "chain", "network", "direction", "depth", "max_transactions", "genesis_hash")
        payload = {key: getattr(args, key) for key in allowed if getattr(args, key, None) is not None}
        if getattr(args, "document", None):
            payload = _json(args.document)
        if getattr(args, "query", None):
            payload["query"] = _json(args.query)
        if getattr(args, "plan", None):
            payload["plan"] = _json(args.plan)
    return chain_analysis_api.dispatch(conn, f"ui.chain_analysis.{operation}", payload, workspace=args.workspace, profile=args.profile)

"""Write crates/ai/tests/data/vectors.json: Python's answers for the pure AI parts.

    uv run --locked python crates/ai/scripts/gen_vectors.py [--check]

Each section feeds one Rust test in ``crates/ai/tests/python_vectors.rs``:
tool capabilities, per-turn selection, schema validation (catalog tools and
synthetic schemas), argument and result redaction, consent summaries, call
parsing, denials, the auto-read planner, context trimming, and provenance
with the privacy receipt. Inputs are generated deterministically from the
catalog and from the string literals of the Python rules, so a keyword the
Rust port drops shows up as a failing case. ``tests/test_rust_ai_tools.py``
checks that the file still matches Python.
"""

from __future__ import annotations

import ast
import copy
import inspect
import json
import re
import sys
import textwrap
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "vectors.json"
sys.path.insert(0, str(ROOT))

from kassiber import daemon  # noqa: E402
from kassiber.ai import prompt, tools  # noqa: E402
from kassiber.errors import AppError  # noqa: E402

MAX_TOOL_CASES = 60


def outcome(callable_, *args, **kwargs):
    try:
        return {"ok": callable_(*args, **kwargs)}
    except AppError as exc:
        return {
            "error": {
                "code": exc.code,
                "message": str(exc),
                "details": exc.details,
                "hint": exc.hint,
                "retryable": exc.retryable,
            }
        }
    except Exception as exc:  # noqa: BLE001 - recorded as an unexpected failure
        return {"exception": type(exc).__name__}


def literals(function) -> list[str]:
    source = textwrap.dedent(inspect.getsource(function))
    found: list[str] = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            if 2 <= len(node.value) <= 40 and "\n" not in node.value and node.value not in found:
                found.append(node.value)
    return found


# Selection ------------------------------------------------------------------


def wire_names(definitions):
    return [item["name"] for item in definitions]


def selection_cases():
    words = literals(tools.select_tool_capabilities) + literals(tools._continues_review_checkpoint)
    cases = []
    base = [
        (None, None, None),
        (None, None, "core"),
        (None, None, "full"),
        (None, None, "scoped"),
        ([], None, None),
        ([], None, "core"),
        ([{"role": "user", "content": "What can you do? Show all tools."}], None, None),
        ([{"role": "user", "content": "What can you do?"}], None, "core"),
        ([{"role": "user", "content": "Export my 2025 Austrian tax report"}], {"route": "/reports"}, None),
        ([{"role": "user", "content": "Explain this transaction"}], {"route": "/transactions"}, None),
        ([{"role": "user", "content": "Show the treasury across all books"}], {"route": "/books"}, None),
        ([{"role": "user", "content": "Help me resolve these"}], {"route": "/quarantine"}, None),
        ([{"role": "user", "content": "Help"}], {"route": "/BOOKS"}, None),
        ([{"role": "user", "content": "Help"}], {"route": 7}, None),
        ([{"role": "user", "content": "Help"}], {"route": None, "capabilities": ["loans", "bogus", 3, "merchant"]}, None),
        ([{"role": "user", "content": "Help"}], {"capabilities": "loans"}, None),
        ([{"role": "user", "content": "Resolve my quarantine"}], None, "core"),
        ([{"role": "user", "content": "Bitte hilf mir, die Quarantäne zu lösen"}], None, None),
        ([{"role": "assistant", "content": "x"}, {"role": "user", "content": 5}], None, None),
        (
            [
                {"role": "user", "content": "first loan"},
                {"role": "user", "content": "second wallet"},
                {"role": "user", "content": "third"},
                {"role": "user", "content": "fourth report"},
            ],
            None,
            None,
        ),
    ]
    checkpoint = "Paused.\n```json\n" + json.dumps({"review_checkpoint": {"cases": ["c1"]}}) + "\n```"
    for continuation in ("continue", "Continue.", "  bitte weiter!? ", "weiter?", "go on"):
        for content in (
            checkpoint,
            "```json\n{\"review_checkpoint\": {}}\n```",
            "```json\n{\"review_checkpoint\": {\"a\": 1}, \"x\": 2}\n```",
            "text```json\n{\"review_checkpoint\": {\"a\": 1}}\n```",
            "```json \t\r\n{\"review_checkpoint\": {\"a\": 1}}\r\n```\n",
            "```json\n{not json}\n```\n```json\n{\"review_checkpoint\": {\"a\": 1}}\n```",
            "```json\n{\"review_checkpoint\": {\"a\": 1}, \"review_checkpoint\": {\"b\": 2}}\n```",
            "x" * 17000 + checkpoint,
            checkpoint + "x" * 17000,
            12,
        ):
            base.append(
                (
                    [
                        {"role": "user", "content": "review"},
                        {"role": "assistant", "content": content},
                        {"role": "user", "content": continuation},
                    ],
                    None,
                    None,
                )
            )
    for word in words:
        for template in ("please {} now", "{}", "Tell me about {} TODAY"):
            base.append(([{"role": "user", "content": template.format(word)}], None, None))
        base.append(([{"role": "user", "content": "hi"}], {"route": word}, None))
    for profile in ("core", "scoped", "full"):
        base.append(([{"role": "user", "content": "my wallet transfer and loan report"}], None, profile))
    for messages, screen_context, profile in base:
        case = {"messages": messages, "screen_context": screen_context, "profile": profile}
        result = outcome(prompt.build_responses_tools, copy.deepcopy(messages), screen_context=screen_context, profile=profile)
        # Catalog indexes keep the file small; order is the catalog's.
        index = {tool.provider_name: position for position, tool in enumerate(tools.TOOL_CATALOG)}
        case["result"] = {"tools": [index[name] for name in wire_names(result["ok"])]} if "ok" in result else result
        selected = outcome(tools.select_tool_capabilities, copy.deepcopy(messages), screen_context)
        case["capabilities"] = sorted(selected["ok"]) if selected.get("ok") is not None else selected.get("ok")
        cases.append(case)
    cases.append(
        {
            "messages": None,
            "screen_context": None,
            "profile": "bogus",
            "result": outcome(prompt.build_responses_tools, None, profile="bogus"),
            "capabilities": None,
        }
    )
    return cases


# Validation -----------------------------------------------------------------

GENERIC = [None, True, False, 0, 1, -1, 2**31, 2**70, 1.5, -0.5, 1e300, "", "a", " x ", [], [1, 2], [1, 1], {}, {"k": 1}]


def fill(schema, depth=0):
    """A plausible value for a required field."""
    if not isinstance(schema, dict) or depth > 4:
        return None
    if isinstance(schema.get("enum"), list) and schema["enum"]:
        return schema["enum"][0]
    types = schema.get("type")
    kind = types[0] if isinstance(types, list) and types else types
    pattern = schema.get("pattern")
    if pattern == "^(0|[1-9][0-9]*)$" or pattern == "^[0-9]+(?:\\.[0-9]+)?$":
        return "1"
    if pattern == "^[a-f0-9]{64}$":
        return "a" * 64
    if pattern == "^quarantine:":
        return "quarantine:x"
    if kind == "string":
        return "x" * max(1, schema.get("minLength", 1) if isinstance(schema.get("minLength"), int) else 1)
    if kind in {"integer", "number"}:
        minimum = schema.get("minimum")
        return minimum if isinstance(minimum, int) else 1
    if kind == "boolean":
        return True
    if kind == "array":
        item = fill(schema.get("items"), depth + 1)
        count = schema.get("minItems") if isinstance(schema.get("minItems"), int) else 0
        return [item] * count if item is not None else []
    if kind == "object":
        props = schema.get("properties") if isinstance(schema.get("properties"), dict) else {}
        return {key: fill(props.get(key), depth + 1) for key in schema.get("required", []) if isinstance(key, str)}
    return None


def candidates(schema):
    values = list(GENERIC)
    if not isinstance(schema, dict):
        return values
    if isinstance(schema.get("enum"), list):
        values.extend(schema["enum"][:2])
        values.append("not-an-allowed-value")
    for key in ("minLength", "maxLength"):
        bound = schema.get(key)
        if isinstance(bound, int):
            values.extend(["y" * max(bound - 1, 0), "y" * bound, "y" * (bound + 1), "é" * bound, "é" * (bound + 1)])
    for key in ("minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"):
        bound = schema.get(key)
        if isinstance(bound, (int, float)) and not isinstance(bound, bool):
            values.extend([bound - 1, bound, bound + 1, float(bound) - 0.5, float(bound) + 0.5])
    if isinstance(schema.get("pattern"), str):
        values.extend(["12", "12\n", "12\n\n", "01", "1.5", "1.", "a" * 64, "a" * 64 + "\n", "A" * 64, "quarantine:x", "x quarantine:", "\u0661\u0662"])
    for key in ("minItems", "maxItems"):
        bound = schema.get(key)
        if isinstance(bound, int):
            item = fill(schema.get("items"))
            values.extend([[item] * max(bound - 1, 0), [item] * bound, [item] * (bound + 1)])
    if isinstance(schema.get("items"), dict):
        values.extend([[None], [{}], ["s"], [1.5]])
    return values


def validation_cases():
    cases = []
    for tool in tools.TOOL_CATALOG:
        schema = tool.parameters
        props = schema.get("properties", {}) if isinstance(schema, dict) else {}
        required = [key for key in schema.get("required", []) if isinstance(key, str)]
        base = {key: fill(props.get(key)) for key in required}
        tool_cases = [{}, base, {**base, "zz_unknown": 1}, {"debug": True, **base}]
        if tool.name in {"status", "ui.transactions.search", "ui.transfers.components.plan"}:
            # The size limit is checked first, on the ASCII-escaped length.
            # `{"zz":"` and `"}` add nine characters: 65,536 is the last size accepted.
            tool_cases += [{"zz": "x" * 65527}, {"zz": "x" * 65528}, {"zz": "é" * 10921}, {"zz": "é" * 10922}]
        for name, child in props.items():
            for value in candidates(child):
                tool_cases.append({name: value, **{k: v for k, v in base.items() if k != name}})
            if isinstance(child, dict) and isinstance(child.get("properties"), dict):
                inner_required = fill(child)
                for inner, inner_schema in list(child["properties"].items())[:6]:
                    for value in candidates(inner_schema)[:12]:
                        tool_cases.append({name: {inner: value, **{k: v for k, v in inner_required.items() if k != inner}}, **{k: v for k, v in base.items() if k != name}})
            if isinstance(child, dict) and isinstance(child.get("items"), dict) and isinstance(child["items"].get("properties"), dict):
                item_required = fill(child["items"])
                for inner, inner_schema in list(child["items"]["properties"].items())[:6]:
                    for value in candidates(inner_schema)[:10]:
                        tool_cases.append({name: [{inner: value, **{k: v for k, v in item_required.items() if k != inner}}], **{k: v for k, v in base.items() if k != name}})
        seen = set()
        kept = 0
        for arguments in tool_cases:
            key = json.dumps(arguments, sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            result = outcome(daemon._validate_ai_tool_arguments, tool, copy.deepcopy(arguments))
            cases.append({"tool": tool.name, "arguments": arguments, "result": result if "ok" not in result else {"ok": None}})
            kept += 1
            if kept >= MAX_TOOL_CASES and len(tool_cases) > MAX_TOOL_CASES:
                # Keep the tail: nested and item cases come last.
                break
        for arguments in tool_cases[-40:]:
            key = json.dumps(arguments, sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            result = outcome(daemon._validate_ai_tool_arguments, tool, copy.deepcopy(arguments))
            cases.append({"tool": tool.name, "arguments": arguments, "result": result if "ok" not in result else {"ok": None}})
    return cases


SYNTHETIC = [
    ({"type": "integer", "enum": [1, 2]}, ["1", 3, 1, True, 1.0, 2.0]),
    ({"type": ["string", "null"], "enum": ["a", None]}, ["a", None, "b", 1]),
    ({"enum": [1, "a", [1, 2], {"k": 1}]}, [True, 1.0, "a", [1, 2], [1.0, 2], {"k": True}, {"k": 2}, [2, 1]]),
    ({"type": "number"}, [1, 1.5, True, "1"]),
    ({"type": "object", "required": ["a", "b", 3], "properties": {"a": {"type": "string"}}, "additionalProperties": False}, [{}, {"a": "x"}, {"a": 1, "b": 2}, {"a": "x", "b": 1, "c": 2}, {"c": 1}]),
    ({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": False}, [{"c": 1, "a": 2}, {"b": 1, "a": "x"}, {"a": 5}]),
    ({"type": "object", "additionalProperties": {"type": "boolean"}}, [{"x": True}, {"x": 1}, {"x": True, "y": "n"}]),
    ({"type": "object", "properties": {"a": True}, "additionalProperties": {"type": "integer"}}, [{"a": "s"}, {"b": "s"}]),
    ({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "string"}}}, [{"b": 1, "a": "x"}, {"a": "x", "b": 1}, {"z": 1}]),
    ({"type": "array", "minItems": 2, "maxItems": 3, "uniqueItems": True, "items": {"type": "integer", "minimum": 0}}, [[1], [1, 1], [1, 2], [1, -1], [1, 2, 3, 4], [1, True], [1, 1.0], [-1, -1], [[1], [1]], [{"a": 1, "b": 2}, {"b": 2, "a": 1}]]),
    ({"type": "array", "items": {"type": "object", "required": ["k"]}}, [[{}], [{"k": 1}, 5]]),
    ({"type": "string", "minLength": 2, "maxLength": 3, "pattern": "^[a-f0-9]+$"}, ["a", "ab", "abcd", "xy", "ab\n", "\U0001F600\U0001F600", "zz\n"]),
    ({"type": "string", "pattern": "^(0|[1-9][0-9]*)$"}, ["0", "00", "10", "10\n", "10\n\n", "\n10", "1\u0662"]),
    ({"type": "string", "pattern": "^[0-9]+(?:\\.[0-9]+)?$"}, ["1", "1.5", "1.", ".5", "1.5\n"]),
    ({"type": "string", "pattern": "^quarantine:"}, ["quarantine:", "x quarantine:", "quarantine:\n", "Quarantine:"]),
    ({"type": "integer", "exclusiveMinimum": 0, "exclusiveMaximum": 10, "minimum": 2, "maximum": 8}, [0, 1, 2, 8, 9, 10, 11]),
    ({"type": "number", "exclusiveMinimum": 0.5, "maximum": 1}, [0, 0.5, 0.6, 1, 1.0000001, 2**80]),
    ({"type": ["integer", "string"], "minimum": 1, "minLength": 2}, [0, "a", "ab", 5]),
    ({"type": "object", "properties": {"a": {"type": "object", "properties": {"b": {"type": "string"}}}}}, [{"a": {"b": 1}}, {"a": []}]),
    ({"minItems": 1, "minLength": 1, "minimum": 1}, [[], "", 0, None, {}]),
    ({"type": "bogus"}, [1, None]),
    ({"type": []}, [1]),
    ({"type": [3, "string"]}, ["a", 1]),
    ({"enum": "a"}, ["a", "b"]),
    ({"minItems": True, "maxLength": True, "maximum": True}, [[], [1, 2], "ab", 2]),
    ({"pattern": 5, "type": "string"}, ["x"]),
]


def deep_schema(levels):
    schema = {"type": "string"}
    for _ in range(levels):
        schema = {"type": "object", "properties": {"a": schema}}
    return schema


def deep_value(levels):
    value = "x"
    for _ in range(levels):
        value = {"a": value}
    return value


def synthetic_cases():
    cases = []
    for schema, values in SYNTHETIC:
        for value in values:
            cases.append({"schema": schema, "value": value, "result": outcome(daemon._validate_ai_schema_value, copy.deepcopy(value), schema, path="t")})
    for levels in (15, 16, 17, 18):
        cases.append({"schema": deep_schema(levels), "value": deep_value(levels), "result": outcome(daemon._validate_ai_schema_value, deep_value(levels), deep_schema(levels), path="t")})
    for result in cases:
        if "ok" in result["result"]:
            result["result"] = {"ok": None}
    return cases


# Redaction and summaries ----------------------------------------------------

REDACTION_VALUES = [
    {"message": "Request https://private.example/secret?token=value failed while reading /Users/alice/Taxes/private.pdf", "nested": ["C:\\Users\\Alice\\Documents\\private.csv", "\\\\server\\private-share\\audit\\statement.pdf"], "asset_route": "BTC/LBTC/swap"},
    "Saved to /Users/alice/Alice's Taxes/private (final), #1.pdf then uploaded",
    {"route": "/transactions"},
    {"route": "/a/b"},
    "see ~/notes/tax.txt",
    "see ~\\notes\\tax.txt",
    "word/with/slashes and x/y",
    "a_/b/c",
    "1/2/3",
    "HTTPS://Example.COM/x and mailto:x@y.z and git+ssh://h/p 'q' <a>",
    "ſtp://kelvin.example/x",
    "ftp://example.com",
    "\u2028/usr/lib/x",
    "/usr/lib",
    "//",
    "D:/x/y",
    "D:/x",
    {"wallet": "cold", "descriptor": "wpkh(xpub...)", "mnemonic": "abandon abandon abandon", "recovery_phrase": "abandon", "seed": "00" * 32, "wif": "Kxabc", "xprv": "xprv9s21...", "nested": {"api_token": "secret"}},
    {"ok": False, "envelope": {"kind": "error", "error": {"code": "internal_error", "debug": "Traceback: secret", "message": "boom"}}, "debug": "top-level traceback"},
    {"authored_source": "gui", "seed_count": 3, "Header": "x", "AUTHORIZATION": "y", "Config_JSON": 1, "silent-payment": 2, "api-key": 3, "SCAN_KEY": 4, "Private": 5},
    {"note": "token=abc123 and password: hunter2", "list": [["xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz"]]},
    {"amount": 1.5, "n": None, "b": True, "i": 12345678901234567890},
    ["/a/b", ["deep", {"k": "/x/y"}]],
    "\ud800 /tmp/x/y \udfff",
    "Bearer abc.def and sk-1234567890abc",
]


def summary_cases():
    source = inspect.getsource(tools.summarize_tool_call)
    keys = sorted(set(re.findall(r'arguments\.get\(\s*"(\w+)"', source)) | {"artifact", "report_read_sync", "auto_sync_before_report_reads"})
    cases = []
    variants = [" val ", "", "   ", None, True, False, 3, ["a", "b"], ["a"], {"k": True}]
    for tool in tools.TOOL_CATALOG:
        arguments_list = [{}, {key: " val " for key in keys}]
        if tool.name in source:
            for key in keys:
                for variant in variants:
                    arguments_list.append({key: variant})
        if tool.name in {"ui.review.plan", "ui.review.apply"}:
            arguments_list += [{"operations": [1, 2]}, {"artifact": {"operations": [1, 2, 3]}}, {"artifact": [1]}, {"operations": "x"}]
        if tool.name == "ui.maintenance.configure":
            arguments_list += [
                {"report_read_sync": None, "auto_sync_before_report_reads": True},
                {"auto_sync_before_report_reads": False, "background_enabled": True, "bitcoin_rail_carrying_value": False, "source_classes": {"rates": 0, "onchain": "yes", "lightning": None}, "market_rate_provider": " coingecko "},
            ]
        if tool.name == "ui.journals.quarantine.resolve":
            arguments_list += [{"transaction": " tx-1 ", "action": "exclude"}, {"action": "exclude"}]
        if tool.name == "ui.reports.export":
            arguments_list += [{"report": "capital_gains_tax", "format": "xlsx"}, {"report": "x", "format": "straße"}]
        if tool.name == "ui.transfers.payouts.create":
            arguments_list += [{"tx_out": "a", "payout_asset": "LBTC", "payout_amount": "0.1"}, {"tx_out": "a", "payout_asset": "", "payout_amount": "0.1"}]
        if tool.name == "ui.loans.mark":
            arguments_list += [{"txid": " t ", "as": "collateral"}, {"txid": "t", "as": 5}]
        if tool.name == "ui.source_funds.links.create":
            arguments_list += [{"to_transaction": "t", "link_type": " spend "}, {"to_transaction": "t"}]
        if tool.name == "ui.source_funds.links.review":
            arguments_list += [{"link": "l", "state": "reviewed"}, {"link": "l"}]
        seen: dict[str, int] = {}
        for arguments in arguments_list:
            summary = tools.summarize_tool_call(tool, arguments)
            # Three witnesses per distinct summary are enough.
            seen[summary] = seen.get(summary, 0) + 1
            if seen[summary] <= 3:
                cases.append({"tool": tool.name, "arguments": arguments, "summary": summary})
    return cases


def parse_cases():
    raws = [
        {}, {"id": "c1"}, {"id": "", "function": {}}, {"id": 5, "function": {"name": "status"}},
        {"id": "c2", "function": {"name": "status", "arguments": None}},
        {"id": "c3", "function": {"name": "status", "arguments": ""}},
        {"id": "c4", "function": {"name": 4, "arguments": {"a": 1}}},
        {"id": "c5", "function": {"name": "x", "arguments": "{\"a\": 1, \"a\": 2}"}},
        {"id": "c6", "function": {"name": "x", "arguments": "[1]"}},
        {"id": "c7", "function": {"name": "x", "arguments": "{bad"}},
        {"id": "c8", "function": {"name": "x", "arguments": 5}},
        {"id": "c9", "function": {"name": "x", "arguments": []}},
        {"id": "c10", "function": {"name": "x", "arguments": "{\"n\": NaN}"}},
        {"id": "c11", "function": "status"},
        {"id": "c12", "function": {"name": "x", "arguments": "  {\"a\": 1}  "}},
        {"id": "c13", "function": {"name": "x", "arguments": False}},
        {"id": "c14", "function": {"name": "x", "arguments": 0}},
    ]
    cases = []
    for index, raw in enumerate(raws):
        call = daemon._parse_ai_tool_call(raw, index)
        cases.append({"raw": raw, "index": index, "call": {"call_id": call.call_id, "name": call.name, "arguments": call.arguments, "argument_error": call.argument_error}})
    return cases


# Planner, context and provenance --------------------------------------------


def planner_cases():
    words = (
        literals(daemon._planned_auto_read_tools)
        + literals(daemon._extract_transaction_search_query)
        + literals(daemon._useful_search_fragment)
        + literals(daemon._balance_history_interval)
    )
    texts = [f"tell me about {word} please" for word in words] + [word.upper() for word in words]
    texts += [
        "", "   ", "find transactions for \"Bitrefill\" and more", "search for coffee shop, then tax",
        "search transactions for  multiple   spaces here", "find matching x", "look for ab", "look for :",
        "show me transactions for alice in 2024", "show txs for bob", "the note is groceries-2024",
        "counterparty called acme.co/x tax", "tag tagged", "abcdef0123456789 tx", "what is my tax summary for 2024?",
        "steuer 1999 und 2024", "tax year ٢٠٢٤", "top 3 transactions", "top wallets", "bottom tx",
        "largest fees", "recent transactions", "last tx", "letzte transaktion", "my tor setup", "tor-node",
        "monthly balance history", "hourly trend", "weekly trend", "daily", "custody gap at whirlpool",
        "transfer swap", "all books", "privacy egress", "rule", "saved view", "loan collateral",
        "price eur", "'ab'", "'a'", "\"" + "x" * 90 + "\"", "find " + "y" * 100, "find tx", "find transactions",
        "a\ud800b tax", "TRANSACTION Ü", "İstanbul wallet",
        # More planned reads than the cap of twelve.
        "pending transfer balance tax wallet backend loan privacy rate history quarantine total",
    ]
    cases = []
    for text in texts:
        validated = {"system_prompt_kind": "kassiber", "messages": [{"role": "user", "content": text}]}
        planned = daemon._planned_auto_read_tools(validated)
        cases.append({"system_prompt_kind": "kassiber", "messages": validated["messages"], "planned": [[call.name, call.arguments] for call in planned]})
    for kind, messages in (
        ("raw", [{"role": "user", "content": "tax"}]),
        (None, [{"role": "user", "content": "tax"}]),
        ("kassiber", [{"role": "user", "content": "tax"}, {"role": "assistant", "content": "x"}]),
        ("kassiber", [{"role": "user", "content": 5}]),
        ("kassiber", [{"role": "user", "content": "tax"}, {"role": "user", "content": "loan"}]),
        ("kassiber", []),
    ):
        planned = daemon._planned_auto_read_tools({"system_prompt_kind": kind, "messages": messages})
        cases.append({"system_prompt_kind": kind, "messages": messages, "planned": [[call.name, call.arguments] for call in planned]})
    return cases


def _sized_contexts():
    """Contexts whose entry or whole message sits exactly at a limit."""
    def entry(tool, size):
        return {"tool": tool, "arguments": {}, "result": {"ok": True, "pad": ["p" * 1000] * (size // 1000) + ["q" * (size % 1000)]}}

    def entry_length(item):
        trimmed = daemon._trim_auto_context_value(tools.redact_ai_tool_result(item))
        return len(json.dumps(trimmed, sort_keys=True, separators=(",", ":")))

    contexts = []
    for target in (daemon.AUTO_CONTEXT_ENTRY_MAX_CHARS, daemon.AUTO_CONTEXT_ENTRY_MAX_CHARS + 1):
        size = target - entry_length(entry("t", 0))
        while entry_length(entry("t", size)) < target:
            size += 1
        while entry_length(entry("t", size)) > target:
            size -= 1
        assert entry_length(entry("t", size)) == target
        contexts.append([entry("t", size)])

    def message_length(context):
        payload = {"untrusted_accounting_data": True, "auto_read_tools": [daemon._auto_context_entry_for_model(e) for e in context]}
        return len(json.dumps(payload, sort_keys=True, separators=(",", ":")))

    # Four entries just under the entry limit, then a small last one grown
    # to the exact message size.
    base = [entry(f"t{index}", 5700) for index in range(4)]
    for target in (daemon.AUTO_CONTEXT_MAX_CHARS, daemon.AUTO_CONTEXT_MAX_CHARS + 1):
        size = 0
        while message_length([*base, entry("last", size)]) < target and size < 5000:
            size += 1
        assert message_length([*base, entry("last", size)]) == target, (target, size)
        contexts.append([*base, entry("last", size)])
    return contexts


def context_cases():
    def entry(tool, result, arguments=None):
        return {"tool": tool, "arguments": arguments or {}, "result": result}

    big = {"ok": True, "envelope": {"kind": "ui.reports.summary", "schema_version": 1, "data": {"summary": {"rows": list(range(40))}, "metrics": "m" * 2100, "filters": {"a": {"b": {"c": {"d": {"e": {"f": {"g": {"h": {"i": 1}}}}}}}}}, "counts": [1], "ready": True, "blockers": [], "other": "z" * 7000}}}
    contexts = [
        [],
        [entry("status", {"ok": True, "envelope": {"kind": "status", "data": {"x": 1}}})],
        [entry("ui.reports.summary", big)],
        [entry("ui.reports.summary", {"ok": False, "reason": "tool_error", "message": "see /Users/a/b"})],
        [entry("ui.reports.summary", {"ok": False, "reason": ""})],
        [entry("x", {"envelope": "notadict"})],
        [entry("x", {"ok": True, "envelope": {"kind": "k", "data": ["list"]}, "pad": "p" * 7000})],
        [entry("ui.wallets.list", {"ok": True, "envelope": {"kind": "ui.wallets.list", "data": {"items": list(range(30)), "token": "secret"}}}, {"password": "x", "limit": 3})],
        [entry(f"t{i}", {"ok": True, "envelope": {"kind": "k", "data": {"summary": "s" * 1900, "x": "y" * 3000}}}) for i in range(12)],
        [entry("deep", {"ok": True, "envelope": {"kind": "k", "data": deep_value(12)}})],
        [entry("unicode", {"ok": True, "envelope": {"kind": "k", "data": {"text": "ü" * 2500}}})],
        [entry("edge", {"ok": True, "envelope": {"kind": "k", "data": {"at": "a" * 2000, "over": "b" * 2001}}})],
        [entry("list", {"ok": True, "envelope": {"kind": "k", "data": {"at": list(range(25)), "over": list(range(26))}}})],
        *_sized_contexts(),
    ]
    return [{"context": context, "message": daemon._auto_tool_context_for_model(copy.deepcopy(context))} for context in contexts]


def envelope(kind, data):
    return {"kind": kind, "schema_version": 1, "data": data}


def provenance_cases():
    health = {"counts": {"active_transactions": 7, "quarantines": 2}, "journals": {"last_processed_at": "2026-01-01T00:00:00Z", "quarantine_count": 3}}
    steps_list = [
        [("status", {"ok": True, "envelope": envelope("status", {})})],
        [("ui.workspace.health", {"ok": True, "envelope": envelope("ui.workspace.health", health)})],
        [("ui.report.blockers", {"ok": True, "envelope": envelope("ui.report.blockers", {"health": health, "rates_coverage": {"summary": {"missing_price_transactions": 4, "active_transactions": 9}}})})],
        [("ui.rates.coverage", {"ok": True, "envelope": envelope("ui.rates.coverage", {"summary": {"missing_price_transactions": True, "active_transactions": 5}})})],
        [("ui.reports.summary", {"ok": True, "envelope": envelope("ui.reports.summary", {}), "auto_journal_process": envelope("ui.journals.process", {"processed_at": "t", "processed_transactions": 3, "quarantined": 1})})],
        [("ui.maintenance.run", {"ok": True, "envelope": envelope("ui.maintenance.run", {"health": health, "maintenance": {"journals": envelope("ui.journals.process", {"processed_at": "p"}), "sync": envelope("ui.wallets.sync", {"ok": False, "results": [1, 2]})}})})],
        [("ui.wallets.sync", {"ok": True, "envelope": envelope("ui.wallets.sync", {"results": []}), "auto_sync": envelope("ui.wallets.sync", {"ok": True})})],
        [("a", {"ok": False, "reason": "tool_not_advertised"}), ("b", {"ok": False}), ("a", {"ok": 1}), ("c", {"ok": True}), ("c", {"ok": True}), ("ui.profiles.snapshot", {"ok": True})],
        [("ui.journals.process", {"ok": True, "envelope": envelope("ui.journals.process", {"processed_transactions": True, "quarantined": 2})})],
        [("x", {"ok": True, "envelope": {"kind": 5, "data": {}}}), ("y", {"ok": True, "envelope": {"kind": "ui.workspace.health", "data": []}})],
    ]
    initial_states = [
        {},
        {"advertised_tools": ["status", "x"], "notices": ["reasoning_effort_unavailable", "bogus", "fast_mode_unavailable"]},
        {"tools_attempted": "notalist", "tool_denials": None, "tools_used": {}, "active_transactions": 99, "notices": "x"},
        {"auto_sync_attempted": 1, "auto_sync_ok": "maybe"},
    ]
    egresses = [
        {"records": [], "gap": False},
        {"records": [{"subsystem": "ai", "host": "h", "port": 443, "bytes_out": 10}, {"subsystem": "ai", "host": "h", "port": 443, "bytes_out": None}, {"subsystem": None, "host": "", "port": 1, "bytes_out": 5}, {"subsystem": "sync", "host": "g", "port": 80}, "junk"], "gap": True},
    ]
    cases = []
    for steps in steps_list:
        for initial in initial_states:
            for egress in egresses:
                state = copy.deepcopy(initial)
                runtime = SimpleNamespace(maintenance_state=state)
                for tool, result in steps:
                    daemon._record_ai_tool_usage(runtime, tool, copy.deepcopy(result))
                ledger = SimpleNamespace(snapshot=lambda after_id, limit, egress=egress: copy.deepcopy(egress))
                provider = {"name": "p", "kind": "local" if steps is steps_list[0] else "remote"}
                validated = {"model": "m", "screen_context": {"route": "/x"} if steps is steps_list[1] else None, "persist": "auto"}
                with mock.patch.object(daemon, "get_egress_ledger", lambda: ledger), mock.patch.object(daemon, "now_iso", lambda: "2026-11-05T00:00:00Z"):
                    provenance = daemon._ai_answer_provenance(provider, validated, runtime)
                cases.append({"initial": initial, "steps": [list(step) for step in steps], "state": state, "provider": provider, "validated": validated, "egress": egress, "provenance": provenance})
    return cases


def denied_cases():
    return [
        {"reason": reason, "message": message, "result": daemon._tool_result_denied(reason, message=message)}
        for reason, message in (("x", None), ("x", ""), ("tool_error", "failed at /Users/a/b.txt"), ("y", "token=abc https://h.example/q"))
    ]


def projection_cases():
    from kassiber.core import chain_analysis_ai

    values = [
        {"nodes": [{"id": "txid:" + "a" * 64, "kind": "transaction", "amount_msat": "1000", "label": "Alice"}], "snapshot_id": "s" * 64},
        {"id": "12345678-1234-1234-1234-1234567890AB", "case_id": "c", "subject": "bc1qxyz", "subjects": ["a", 1, None, {"x": 1}]},
        {"edges": [{"kind": "spends", "source": "out:1", "target": "in:2"}, {"kind": "label", "source": "local_label", "target": "x"}, {"kind": "spends", "source": "s"}, {"source": "free text"}]},
        {"kind": "bc1qcode", "code": "fine_code-x", "rule": "a" * 64, "status": "x" * 101, "reason": "Upper", "level": "", "rules": ["ok_code", "tpub123", "BAD"], "model": 5},
        {"asset": "BTC", "target_asset": "lbtc", "source_asset": "TOOLONGASSET", "fee_msat": "-0", "min_amount_msat": "-12", "amount_msat": "01", "interpretation_count": "12", "7": "33", "\u0663": "x", "8": 4},
        {"final_fee_rate_sat_vb": "1.50", "created_at": "2026-01-01T00:00:00Z", "start": "2026-01-01", "end": "x" * 9, "valid_until": "2026-01-01T00:00:00+00:00\n"},
        {"unknown_field": "x", "summary": {"count": 3, "free": "y"}, "items": [[1, "a"], {"id": "z"}], "error": {"code": "stale_x", "details": {"subject": "s"}}},
        {"ai_reference_scope": "x", "observer": "owner"},
        ["not", "a", "dict"],
        {"decimal_like": 1.5, "count": 2.5, "deleted": True},
    ]

    def issue(scope, value):
        return f"ref({value})"

    cases = []
    with mock.patch.object(chain_analysis_ai, "_scope", lambda conn, profile: b"scope"), mock.patch.object(chain_analysis_ai, "_reference", issue):
        for value in values:
            cases.append({"value": value, "projected": chain_analysis_ai.project_ai_result(None, "p", copy.deepcopy(value))})
    return cases


def document():
    return {
        "capabilities": {tool.name: sorted(tools.tool_capabilities(tool)) for tool in tools.TOOL_CATALOG},
        "selection": selection_cases(),
        "validation": validation_cases(),
        "synthetic_validation": synthetic_cases(),
        "redaction": [
            {"value": value, "arguments": tools.redact_tool_arguments(value), "result": tools.redact_ai_tool_result(value)}
            for value in REDACTION_VALUES
        ],
        "summaries": summary_cases(),
        "parse": parse_cases(),
        "denied": denied_cases(),
        "planner": planner_cases(),
        "context": context_cases(),
        "provenance": provenance_cases(),
        "projection": projection_cases(),
    }


def render() -> str:
    return json.dumps(document(), ensure_ascii=True, separators=(",", ":")) + "\n"


def main(argv: list[str]) -> int:
    text = render()
    if "--check" in argv:
        if TARGET.read_text(encoding="utf-8") != text:
            print(f"{TARGET} is stale; rerun {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

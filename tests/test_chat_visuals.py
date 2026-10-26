"""Assistant visuals: argument validation, projections, router and daemon wiring."""

from __future__ import annotations

import contextlib
import io
import json
import queue
import tempfile
import threading
import unittest
from pathlib import Path

from kassiber.ai.prompt import build_responses_tools
from kassiber.ai.tools import CORE_TOOL_NAMES, get_tool, redact_ai_tool_result
from kassiber.cli.main import main
from kassiber.cli.termrender import render_envelope_table
from kassiber.core import chat_visuals
from kassiber.daemon import (
    AiToolRuntime,
    ParsedAiToolCall,
    _execute_read_only_ai_tool,
    _planned_auto_read_tools,
    _tool_result_for_model,
    current_context_snapshot,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.mcp import tools as mcp_tools


def _spec(**args):
    return chat_visuals.normalize_visual_args(args)


def _build(spec, payload, **kwargs):
    kwargs.setdefault("fiat_currency", "EUR")
    return chat_visuals.build_chat_visual(spec, payload, **kwargs)


class NormalizeVisualArgsTest(unittest.TestCase):
    def test_defaults_per_source(self):
        spec = _spec(source="balance_history")
        self.assertEqual(spec["measure"], "quantity")
        self.assertEqual(spec["report_args"], {"interval": "month", "limit": 500})
        self.assertEqual(spec["limit"], 60)
        self.assertEqual(_spec(source="portfolio")["group_by"], "wallet")
        self.assertEqual(_spec(source="tax_summary")["measure"], "gain_loss")
        self.assertEqual(_spec(source="cash_flow")["report_args"], {"period": "1year"})
        transactions = _spec(source="transactions")
        self.assertEqual((transactions["sort"], transactions["limit"]), ("recent", 10))
        self.assertEqual(_spec(source="overview")["measure"], "market_value")
        self.assertEqual(_spec(source="readiness")["measure"], "status")

    def test_rejects_unknown_values(self):
        for args in (
            {"source": "html"},
            {"source": "tax_summary", "measure": "quantity"},
            {"source": "balance_history", "interval": "year"},
            {"source": "balance_history", "limit": 0},
            {"source": "balance_history", "limit": True},
            {"source": "portfolio", "group_by": "account"},
            {"source": "cash_flow", "period": "2weeks"},
            {"source": "transactions", "limit": 26},
            {"source": "transactions", "sort": "random"},
            {"source": "transactions", "direction": "sideways"},
        ):
            with self.subTest(args=args), self.assertRaises(AppError) as ctx:
                chat_visuals.normalize_visual_args(args)
            self.assertEqual(ctx.exception.code, "validation")

    def test_filter_from_another_source_is_refused(self):
        # A balance filter on the tax chart would silently do nothing.
        with self.assertRaises(AppError) as ctx:
            _spec(source="tax_summary", wallet="Cold")
        self.assertEqual(ctx.exception.details["unknown"], ["wallet"])

    def test_transactions_filters_pass_through(self):
        spec = _spec(
            source="transactions", query="coffee", direction="outbound", period="all",
            status="review", wallet="Hot", sort="largest", limit=5,
        )
        self.assertEqual(
            spec["report_args"],
            {"query": "coffee", "wallet": "Hot", "direction": "outbound", "status": "review"},
        )

    def test_transaction_flow_requires_transaction(self):
        with self.assertRaises(AppError):
            _spec(source="transaction_flow")
        self.assertEqual(
            _spec(source="transaction_flow", transaction=" abc ")["report_args"],
            {"transaction": "abc"},
        )

    def test_title_drops_control_characters_and_is_bounded(self):
        spec = _spec(source="tax_summary", title="Gains‮ by\nyear ")
        self.assertEqual(spec["title"], "Gains by year")
        with self.assertRaises(AppError):
            _spec(source="tax_summary", title="x" * 81)


class ProjectionTest(unittest.TestCase):
    def test_balance_history_marks_missing_prices_and_carries_every_measure(self):
        payload = {
            "rows": [
                {"period_start": "2026-01-01T00:00:00Z", "asset": "BTC", "quantity": 0.1, "market_value": 0.0, "cumulative_cost_basis": 5000.0},
                {"period_start": "2026-02-01T00:00:00Z", "asset": "BTC", "quantity": 0.1, "market_value": 6000.0, "cumulative_cost_basis": 5000.0},
            ],
            "filters": {"interval": "month", "end": "2026-02-15T00:00:00Z"},
            "summary": {"truncated": False},
        }
        result = _build(
            _spec(source="balance_history", measure="market_value"),
            payload,
            fiat_currency="eur",
            readiness=chat_visuals.readiness_from_blockers({
                "blockers": [{"id": "journals_stale"}, {"id": "custody_quantity_unresolved"}],
                "health": {"journals": {"quarantine_count": 2}},
                "rates_coverage": {"summary": {"missing_price_transactions": 3}},
            }),
        )
        self.assertEqual(result["visual"]["chart"], "line")
        self.assertEqual(result["visual"]["unit"], {"kind": "fiat", "code": "EUR"})
        self.assertEqual(result["visual"]["measures"], ["quantity", "market_value", "cost_basis"])
        self.assertEqual([row["market_value"] for row in result["rows"]], [None, 6000.0])
        self.assertEqual([row["quantity"] for row in result["rows"]], [0.1, 0.1])
        self.assertEqual(
            [caveat["code"] for caveat in result["caveats"]],
            ["journals_stale", "quarantines", "custody_unresolved", "unpriced_transactions", "missing_prices"],
        )

    def test_balance_history_falls_to_zero_after_a_full_exit(self):
        # Everything sold in March: the report has no row for any bucket after.
        payload = {
            "rows": [
                {"period_start": "2026-01-01T00:00:00Z", "asset": "BTC", "quantity": 1.0, "market_value": 9.0},
                {"period_start": "2026-02-01T00:00:00Z", "asset": "BTC", "quantity": 1.0, "market_value": 9.0},
            ],
            "filters": {"interval": "month", "end": "2026-04-15T00:00:00Z"},
            "summary": {"truncated": False},
        }
        result = _build(_spec(source="balance_history"), payload)
        self.assertEqual(
            [(row["period"][:7], row["quantity"]) for row in result["rows"]],
            [("2026-01", 1.0), ("2026-02", 1.0), ("2026-03", 0.0), ("2026-04", 0.0)],
        )
        self.assertEqual(result["digest"]["assets"]["BTC"]["quantity"]["last"], 0.0)
        # A gap in the middle is a zero balance too, not a flat line.
        gap = {**payload, "rows": [payload["rows"][0], {**payload["rows"][1], "period_start": "2026-03-01T00:00:00Z"}]}
        middle = _build(_spec(source="balance_history"), gap)
        self.assertEqual([row["quantity"] for row in middle["rows"]][:3], [1.0, 0.0, 1.0])

    def test_balance_history_plots_omitted_zero_balances_and_whole_buckets(self):
        payload = {
            "rows": [
                # The report cut this oldest bucket after its BTC row.
                {"period_start": "2026-01-01T00:00:00Z", "asset": "LBTC", "quantity": 0.5},
                {"period_start": "2026-02-01T00:00:00Z", "asset": "BTC", "quantity": 1.0, "market_value": 9.0},
                {"period_start": "2026-02-01T00:00:00Z", "asset": "LBTC", "quantity": 0.5, "market_value": 4.0},
                # LBTC was spent: the report omits its zero row.
                {"period_start": "2026-03-01T00:00:00Z", "asset": "BTC", "quantity": 1.0, "market_value": 9.0},
            ],
            "filters": {"end": "2026-03-31T00:00:00Z"},
            "summary": {"truncated": True},
        }
        result = _build(_spec(source="balance_history"), payload)
        self.assertEqual(
            [(row["period"][:7], row["asset"], row["quantity"]) for row in result["rows"]],
            [("2026-02", "BTC", 1.0), ("2026-02", "LBTC", 0.5), ("2026-03", "BTC", 1.0), ("2026-03", "LBTC", 0.0)],
        )
        self.assertEqual(result["rows"][-1]["market_value"], 0.0)
        self.assertEqual(result["caveats"], [{"code": "truncated"}])
        limited = _build(
            _spec(source="balance_history", limit=1),
            {**payload, "summary": {"truncated": False}},
        )
        self.assertEqual({row["period"] for row in limited["rows"]}, {"2026-03-01T00:00:00Z"})
        self.assertEqual(limited["caveats"], [{"code": "truncated"}])

    def test_portfolio_keeps_wallet_and_asset_and_unknown_values(self):
        payload = {
            "rows": [
                {"wallet": "Hot", "asset": "BTC", "quantity": 0.1, "market_value": 5000.0, "cost_basis": 1.0, "unrealized_pnl": 4999.0},
                {"wallet": "Cold", "asset": "BTC", "quantity": 1.0, "market_value": 50000.0, "cost_basis": 1.0, "unrealized_pnl": 49999.0},
                {"wallet": "Hot", "asset": "LBTC", "quantity": 0.2, "market_value": 0.0, "cost_basis": 1.0, "unrealized_pnl": -1.0},
            ]
        }
        result = _build(_spec(source="portfolio"), payload)
        self.assertEqual(
            [(row["wallet"], row["asset"], row["market_value"], row["unrealized_pnl"]) for row in result["rows"]],
            [("Cold", "BTC", 50000.0, 49999.0), ("Hot", "BTC", 5000.0, 4999.0), ("Hot", "LBTC", None, None)],
        )
        self.assertEqual(result["caveats"], [{"code": "missing_prices", "count": 1}])
        self.assertEqual(result["visual"]["group_by"], "wallet")

    def test_tax_summary_keeps_year_totals_and_details(self):
        payload = {
            "rows": [
                {"row_type": "detail", "year": 2025, "asset": "BTC", "transaction_type": "sell", "gain_loss": 10.0, "proceeds": 20.0, "cost_basis": 10.0},
                {"row_type": "year_total", "year": 2025, "gain_loss": 10.0, "proceeds": 20.0, "cost_basis": 10.0},
                {"row_type": "year_total", "year": 2026, "gain_loss": -4.5, "proceeds": 1.0, "cost_basis": 5.5},
                {"row_type": "grand_total", "year": None, "gain_loss": 5.5},
            ]
        }
        result = _build(_spec(source="tax_summary"), payload)
        self.assertEqual([(row["year"], row["gain_loss"]) for row in result["rows"]], [(2025, 10.0), (2026, -4.5)])
        self.assertEqual(result["details"][0]["transaction_type"], "sell")
        self.assertEqual(result["digest"], {"years": result["rows"]})

    def test_overview_hides_values_without_a_market_rate(self):
        payload = {
            "fiat": {
                "eurBalance": 100.0, "eurCostBasis": 80.0, "eurUnrealized": 20.0, "eurRealizedYTD": 3.0,
                "completeness": {"marketRateMissing": True, "costBasisComplete": False, "reasons": ["journals_stale"], "quarantineCount": 2},
            },
            "balanceSummary": {"totalBtc": 0.5},
            "marketRate": {"rate": 1.0},
            "portfolioSeries": [{"date": f"2026-01-{day:02d}", "balanceBtc": 0.5, "valueEur": 1.0, "costBasisEur": 0.5} for day in range(1, 29)],
            "connections": [{"label": f"W{index}", "balance": float(index)} for index in range(8)],
            "status": {"transactionCount": 9},
        }
        result = _build(_spec(source="overview"), payload)
        stats = result["stats"]
        self.assertIsNone(stats["portfolio_value"])
        self.assertIsNone(stats["unrealized"])
        self.assertEqual(stats["cost_basis"], 80.0)
        self.assertFalse(stats["basis_complete"])
        self.assertEqual(
            [caveat["code"] for caveat in result["caveats"]],
            ["journals_stale", "quarantines", "market_rate_missing"],
        )
        self.assertEqual(len(result["wallets"]), chat_visuals.MAX_OVERVIEW_WALLETS + 1)
        self.assertEqual(result["wallets"][0]["label"], "W7")
        self.assertEqual(result["wallets"][-1]["other_count"], 2)
        self.assertAlmostEqual(sum(wallet["share"] for wallet in result["wallets"]), 1.0)

    def test_cash_flow_reads_the_dashboard_buckets(self):
        def stats(count, btc, eur, missing=0):
            return {"count": count, "btc": btc, "eur": eur, "missingPrice": missing}

        payload = {
            "window": {"since": "2026-01-01T00:00:00Z", "until": "2026-03-31T00:00:00Z"},
            "buckets": [
                {"bucketKey": "2026-01", "bucketStart": "2026-01-01T00:00:00+00:00",
                 "stats": {"incoming": stats(2, 0.3, 30.0), "outgoing": stats(1, 0.1, 0.0, 1), "transfers": stats(1, 0.2, 0.0), "swaps": stats(0, 0, 0)}},
            ],
            "summary": {"incoming": stats(2, 0.3, 30.0), "outgoing": stats(1, 0.1, 0.0, 1), "transfers": stats(1, 0.2, 0.0), "swaps": stats(0, 0, 0), "review": 1, "pending": 0},
        }
        result = _build(_spec(source="cash_flow", period="3months"), payload)
        row = result["rows"][0]
        self.assertEqual((row["in_btc"], row["out_btc"], row["transfer_count"]), (0.3, 0.1, 1))
        self.assertEqual(result["visual"]["bucket"], "week")
        self.assertEqual(result["totals"]["incoming"]["count"], 2)
        self.assertEqual(result["caveats"], [{"code": "missing_prices", "count": 1}])

    def test_transactions_rows_and_summary(self):
        payload = {
            "txs": [
                {"id": "a", "asset": "BTC", "occurredAt": "2026-01-02T00:00:00Z", "amountSat": 5000, "feeSat": 0, "eur": 3.0, "account": "Cold", "counter": "Shop", "note": "x" * 200, "queueStatus": "completed", "tags": ["a", "b", "c", "d", "e"]},
                {"id": "b", "asset": "BTC", "occurredAt": "2026-01-01T00:00:00Z", "amountSat": -2000, "feeSat": 100, "eur": -1.0, "account": "Hot"},
            ],
            "total": 7,
        }
        result = _build(_spec(source="transactions"), payload)
        self.assertEqual([row["direction"] for row in result["rows"]], ["in", "out"])
        self.assertEqual(len(result["rows"][0]["note"]), chat_visuals.MAX_NOTE_CHARS)
        self.assertEqual(len(result["rows"][0]["tags"]), 4)
        self.assertEqual(result["summary"], {"shown": 2, "total": 7, "in_sat": 5000, "out_sat": 2000})
        self.assertEqual(result["caveats"], [{"code": "more_transactions", "count": 5}])
        # A sub-sat Lightning payment keeps its sign; other assets are not summed as BTC.
        lightning = _build(_spec(source="transactions"), {"txs": [{"id": "ln", "amountSat": -0.5, "asset": "LNBTC"}]})
        self.assertEqual(lightning["rows"][0]["direction"], "out")
        mixed = _build(_spec(source="transactions"), {"txs": [{"id": "a", "amountSat": 5, "asset": "BTC"}, {"id": "b", "amountSat": 5, "asset": "USDT"}]})
        self.assertIsNone(mixed["summary"]["in_sat"])

    def test_transaction_flow_shows_only_own_wallet_labels(self):
        payload = {
            "transaction": {"id": "tx-1"},
            "supportLevel": "full",
            "inputs": [
                # The graph labels a node with its outpoint; the wallet is its own field.
                {"label": "cafebabe:1", "wallet": "Cold", "ownership": "owned", "valueSats": 10_000, "valueState": "known", "address": "bc1qsecret"},
                {"label": "f00d:0", "wallet": "Cold, Hot", "ownership": "ambiguous", "valueSats": 1_000, "valueState": "known"},
            ],
            "outputs": [
                {"label": "deadbeef:0", "ownership": "unknown", "valueSats": 9_000, "valueState": "known", "address": "bc1qother"},
                {"id": "output-overflow", "overflow": True, "overflowCount": 3, "label": "+3 more", "ownership": "overflow"},
            ],
            "fee": {"label": "Miner fee", "ownership": "network_fee", "valueSats": 1_000, "valueState": "known"},
            "accounting": {"quarantine": None},
        }
        result = _build(
            _spec(source="transaction_flow", transaction="tx-1"),
            payload,
            local_reference={"route": "/transactions", "transaction": "tx-1"},
            context={"transaction": {"id": "tx-1", "amountSat": -9000, "feeSat": 1000, "account": "Cold"}},
        )
        labels = [(row["side"], row["label"], row["label_kind"]) for row in result["rows"]]
        self.assertEqual(
            labels,
            [("input", "Cold", "wallet"), ("input", "Cold, Hot", "wallet"), ("output", "Output 1", "generic"), ("output", "+3 more", "overflow"), ("fee", "Miner fee", "fee")],
        )
        self.assertNotIn("bc1q", json.dumps(result))
        self.assertNotIn("deadbeef", json.dumps(result))
        self.assertNotIn("cafebabe", json.dumps(result))
        self.assertNotIn("f00d", json.dumps(result))
        self.assertEqual(result["caveats"], [{"code": "amounts_hidden", "count": 1}])
        self.assertEqual(result["transaction"]["direction"], "out")
        self.assertEqual(result["local_reference"]["transaction"], "tx-1")

    def test_readiness_turns_blockers_into_checks(self):
        payload = {
            "ready": False,
            "blockers": [
                {"id": "journal_quarantine", "severity": "blocking", "title": "Quarantined journal rows", "detail": "2 need review."},
                {"id": "sync_failed", "severity": "blocking", "title": "Wallet sync failed", "detail": "Cold could not sync."},
                {"id": "source_overlap", "severity": "review", "title": "Overlapping sources", "detail": None},
            ],
            "custody_quantity": {"issue_count": 0},
            "health": {
                "counts": {"active_transactions": 4, "quarantines": 2},
                "journals": {"needs_processing": True, "quarantine_count": 2},
                "review_queue": {"review": 0},
            },
            "rates_coverage": {"summary": {"missing_price_transactions": 3, "cache_coverable_missing": 1}},
        }
        result = _build(_spec(source="readiness"), payload)
        checks = {row["check"]: row for row in result["rows"]}
        self.assertEqual(checks["journals"]["action"], "process_journals")
        self.assertEqual((checks["quarantine"]["status"], checks["quarantine"]["count"]), ("blocked", 2))
        self.assertEqual(checks["prices"]["detail"], "1")
        blockers = [row for row in result["rows"] if row["check"] == "blocker"]
        # Blockers with their own check are not listed twice.
        self.assertEqual([(b["title"], b["status"]) for b in blockers], [("Wallet sync failed", "blocked"), ("Overlapping sources", "warning")])
        self.assertEqual(blockers[0]["detail"], "Cold could not sync.")
        self.assertEqual(result["summary"], {"ready": False, "passed": 3, "total": 8})

    def test_model_digest_drops_long_rows_but_keeps_caveats(self):
        result = _build(
            _spec(source="balance_history"),
            {"rows": [{"period_start": f"2026-{m:02d}-01T00:00:00Z", "asset": "BTC", "quantity": float(m)} for m in range(1, 13)], "summary": {}},
        )
        self.assertEqual(result["digest"]["assets"]["BTC"]["quantity"]["change"], 11.0)
        result["rows"] = result["rows"] * 4  # 48 rows
        result["caveats"] = [{"code": "truncated"}]
        digest = chat_visuals.model_digest(result)
        self.assertNotIn("rows", digest)
        self.assertEqual(digest["rows_omitted"], 48)
        self.assertEqual(digest["caveats"], [{"code": "truncated"}])
        # The auto-read fallback keeps only `summary`; the caveats survive there.
        self.assertEqual(digest["summary"]["caveats"], [{"code": "truncated"}])
        self.assertTrue(digest["shown_to_user"])
        small = chat_visuals.model_digest(_build(_spec(source="tax_summary"), {"rows": []}))
        self.assertEqual(small["rows"], [])
        envelope = {"ok": True, "envelope": {"kind": "ui.visuals.render", "data": result}}
        self.assertNotIn("rows", _tool_result_for_model(envelope)["envelope"]["data"])
        other = {"ok": True, "envelope": {"kind": "status", "data": {"rows": [1]}}}
        self.assertIs(_tool_result_for_model(other), other)

    def test_rows_render_as_cli_table(self):
        result = _build(
            _spec(source="tax_summary"),
            {"rows": [{"row_type": "year_total", "year": 2026, "gain_loss": 12.0}]},
        )
        result["caveats"] = [{"code": "journals_stale"}, {"code": "quarantines", "count": 2}]
        table = render_envelope_table({"kind": "ui.visuals.render", "data": result}, terminal_width=80)
        self.assertIn("2026", table)
        self.assertNotIn("journals_stale", table)


class CatalogTest(unittest.TestCase):
    def test_tool_is_read_only_local_and_offered_for_charts_and_reports(self):
        entry = get_tool("ui_visuals_render")
        self.assertEqual(entry.name, "ui.visuals.render")
        self.assertEqual(entry.kind_class, "read_only")
        self.assertFalse(entry.egresses)
        self.assertIn(entry.name, CORE_TOOL_NAMES)

        def offered(question, profile):
            return {
                tool["name"]
                for tool in build_responses_tools([{"role": "user", "content": question}], profile=profile)
            }

        for profile in ("core", "scoped"):
            for question in ("Plot my coins", "Zeig ein Diagramm", "What is my balance?", "show my transactions"):
                with self.subTest(profile=profile, question=question):
                    self.assertIn("ui_visuals_render", offered(question, profile))
            # A quarantine review keeps its own bounded pack.
            self.assertNotIn("ui_visuals_render", offered("Investigate quarantine", profile))

    def test_schema_matches_core(self):
        properties = get_tool("ui.visuals.render").parameters["properties"]
        self.assertEqual(tuple(properties["source"]["enum"]), chat_visuals.VISUAL_SOURCES)
        self.assertEqual(
            set(properties["measure"]["enum"]),
            {m for measures in chat_visuals.VISUAL_MEASURES.values() for m in measures},
        )
        self.assertEqual(tuple(properties["interval"]["enum"]), chat_visuals.BALANCE_HISTORY_INTERVALS)
        self.assertEqual(tuple(properties["period"]["enum"]), chat_visuals.CASH_FLOW_PERIODS)
        self.assertEqual(tuple(properties["sort"]["enum"]), chat_visuals.TRANSACTION_SORTS)
        self.assertEqual(tuple(properties["status"]["enum"]), chat_visuals.TRANSACTION_STATUSES)
        self.assertEqual(tuple(properties["direction"]["enum"]), chat_visuals.TRANSACTION_DIRECTIONS)

    def test_not_exposed_to_external_agents(self):
        exposed = {entry.name for entry in mcp_tools.mcp_entries()}
        self.assertNotIn("ui.visuals.render", exposed)


def _planned_visuals(question):
    planned = _planned_auto_read_tools(
        {"system_prompt_kind": "kassiber", "messages": [{"role": "user", "content": question}]}
    )
    return [item.arguments for item in planned if item.name == "ui.visuals.render"]


class PlannedVisualTest(unittest.TestCase):
    """A visual asked for by name is drawn even when the model calls no tool."""

    def test_requests_plan_the_matching_source(self):
        cases = {
            "show me a chart of my balance, weekly": [{"source": "balance_history", "interval": "week"}],
            "Zeig mir ein Diagramm der Gewinne pro Steuerjahr": [{"source": "tax_summary"}],
            "plot my portfolio by wallet": [{"source": "portfolio"}],
            "can you visualize this?": [{"source": "balance_history", "interval": "month"}],
            "chart my cash flow": [{"source": "cash_flow"}],
            "Give me an overview": [{"source": "overview"}],
            "what needs attention before I file?": [{"source": "readiness"}],
            "show my largest outgoing transactions": [{"source": "transactions", "sort": "largest", "direction": "outbound"}],
            "Zeig meine letzten Transaktionen": [{"source": "transactions"}],
        }
        for question, expected in cases.items():
            with self.subTest(question=question):
                self.assertEqual(_planned_visuals(question), expected)

    def test_no_visual_without_asking_for_one(self):
        for question in (
            "what is my balance history?",
            "show the ownership graph for this cluster",
            "graph the transaction inputs",
            "show the transaction graph",
            "a paragraph about taxes",
        ):
            with self.subTest(question=question):
                self.assertEqual(_planned_visuals(question), [])


def _run_cli(data_root, *argv):
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(io.StringIO()):
        code = main(["--data-root", data_root, "--machine", *argv])
    if code != 0:
        raise AssertionError(stdout.getvalue())


def _execute_on_conn(call, runtime, conn):
    results = []
    thread = threading.Thread(target=lambda: results.append(_execute_read_only_ai_tool(call, runtime)))
    thread.start()
    while thread.is_alive() or not runtime.main_thread_tasks.empty():
        try:
            task = runtime.main_thread_tasks.get(timeout=0.05)
        except queue.Empty:
            continue
        try:
            task.response.put((True, task.callback(conn)))
        except Exception as exc:  # pragma: no cover - surfaced through result
            task.response.put((False, exc))
    thread.join(timeout=5)
    return results[0]


class DaemonVisualToolTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory()
        root = Path(cls._tmp.name)
        cls.data_root = str(root / "data")
        csv_path = root / "transactions.csv"
        csv_path.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-01T10:00:00Z,seed-inbound-1,inbound,BTC,0.10000000,0,50000,Seed\n"
            "2026-02-01T10:00:00Z,seed-outbound-1,outbound,BTC,0.02000000,0.00001,60000,Coffee\n",
            encoding="utf-8",
        )
        _run_cli(cls.data_root, "init")
        _run_cli(cls.data_root, "workspaces", "create", "Demo")
        _run_cli(cls.data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
        _run_cli(
            cls.data_root, "wallets", "create", "--label", "Cold", "--kind", "address",
            "--address", "bc1qtestaddress0000000000000000000000000000000",
        )
        _run_cli(cls.data_root, "wallets", "import-csv", "--wallet", "Cold", "--file", str(csv_path))
        _run_cli(cls.data_root, "rates", "set", "BTC-EUR", "2026-01-01T00:00:00Z", "50000")

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def setUp(self):
        self.conn = open_db(self.data_root)
        scope = current_context_snapshot(self.conn)
        self.runtime = AiToolRuntime(
            data_root=self.data_root,
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={
                "provider_kind": "remote",
                "scope_workspace_id": scope["workspace_id"],
                "scope_profile_id": scope["profile_id"],
                "advertised_tools": None,
            },
        )

    def tearDown(self):
        self.conn.close()

    def _render(self, **arguments):
        result = _execute_on_conn(
            ParsedAiToolCall(call_id="call-visual", name="ui.visuals.render", arguments=arguments),
            self.runtime,
            self.conn,
        )
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["envelope"]["kind"], "ui.visuals.render")
        return result["envelope"]["data"]

    def test_balance_history_chart_comes_from_local_report(self):
        data = self._render(source="balance_history", interval="month", title="Balance")
        self.assertEqual(data["visual"]["title"], "Balance")
        self.assertEqual(data["rows"][0]["asset"], "BTC")
        self.assertAlmostEqual(data["rows"][0]["quantity"], 0.1)
        # Stale journals were refreshed before the read, like the report tools.
        self.assertNotIn("journals_stale", [c["code"] for c in data["caveats"]])

    def test_every_source_renders_from_the_book(self):
        for arguments in (
            {"source": "overview"},
            {"source": "portfolio", "measure": "quantity"},
            {"source": "tax_summary"},
            {"source": "cash_flow", "period": "all"},
            {"source": "cash_flow", "period": "1year"},
            {"source": "transactions", "sort": "largest"},
            {"source": "transactions", "direction": "outbound"},
            {"source": "readiness"},
        ):
            with self.subTest(arguments=arguments):
                data = self._render(**arguments)
                self.assertEqual(data["visual"]["source"], arguments["source"])
                self.assertIn("digest", data)
                json.dumps(redact_ai_tool_result(data))

    def test_sources_agree_with_the_book(self):
        overview = self._render(source="overview")
        self.assertAlmostEqual(overview["stats"]["btc_balance"], 0.07999, places=5)
        flow = self._render(source="cash_flow", period="all")
        self.assertEqual(flow["totals"]["incoming"]["count"], 1)
        self.assertEqual(flow["totals"]["outgoing"]["count"], 1)
        largest = self._render(source="transactions", sort="largest", limit=1)
        self.assertEqual([row["amount_sat"] for row in largest["rows"]], [10_000_000])
        outgoing = self._render(source="transactions", direction="outbound")
        self.assertEqual([row["direction"] for row in outgoing["rows"]], ["out"])
        smallest = self._render(source="transactions", sort="smallest", limit=1)
        self.assertEqual([row["amount_sat"] for row in smallest["rows"]], [-2_000_000])

    def test_only_report_visuals_run_read_maintenance(self):
        from kassiber.daemon import _ai_read_wants_maintenance

        def wants(source):
            return _ai_read_wants_maintenance(
                ParsedAiToolCall(call_id="c", name="ui.visuals.render", arguments={"source": source})
            )

        self.assertTrue(wants("balance_history"))
        self.assertTrue(wants("tax_summary"))
        for source in ("transactions", "transaction_flow", "cash_flow", "readiness"):
            self.assertFalse(wants(source), source)

    def test_transaction_flow_without_graph_still_names_the_transaction(self):
        row = self.conn.execute("SELECT id FROM transactions ORDER BY occurred_at LIMIT 1").fetchone()
        data = self._render(source="transaction_flow", transaction=row["id"])
        self.assertEqual(data["visual"]["chart"], "flow")
        self.assertEqual(data["transaction"]["amount_sat"], 10_000_000)
        self.assertEqual(data["local_reference"], {"route": "/transactions", "transaction": row["id"]})

    def test_invalid_arguments_are_refused(self):
        result = _execute_on_conn(
            ParsedAiToolCall(call_id="bad", name="ui.visuals.render", arguments={"source": "tax_summary", "wallet": "Cold"}),
            self.runtime,
            self.conn,
        )
        self.assertFalse(result["ok"])


if __name__ == "__main__":
    unittest.main()

"""Unit tests for the AI core (SSE parser, error mapping, provider CRUD).

Stdlib-only, no pytest dep. Mirrors `tests/test_cli_smoke.py` style. Smoke
tests for the CLI/daemon surface live in test_cli_smoke.py; these tests
cover the underlying primitives directly so failures point at one layer.
"""

from __future__ import annotations

import io
import json
import os
import queue
import socket
import subprocess
import tempfile
import threading
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.ai import (
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    get_ai_provider_api_key_for_use,
    list_db_ai_providers,
    redact_ai_provider_for_output,
    require_ai_provider_acknowledged,
    resolve_ai_provider,
    set_default_ai_provider,
    set_db_ai_provider_api_key,
    set_db_ai_provider_native_secret_ref,
    clear_default_ai_provider,
    update_db_ai_provider,
    seed_default_ai_provider_if_empty,
)
from kassiber.ai.client import (
    CLI_DEFAULT_MODEL,
    CliAIClient,
    DEFAULT_TIMEOUT_SECONDS,
    OpenAIResponsesClient,
    ResponsesToolCallAccumulator,
    parse_sse_chunks,
    _cli_failure,
    _http_error_app_error,
    _network_error_app_error,
    _resolve_cli_executable,
    _cli_subprocess_env,
)
from kassiber.ai.prompt import (
    DEFAULT_KASSIBER_SYSTEM_PROMPT,
    build_chat_messages,
    build_openai_tools,
)
from kassiber.ai.tools import (
    SKILL_REFERENCE_NAMES,
    get_tool,
    read_skill_reference,
    redact_ai_tool_result,
    redact_tool_arguments,
    summarize_tool_call,
)
from kassiber.ai.providers import ai_provider_secret_service_id, list_with_default
from kassiber.core import accounts as core_accounts
from kassiber.core import chat_history as core_chat_history
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.redaction import redact_secret_text, redact_secret_value


class SseParserTest(unittest.TestCase):
    """The SSE parser is the load-bearing piece for streaming. Pin the
    edge cases the wire format throws at us in practice."""

    def _parse(self, text):
        return list(parse_sse_chunks(text.splitlines(keepends=True)))

    def test_single_event(self):
        chunks = self._parse('data: {"choices":[{"delta":{"content":"hi"}}]}\n\n')
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "hi")

    def test_multiple_events(self):
        text = (
            'data: {"choices":[{"delta":{"content":"a"}}]}\n\n'
            'data: {"choices":[{"delta":{"content":"b"}}]}\n\n'
        )
        chunks = self._parse(text)
        self.assertEqual([c["choices"][0]["delta"]["content"] for c in chunks], ["a", "b"])

    def test_done_terminates_stream(self):
        text = (
            'data: {"choices":[{"delta":{"content":"a"}}]}\n\n'
            "data: [DONE]\n\n"
            'data: {"choices":[{"delta":{"content":"b"}}]}\n\n'
        )
        chunks = self._parse(text)
        # Anything after [DONE] is intentionally dropped.
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "a")

    def test_comments_and_unknown_fields_ignored(self):
        text = (
            ": keepalive\n"
            "event: message\n"
            "id: 1\n"
            'data: {"choices":[{"delta":{"content":"a"}}]}\n'
            "\n"
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)

    def test_multi_line_data_concatenated(self):
        # Per the SSE spec, consecutive `data:` lines join with `\n` before
        # the event boundary. JSON tolerates internal whitespace, so a
        # well-formed pretty-printed object across lines parses cleanly.
        text = (
            'data: {"choices":[\n'
            'data:   {"delta":{"content":"hi"}}\n'
            "data: ]}\n"
            "\n"
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "hi")

    def test_malformed_payload_skipped(self):
        text = (
            "data: not-json\n\n"
            'data: {"choices":[{"delta":{"content":"ok"}}]}\n\n'
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "ok")

    def test_trailing_chunk_without_blank_line(self):
        # Some servers don't emit a final blank line before closing the
        # connection. The parser should still flush the buffered data.
        text = 'data: {"choices":[{"delta":{"content":"end"}}]}'
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "end")


class ToolCatalogPromptTest(unittest.TestCase):
    def test_shared_redaction_treats_recovery_material_as_secret(self):
        text = redact_secret_text(
            "mnemonic=abandon abandon abandon recovery_phrase=legal winner thank seed=letter"
        )
        self.assertNotIn("abandon", text)
        self.assertNotIn("legal", text)
        self.assertNotIn("winner", text)
        self.assertNotIn("letter", text)
        self.assertEqual(
            redact_secret_value(
                {
                    "mnemonic": "abandon abandon abandon",
                    "seed_words": "legal winner thank",
                    "safe": "ok",
                }
            ),
            {"mnemonic": "[redacted]", "seed_words": "[redacted]", "safe": "ok"},
        )

    def test_cli_provider_failure_details_do_not_echo_stdout_or_stderr(self):
        completed = subprocess.CompletedProcess(
            args=["codex"],
            returncode=1,
            stdout="prompt fragment: explain my wallet sk-test-secret",
            stderr="Authorization: Bearer sk-test-secret\nprompt fragment",
        )

        error = _cli_failure("codex", completed)

        self.assertEqual(error.details["exit_code"], 1)
        self.assertIn("stdout_bytes", error.details)
        self.assertIn("stderr_bytes", error.details)
        self.assertNotIn("stdout", error.details)
        self.assertNotIn("stderr", error.details)
        self.assertNotIn("prompt fragment", repr(error.details))
        self.assertNotIn("sk-test-secret", repr(error.details))

    def test_tool_catalog_stability(self):
        expected_tool_names = {
            "status",
            "ui_overview_snapshot",
            "ui_transactions_list",
            "ui_transactions_extremes",
            "ui_transactions_search",
            "ui_transactions_history",
            "ui_activity_history",
            "ui_wallets_list",
            "ui_wallets_utxos",
            "ui_wallets_identify",
            "ui_backends_list",
            "ui_profiles_snapshot",
            "ui_reports_capital_gains",
            "ui_reports_summary",
            "ui_reports_balance_sheet",
            "ui_reports_portfolio_summary",
            "ui_reports_tax_summary",
            "ui_reports_balance_history",
            "ui_reports_lightning_profitability",
            "ui_reports_privacy_hygiene",
            "ui_reports_privacy_mirror",
            "ui_connections_node_snapshot",
            "ui_journals_snapshot",
            "ui_journals_quarantine",
            "ui_journals_quarantine_resolve",
            "ui_journals_events_list",
            "ui_journals_transfers_list",
            "ui_journals_process",
            "ui_rates_summary",
            "ui_rates_coverage",
            "ui_rates_rebuild",
            "ui_report_blockers",
            "ui_audit_changes_since_last_answer",
            "ui_maintenance_settings",
            "ui_workspace_health",
            "ui_next_actions",
            "ui_source_funds_sources_list",
            "ui_source_funds_links_list",
            "ui_source_funds_preview",
            "read_skill_reference",
            "ui_wallets_sync",
            "ui_maintenance_configure",
            "ui_maintenance_run",
            "ui_source_funds_sources_create",
            "ui_source_funds_links_create",
            "ui_source_funds_links_review",
            "ui_source_funds_suggest",
            "ui_source_funds_links_bulk_review",
            "ui_transfers_suggest",
            "ui_custody_coverage_snapshot",
            "ui_custody_lineage_snapshot",
            "ui_custody_gaps_list",
            "ui_custody_gaps_review_context",
            "ui_custody_gaps_history",
            "ui_custody_review_plan",
            "ui_custody_review_apply",
            "ui_transfers_review_context",
            "ui_transfers_list",
            "ui_transfers_payouts_list",
            "ui_transfers_components_list",
            "ui_transfers_rules_list",
            "ui_saved_views_list",
            "ui_transfers_pair",
            "ui_transfers_payouts_create",
            "ui_transfers_payouts_delete",
            "ui_transfers_update",
            "ui_transfers_components_plan",
            "ui_transfers_components_apply",
            "ui_transfers_unpair",
            "ui_transfers_bulk_pair",
            "ui_transfers_dismiss",
            "ui_transfers_rules_create",
            "ui_transfers_rules_delete",
            "ui_transfers_rules_set_enabled",
            "ui_transfers_rules_apply",
            "ui_saved_views_create",
            "ui_saved_views_delete",
            "ui_transactions_resolve",
            "ui_transactions_graph",
            "ui_transactions_review_context",
            "ui_workspace_overview_snapshot",
            "ui_activity_stale",
            "ui_attachments_list",
            "ui_audit_evidence_summary",
            "ui_review_badges",
            "ui_review_worklist",
            "ui_loans_list",
            "ui_loans_mark",
            "ui_loans_link",
            "ui_loans_unmark",
            "ui_transactions_metadata_update",
            "ui_transactions_history_revert",
            "ui_attachments_copy",
            "ui_source_funds_evidence_list",
            "ui_source_funds_sources_attach",
            "ui_source_funds_links_attach",
            "ui_source_funds_coverage",
            "ui_source_funds_cases_list",
            "ui_source_funds_assemble",
            "ui_source_funds_cases_save",
            "ui_source_funds_export",
            "ui_transactions_commercial_context",
            "ui_btcpay_provenance_list",
            "ui_btcpay_provenance_suggest",
            "ui_btcpay_provenance_links",
            "ui_documents_list",
            "ui_btcpay_provenance_review",
            "ui_documents_create",
            "ui_rates_latest",
            "ui_reports_exit_tax_preview",
            "ui_reports_export",
            "ui_egress_snapshot",
        }
        tool_names = {
            tool["name"]
            for tool in build_openai_tools()
            if tool.get("type") == "function"
        }
        self.assertEqual(tool_names, expected_tool_names)
        self.assertTrue(
            all(tool.get("strict") is False for tool in build_openai_tools())
        )
        for tool_name in tool_names:
            self.assertRegex(tool_name, r"^[A-Za-z0-9_-]{1,64}$")
        self.assertEqual(get_tool("ui_overview_snapshot").name, "ui.overview.snapshot")
        self.assertEqual(get_tool("ui_workspace_health").name, "ui.workspace.health")
        self.assertEqual(get_tool("ui_next_actions").kind_class, "read_only")
        self.assertEqual(get_tool("ui_transactions_extremes").name, "ui.transactions.extremes")
        self.assertEqual(get_tool("ui_transactions_search").name, "ui.transactions.search")
        self.assertEqual(get_tool("ui_wallets_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_wallets_utxos").name, "ui.wallets.utxos")
        self.assertEqual(get_tool("ui_wallets_utxos").kind_class, "read_only")
        # The AI identify tool must never accept a file/CSV harvest channel:
        # only addresses/txids/text, with additionalProperties locked off.
        _identify_params = get_tool("ui_wallets_identify").parameters
        self.assertEqual(_identify_params.get("additionalProperties"), False)
        self.assertEqual(
            set(_identify_params.get("properties", {})),
            {"addresses", "txids", "text"},
        )
        self.assertEqual(get_tool("ui_backends_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_reports_summary").name, "ui.reports.summary")
        self.assertEqual(get_tool("ui_reports_summary").kind_class, "read_only")
        self.assertEqual(get_tool("ui_reports_balance_sheet").name, "ui.reports.balance_sheet")
        self.assertEqual(
            get_tool("ui_reports_portfolio_summary").name,
            "ui.reports.portfolio_summary",
        )
        self.assertEqual(get_tool("ui_reports_tax_summary").name, "ui.reports.tax_summary")
        self.assertEqual(
            get_tool("ui_reports_balance_history").name,
            "ui.reports.balance_history",
        )
        self.assertEqual(get_tool("ui_journals_events_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_journals_quarantine").kind_class, "read_only")
        self.assertEqual(
            get_tool("ui_journals_quarantine_resolve").kind_class,
            "mutating",
        )
        self.assertEqual(get_tool("ui_transfers_components_list").kind_class, "read_only")
        self.assertEqual(
            get_tool("ui_transfers_components_plan").kind_class,
            "read_only",
        )
        self.assertEqual(
            get_tool("ui_transfers_components_apply").kind_class,
            "mutating",
        )
        component_schema = get_tool("ui_transfers_components_plan").parameters[
            "properties"
        ]["components"]["items"]
        self.assertFalse(component_schema["additionalProperties"])
        leg_properties = component_schema["properties"]["legs"]["items"]["properties"]
        self.assertIn("untracked_wallet", leg_properties)
        self.assertIn("valuation_unit", leg_properties)
        self.assertIn("valuation_amount", leg_properties)
        self.assertNotIn("location_ref", leg_properties)
        self.assertNotIn("wallet_id", leg_properties)
        self.assertIn("suspense", leg_properties["role"]["enum"])
        self.assertIn("incomplete draft", leg_properties["role"]["description"])
        self.assertIn(
            "Pass its input_version unchanged as expected_input_version",
            get_tool("ui_transfers_components_plan").description,
        )
        valid_component_arguments = {
            "components": [
                {
                    "component_type": "manual_bridge",
                    "legs": [
                        {
                            "id": "source",
                            "role": "source",
                            "transaction": "out-tx",
                            "amount_msat": "10000000000000001",
                            "valuation_unit": "eur-cent",
                            "valuation_amount": "5000001",
                        },
                        {
                            "id": "destination",
                            "role": "destination",
                            "untracked_wallet": "Missing owned wallet",
                            "amount_msat": "10000000000000001",
                            "valuation_unit": "eur-cent",
                            "valuation_amount": "5000001",
                        },
                    ],
                }
            ],
        }
        daemon_runtime._validate_ai_tool_arguments(
            get_tool("ui_transfers_components_plan"),
            valid_component_arguments,
        )
        invalid_component_arguments = json.loads(json.dumps(valid_component_arguments))
        invalid_component_arguments["components"][0]["legs"][0]["location_ref"] = "/tmp/private"
        with self.assertRaisesRegex(AppError, "unsupported field"):
            daemon_runtime._validate_ai_tool_arguments(
                get_tool("ui_transfers_components_plan"),
                invalid_component_arguments,
            )
        self.assertEqual(get_tool("ui_rates_summary").kind_class, "read_only")
        self.assertEqual(get_tool("ui_rates_coverage").name, "ui.rates.coverage")
        self.assertEqual(get_tool("ui_rates_rebuild").name, "ui.rates.rebuild")
        self.assertEqual(get_tool("ui_rates_rebuild").kind_class, "mutating")
        self.assertEqual(get_tool("ui_report_blockers").name, "ui.report.blockers")
        self.assertEqual(get_tool("ui_reports_report_blockers").name, "ui.report.blockers")
        self.assertEqual(
            get_tool("ui_audit_changes_since_last_answer").name,
            "ui.audit.changes_since_last_answer",
        )
        self.assertEqual(
            get_tool("ui_maintenance_settings").name,
            "ui.maintenance.settings",
        )
        self.assertEqual(get_tool("ui_wallets_sync").name, "ui.wallets.sync")
        self.assertEqual(get_tool("ui.wallets.sync").kind_class, "mutating")
        self.assertEqual(get_tool("ui_journals_process").name, "ui.journals.process")
        self.assertEqual(get_tool("ui.journals.process").kind_class, "mutating")
        self.assertEqual(
            get_tool("ui_maintenance_configure").name,
            "ui.maintenance.configure",
        )
        self.assertEqual(get_tool("ui_maintenance_configure").kind_class, "mutating")
        self.assertEqual(get_tool("ui_maintenance_run").name, "ui.maintenance.run")
        self.assertEqual(get_tool("ui_maintenance_run").kind_class, "mutating")
        review_context_schema = get_tool("ui_transfers_review_context").parameters
        self.assertEqual(
            review_context_schema["properties"]["candidate_type"]["enum"],
            ["transfer", "swap"],
        )
        self.assertIn(
            "provider_swap_id",
            review_context_schema["properties"]["method"]["enum"],
        )
        suggest_schema = get_tool("ui_transfers_suggest").parameters
        self.assertIn(
            "provider_swap_id",
            suggest_schema["properties"]["method"]["enum"],
        )

        self.assertIn("ui_wallets_sync", tool_names)
        self.assertIn("ui_journals_process", tool_names)
        self.assertIn("ui_maintenance_configure", tool_names)
        self.assertIn("ui_maintenance_run", tool_names)
        pair_schema = get_tool("ui_transfers_pair").parameters
        self.assertIn("coinjoin", pair_schema["properties"]["kind"]["enum"])
        self.assertIn("whirlpool", pair_schema["properties"]["kind"]["enum"])
        self.assertIn("chain-swap", pair_schema["properties"]["kind"]["enum"])
        self.assertIn(
            "reverse-submarine-swap",
            pair_schema["properties"]["kind"]["enum"],
        )
        self.assertIn("Coinjoin", get_tool("ui_transfers_pair").description)
        bulk_pair_schema = get_tool("ui_transfers_bulk_pair").parameters
        self.assertIn("method", bulk_pair_schema["properties"])
        self.assertIn(
            "provider_swap_id",
            bulk_pair_schema["properties"]["method"]["enum"],
        )

        journal_events_schema = get_tool("ui_journals_events_list").parameters
        self.assertEqual(
            set(journal_events_schema["properties"]),
            {"transaction", "limit"},
        )
        for tool_name in ("ui_attachments_list", "ui_source_funds_evidence_list"):
            schema = get_tool(tool_name).parameters
            self.assertEqual(schema["additionalProperties"], False)
            self.assertEqual(schema["properties"]["limit"]["maximum"], 200)
            self.assertEqual(schema["properties"]["cursor"]["type"], "string")

        workspace_schema = get_tool("ui_workspace_overview_snapshot").parameters
        self.assertEqual(workspace_schema["required"], ["workspace_id"])
        self.assertEqual(get_tool("ui_review_worklist").kind_class, "read_only")
        self.assertEqual(
            get_tool("ui_review_worklist").parameters["properties"]["limit"]["maximum"],
            50,
        )
        self.assertEqual(get_tool("ui_loans_list").kind_class, "read_only")
        self.assertEqual(
            set(get_tool("ui_loans_mark").parameters["required"]),
            {"txid", "as"},
        )
        self.assertEqual(get_tool("ui_loans_link").kind_class, "mutating")
        self.assertEqual(get_tool("ui_loans_unmark").kind_class, "mutating")

        payout_schema = get_tool("ui_transfers_payouts_create").parameters
        self.assertEqual(
            set(payout_schema["required"]),
            {"tx_out", "payout_asset", "payout_amount"},
        )
        self.assertEqual(
            payout_schema["properties"]["payout_asset"]["enum"],
            ["BTC", "LBTC", "LNBTC"],
        )
        self.assertEqual(get_tool("ui_transfers_payouts_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_transfers_payouts_delete").kind_class, "mutating")
        self.assertEqual(get_tool("ui_transfers_update").kind_class, "mutating")

        for tool_name in (
            "ui_source_funds_sources_attach",
            "ui_source_funds_links_attach",
            "ui_rates_latest",
        ):
            self.assertEqual(get_tool(tool_name).kind_class, "mutating")
        self.assertEqual(
            get_tool("ui_rates_latest").parameters["properties"]["source"]["enum"],
            ["coinbase-exchange", "coingecko"],
        )

    def test_live_tool_catalog_is_capability_scoped(self):
        report_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Export my 2025 Austrian tax report"}],
                screen_context={"route": "/reports"},
            )
        }
        self.assertIn("ui_reports_export", report_tools)
        self.assertIn("ui_reports_tax_summary", report_tools)
        self.assertNotIn("ui_source_funds_assemble", report_tools)

        transaction_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Explain this transaction"}],
                screen_context={"route": "/transactions"},
            )
        }
        self.assertIn("ui_transactions_review_context", transaction_tools)
        self.assertIn("ui_transactions_metadata_update", transaction_tools)
        self.assertNotIn("ui_documents_create", transaction_tools)
        self.assertNotIn("ui_profiles_snapshot", transaction_tools)

        workspace_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Show the treasury across all books"}],
                screen_context={"route": "/books"},
            )
        }
        self.assertIn("ui_workspace_overview_snapshot", workspace_tools)
        self.assertIn("ui_profiles_snapshot", workspace_tools)
        self.assertNotIn("ui_loans_mark", workspace_tools)

        loan_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Review my loan collateral marks"}],
            )
        }
        self.assertIn("ui_loans_list", loan_tools)
        self.assertIn("ui_loans_mark", loan_tools)
        self.assertNotIn("ui_workspace_overview_snapshot", loan_tools)

        review_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "What accounting review work remains?"}],
            )
        }
        self.assertIn("ui_review_worklist", review_tools)

        journal_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Process journals"}],
            )
        }
        self.assertIn("ui_journals_process", journal_tools)

        payout_tools = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "Record a reviewed direct payout"}],
            )
        }
        self.assertIn("ui_transfers_payouts_create", payout_tools)

        discovery_tools = build_openai_tools(
            [{"role": "user", "content": "What can you do? Show all tools."}]
        )
        self.assertEqual(len(discovery_tools), len(build_openai_tools()))

    def test_ai_tool_result_redacts_embedded_urls_and_absolute_paths(self):
        payload = {
            "message": (
                "Request https://private.example/secret?token=value failed while "
                "reading /Users/alice/Taxes/private.pdf"
            ),
            "nested": [
                r"C:\Users\Alice\Documents\private.csv",
                r"\\server\private-share\audit\statement.pdf",
            ],
            "asset_route": "BTC/LBTC/swap",
        }

        redacted = redact_ai_tool_result(payload)
        encoded = json.dumps(redacted, sort_keys=True)

        for private_value in (
            "private.example",
            "/Users/alice",
            "Alice",
            "private-share",
        ):
            self.assertNotIn(private_value, encoded)
        self.assertIn("<redacted-url>", encoded)
        self.assertIn("<redacted-path>", encoded)
        self.assertEqual(redacted["asset_route"], "BTC/LBTC/swap")
        punctuated = redact_ai_tool_result(
            "Saved to /Users/alice/Alice's Taxes/private (final), #1.pdf then uploaded"
        )
        self.assertEqual(punctuated, "Saved to <redacted-path>")
        for leaked_tail in ("Alice", "Taxes", "private", "final", "uploaded"):
            self.assertNotIn(leaked_tail, punctuated)
        # Model-supplied UI routes are not local file paths and remain useful
        # in consent/screen-context previews.
        self.assertEqual(
            redact_tool_arguments({"route": "/transactions"}),
            {"route": "/transactions"},
        )

    def test_core_tool_profile_keeps_common_accounting_tools_only(self):
        tool_names = {
            tool["name"]
            for tool in build_openai_tools(profile="core")
            if tool.get("type") == "function"
        }
        self.assertIn("status", tool_names)
        self.assertNotIn("ui_source_funds_sources_create", tool_names)
        self.assertNotIn("ui_connections_node_snapshot", tool_names)
        self.assertLess(len(tool_names), len(build_openai_tools(profile="full")))

        discovery_names = {
            tool["name"]
            for tool in build_openai_tools(
                [{"role": "user", "content": "What can you do?"}],
                profile="core",
            )
        }
        self.assertNotIn("ui_source_funds_sources_create", discovery_names)
        self.assertNotIn("ui_connections_node_snapshot", discovery_names)

    def test_mutating_tool_preview_redacts_secret_like_arguments(self):
        tool = get_tool("ui.wallets.sync")
        self.assertEqual(
            redact_tool_arguments(
                {
                    "wallet": "cold",
                    "descriptor": "wpkh(xpub...)",
                    "mnemonic": "abandon abandon abandon",
                    "recovery_phrase": "abandon abandon abandon",
                    "seed": "00" * 32,
                    "wif": "Kxabc",
                    "xprv": "xprv9s21...",
                    "nested": {"api_token": "secret"},
                }
            ),
            {
                "wallet": "cold",
                "descriptor": "<redacted>",
                "mnemonic": "<redacted>",
                "recovery_phrase": "<redacted>",
                "seed": "<redacted>",
                "wif": "<redacted>",
                "xprv": "<redacted>",
                "nested": {"api_token": "<redacted>"},
            },
        )
        self.assertEqual(
            summarize_tool_call(tool, {"wallet": "cold"}),
            "Refresh source cold",
        )
        # `debug` carries sanitized tracebacks on error envelopes; it must be
        # dropped (not just masked) at every nesting level so an embedded
        # envelope can never carry one into provider-bound content.
        self.assertEqual(
            redact_tool_arguments(
                {
                    "ok": False,
                    "envelope": {
                        "kind": "error",
                        "error": {
                            "code": "internal_error",
                            "debug": "Traceback: secret at kassiber/daemon.py",
                            "message": "boom",
                        },
                    },
                    "debug": "top-level traceback",
                }
            ),
            {
                "ok": False,
                "envelope": {
                    "kind": "error",
                    "error": {"code": "internal_error", "message": "boom"},
                },
            },
        )
        journal_tool = get_tool("ui.journals.process")
        self.assertEqual(summarize_tool_call(journal_tool, {}), "Process journals")
        quarantine_tool = get_tool("ui.journals.quarantine.resolve")
        self.assertEqual(
            summarize_tool_call(
                quarantine_tool,
                {"transaction": "tx-1", "action": "price_override"},
            ),
            "Apply reviewed price to tx-1",
        )
        self.assertEqual(
            summarize_tool_call(
                quarantine_tool,
                {"transaction": "tx-1", "action": "exclude"},
            ),
            "Exclude tx-1 from accounting",
        )
        components_tool = get_tool("ui.transfers.components.plan")
        component_properties = components_tool.parameters["properties"]["components"][
            "items"
        ]["properties"]
        self.assertNotIn("conversion_reviewed", component_properties)
        self.assertEqual(
            summarize_tool_call(
                components_tool,
                {"components": [{}, {}]},
            ),
            "Preview 2 gap-resolution components",
        )
        rates_tool = get_tool("ui.rates.rebuild")
        self.assertEqual(
            summarize_tool_call(rates_tool, {"pair": "BTC-EUR"}),
            "Fetch spot prices for BTC-EUR",
        )
        maintenance_tool = get_tool("ui.maintenance.run")
        self.assertEqual(
            summarize_tool_call(maintenance_tool, {"sync": "never"}),
            "Process journals without source refresh",
        )
        configure_tool = get_tool("ui.maintenance.configure")
        self.assertEqual(
            summarize_tool_call(
                configure_tool,
                {"auto_sync_before_report_reads": True},
            ),
            "Enable freshness refresh before report reads",
        )
        self.assertEqual(
            summarize_tool_call(
                configure_tool,
                {"market_rate_provider": "coingecko"},
            ),
            "Set market-rate provider to coingecko",
        )

    def test_read_skill_reference_allowlist(self):
        self.assertIn("index", SKILL_REFERENCE_NAMES)
        self.assertIn("swap-matching", SKILL_REFERENCE_NAMES)
        self.assertIn("wallets-backends", SKILL_REFERENCE_NAMES)
        index = read_skill_reference("index")
        self.assertEqual(index["name"], "index")
        self.assertIn("Kassiber In-App Skill Index", index["content"])
        self.assertIn("wallets-backends", index["content"])
        self.assertIn("swap-matching", index["content"])
        self.assertNotIn("kassiber backends create my-esplora", index["content"])
        fallback_reference = read_skill_reference(
            "swap-matching",
            root=Path("/definitely/missing/kassiber/references"),
        )
        self.assertEqual(fallback_reference["name"], "swap-matching")
        swap_reference = read_skill_reference("swap-matching")
        self.assertEqual(swap_reference["name"], "swap-matching")
        self.assertIn("Swap matching", swap_reference["content"])
        reference = read_skill_reference("wallets-backends")
        self.assertEqual(reference["name"], "wallets-backends")
        self.assertIn("Wallets and Backends", reference["content"])
        with self.assertRaises(AppError) as ctx:
            read_skill_reference("../AGENTS")
        self.assertEqual(ctx.exception.code, "tool_not_allowed")

    def test_system_prompt_omits_reference_bodies_until_requested(self):
        messages = build_chat_messages(
            [{"role": "user", "content": "What is pending?"}],
            system_prompt_kind="kassiber",
        )
        self.assertEqual(messages[0]["role"], "system")
        self.assertIn("read_skill_reference", messages[0]["content"])
        self.assertIn('name "index"', messages[0]["content"])
        self.assertIn("automatically refresh stale local journals", messages[0]["content"])
        self.assertIn("report blockers", messages[0]["content"])
        self.assertIn("Never output placeholders", messages[0]["content"])
        self.assertIn("summary report tool", messages[0]["content"])
        self.assertNotIn("kassiber backends create my-esplora", messages[0]["content"])
        self.assertLess(len(DEFAULT_KASSIBER_SYSTEM_PROMPT), 2000)


class ResponsesToolCallAccumulatorTest(unittest.TestCase):
    def test_accumulates_partial_arguments(self):
        accumulator = ResponsesToolCallAccumulator()
        first = accumulator.add_event(
            {
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "read_skill_reference",
                    "arguments": "",
                },
            }
        )
        self.assertEqual(first[0]["function"]["arguments"], "")
        accumulator.add_event(
            {
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "delta": '{"name":"wallets',
            }
        )
        second = accumulator.add_event(
            {
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "delta": '-backends"}',
            }
        )
        self.assertEqual(second[0]["id"], "call_1")
        self.assertEqual(second[0]["function"]["name"], "read_skill_reference")
        self.assertEqual(
            second[0]["function"]["arguments"],
            '{"name":"wallets-backends"}',
        )


class HttpErrorMappingTest(unittest.TestCase):
    """`_http_error_app_error` decides whether errors are retryable, what
    `code` they get, and what hint the user sees. Pin those mappings."""

    def _err(self, status, body=b""):
        return urllib.error.HTTPError(
            url="http://test/v1/responses",
            code=status,
            msg="",
            hdrs=None,  # type: ignore[arg-type]
            fp=io.BytesIO(body),
        )

    def test_401_is_auth_failed(self):
        app_err = _http_error_app_error(self._err(401))
        self.assertEqual(app_err.code, "ai_auth_failed")
        self.assertFalse(app_err.retryable)

    def test_403_is_auth_failed(self):
        app_err = _http_error_app_error(self._err(403))
        self.assertEqual(app_err.code, "ai_auth_failed")

    def test_429_is_rate_limited_and_retryable(self):
        app_err = _http_error_app_error(self._err(429))
        self.assertEqual(app_err.code, "ai_rate_limited")
        self.assertTrue(app_err.retryable)

    def test_500_is_unavailable_and_retryable(self):
        app_err = _http_error_app_error(self._err(503))
        self.assertEqual(app_err.code, "ai_unavailable")
        self.assertTrue(app_err.retryable)

    def test_400_is_request_invalid(self):
        app_err = _http_error_app_error(self._err(400, b'{"error":"bad"}'))
        self.assertEqual(app_err.code, "ai_request_invalid")
        self.assertFalse(app_err.retryable)
        self.assertIn("body", app_err.details)

    def test_provider_error_body_redacts_echoed_secrets_and_truncates(self):
        body = (
            b'Authorization: Bearer sk-provider-header-secret {"api_key":"sk-provider-json-secret"} '
            b"plain sk-provider-plain-secret "
            + (b"x" * 2000)
        )
        app_err = _http_error_app_error(self._err(401, body))
        encoded = json.dumps(app_err.details, sort_keys=True)
        self.assertNotIn("sk-provider-header-secret", encoded)
        self.assertNotIn("sk-provider-json-secret", encoded)
        self.assertNotIn("sk-provider-plain-secret", encoded)
        self.assertIn("[redacted", encoded)
        self.assertTrue(app_err.details["body_truncated"])
        self.assertLessEqual(len(app_err.details["body"]), 512)

    def test_404_is_request_invalid(self):
        app_err = _http_error_app_error(self._err(404))
        self.assertEqual(app_err.code, "ai_request_invalid")

    def test_network_error_is_unavailable(self):
        app_err = _network_error_app_error(ConnectionRefusedError("nope"))
        self.assertEqual(app_err.code, "ai_unavailable")
        self.assertTrue(app_err.retryable)


class ClientDefaultsTest(unittest.TestCase):
    """Constructor invariants the rest of the system relies on."""

    def test_defaults(self):
        client = OpenAIResponsesClient(base_url="http://localhost:11434/v1")
        self.assertEqual(client.timeout, DEFAULT_TIMEOUT_SECONDS)
        self.assertIsNone(client.api_key)
        headers = client._headers(json_body=True)
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertNotIn("Authorization", headers)

    def test_bearer_when_key_present(self):
        client = OpenAIResponsesClient(base_url="http://x/v1", api_key="sk-test")
        headers = client._headers(json_body=False, accept_sse=True)
        self.assertEqual(headers["Authorization"], "Bearer sk-test")
        self.assertEqual(headers["Accept"], "text/event-stream")


class CliAIClientTest(unittest.TestCase):
    def test_codex_list_models_reads_visible_catalog(self):
        catalog = {
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "visibility": "list",
                    "supported_reasoning_levels": [
                        {"effort": "low"},
                        {"effort": "medium"},
                        {"effort": "high"},
                        {"effort": "xhigh"},
                    ],
                },
                {
                    "slug": "hidden-model",
                    "display_name": "Hidden",
                    "visibility": "hide",
                },
            ]
        }
        completed = subprocess.CompletedProcess(
            ["codex", "debug", "models"],
            0,
            stdout=json.dumps(catalog),
            stderr="",
        )
        with (
            patch("shutil.which", return_value="/usr/local/bin/codex"),
            patch("subprocess.run", return_value=completed),
        ):
            client = CliAIClient(locator="codex-cli://default")
            self.assertEqual(
                client.list_models(strict=True),
                [
                    {
                        "id": "gpt-5.5",
                        "check_kind": "codex_model_catalog",
                        "display_name": "GPT-5.5",
                        "supports_reasoning_effort": True,
                        "reasoning_efforts": ["low", "medium", "high", "xhigh"],
                    }
                ],
            )

    def test_cli_resolution_checks_common_gui_fallback_dirs(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "codex"
            binary.write_text("#!/bin/sh\n", encoding="utf-8")
            binary.chmod(0o755)

            with (
                patch("shutil.which", return_value=None),
                patch("kassiber.ai.client.CLI_FALLBACK_DIRS", (tmp,)),
            ):
                self.assertEqual(_resolve_cli_executable("codex"), str(binary))
                client = CliAIClient(locator="codex-cli://default")
                self.assertEqual(client.list_models()[0]["id"], CLI_DEFAULT_MODEL)

    def test_claude_list_models_uses_known_cli_aliases(self):
        with patch("shutil.which", return_value="/usr/local/bin/claude"):
            client = CliAIClient(locator="claude-cli://default")
            self.assertEqual(
                [row["id"] for row in client.list_models()],
                [CLI_DEFAULT_MODEL, "sonnet", "opus"],
            )

    def test_claude_args_keep_normal_cli_auth(self):
        client = CliAIClient(locator="claude-cli://default")
        args = client._claude_args(model="sonnet", effort="high")
        self.assertEqual(args[0], "claude")
        self.assertIn("--no-session-persistence", args)
        self.assertIn("--permission-mode", args)
        self.assertIn("--tools", args)
        self.assertIn("--model", args)
        self.assertIn("--effort", args)
        self.assertNotIn("--bare", args)

    def test_codex_args_use_current_exec_flags(self):
        client = CliAIClient(locator="codex-cli://default")
        args = client._codex_args(
            cwd="/tmp/kassiber-ai-cli",
            output_path="/tmp/kassiber-ai-cli-output",
            model="gpt-5.4",
            effort="medium",
        )
        self.assertEqual(args[:2], ["codex", "exec"])
        self.assertIn("--sandbox", args)
        self.assertIn("read-only", args)
        self.assertIn("--skip-git-repo-check", args)
        self.assertIn("--ephemeral", args)
        self.assertIn("--ignore-rules", args)
        self.assertIn("--output-last-message", args)
        self.assertIn("--model", args)
        self.assertIn("gpt-5.4", args)
        self.assertIn("-c", args)
        self.assertIn('model_reasoning_effort="medium"', args)
        self.assertEqual(args[-1], "-")
        self.assertNotIn("--ask-for-approval", args)

    def test_cli_chat_rejects_kassiber_tools(self):
        client = CliAIClient(locator="claude-cli://default")
        with self.assertRaises(AppError) as raised:
            client.chat(
                messages=[{"role": "user", "content": "hi"}],
                model=CLI_DEFAULT_MODEL,
                tools=[{"type": "function", "function": {"name": "status"}}],
                tool_choice="auto",
            )
        self.assertEqual(raised.exception.code, "ai_cli_tools_disabled")

    def test_cli_subprocess_env_drops_unrelated_secrets(self):
        with patch.dict(
            "os.environ",
            {
                "PATH": "/usr/bin",
                "KASSIBER_POC_TOKEN": "secret",
                "BTCPAY_API_KEY": "secret",
                "ANTHROPIC_API_KEY": "anthropic",
                "ANTHROPIC_AUTH_TOKEN": "anthropic-token",
                "ANTHROPIC_BASE_URL": "https://anthropic.example",
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "AWS_ACCESS_KEY_ID": "aws-key",
                "AWS_SECRET_ACCESS_KEY": "aws-secret",
                "OPENAI_API_KEY": "openai",
                "CODEX_API_KEY": "codex-key",
                "CODEX_ACCESS_TOKEN": "codex-token",
                "HTTPS_PROXY": "http://proxy.example",
                "NO_PROXY": "localhost,127.0.0.1",
            },
            clear=True,
        ):
            claude_env = _cli_subprocess_env("claude")
            codex_env = _cli_subprocess_env("codex")
        self.assertEqual(claude_env["ANTHROPIC_API_KEY"], "anthropic")
        self.assertEqual(claude_env["ANTHROPIC_AUTH_TOKEN"], "anthropic-token")
        self.assertEqual(claude_env["ANTHROPIC_BASE_URL"], "https://anthropic.example")
        self.assertEqual(claude_env["CLAUDE_CODE_USE_BEDROCK"], "1")
        self.assertEqual(claude_env["AWS_ACCESS_KEY_ID"], "aws-key")
        self.assertEqual(claude_env["AWS_SECRET_ACCESS_KEY"], "aws-secret")
        self.assertNotIn("OPENAI_API_KEY", claude_env)
        self.assertEqual(codex_env["OPENAI_API_KEY"], "openai")
        self.assertEqual(codex_env["CODEX_API_KEY"], "codex-key")
        self.assertEqual(codex_env["CODEX_ACCESS_TOKEN"], "codex-token")
        self.assertNotIn("ANTHROPIC_API_KEY", codex_env)
        for env in (claude_env, codex_env):
            self.assertEqual(env["HTTPS_PROXY"], "http://proxy.example")
            self.assertEqual(env["NO_PROXY"], "localhost,127.0.0.1")
            self.assertNotIn("KASSIBER_POC_TOKEN", env)
            self.assertNotIn("BTCPAY_API_KEY", env)
            self.assertEqual(env["NO_COLOR"], "1")


class ListModelsStrictModeTest(unittest.TestCase):
    """`list_models(strict=True)` must surface 4xx so `ai.test_connection`
    can tell a misconfigured base URL apart from a provider that simply
    doesn't expose `/v1/models`."""

    def _http_error(self, status):
        return urllib.error.HTTPError(
            url="http://x/v1/models",
            code=status,
            msg="",
            hdrs=None,  # type: ignore[arg-type]
            fp=io.BytesIO(b""),
        )

    class _FakeResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    def test_default_mode_swallows_4xx_to_empty_list(self):
        # Picker UX: providers that skip /v1/models still let the user fall
        # back to a configured default_model.
        with patch("urllib.request.urlopen", side_effect=self._http_error(404)):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])

    def test_strict_mode_propagates_4xx(self):
        with patch("urllib.request.urlopen", side_effect=self._http_error(404)):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_rejects_invalid_json_200(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(b"<html>not json</html>"),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_rejects_unexpected_200_shape(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(b'{"ok":true}'),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_does_not_change_auth_failure(self):
        # 401 was never swallowed; strict mode shouldn't change that path.
        with patch("urllib.request.urlopen", side_effect=self._http_error(401)):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            with self.assertRaises(AppError) as ctx:
                client.list_models()
            self.assertEqual(ctx.exception.code, "ai_auth_failed")
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_auth_failed")

    def test_list_models_keeps_safe_reasoning_capability_metadata(self):
        payload = {
            "data": [
                {
                    "id": "reasoner",
                    "owned_by": "provider",
                    "supports_reasoning_effort": True,
                    "supported_parameters": ["reasoning_effort", "stream"],
                    "reasoning_efforts": ["low", "medium", "high"],
                    "capabilities": {
                        "reasoning_effort": "supported",
                        "unsafe_blob": {"nested": "ignored"},
                    },
                }
            ]
        }
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(json.dumps(payload).encode("utf-8")),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            self.assertEqual(
                client.list_models(),
                [
                    {
                        "id": "reasoner",
                        "owned_by": "provider",
                        "supports_reasoning_effort": True,
                        "supported_parameters": ["reasoning_effort", "stream"],
                        "reasoning_efforts": ["low", "medium", "high"],
                        "capabilities": {"reasoning_effort": "supported"},
                    }
                ],
            )


class ResponsesBodyContractTest(unittest.TestCase):
    """Caller-supplied options must not override the Responses contract."""

    class _ReadResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    class _StreamResponse:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def __iter__(self):
            yield b"data: [DONE]\n"
            yield b"\n"

    def test_chat_forces_reserved_fields_after_options(self):
        captured: dict = {}

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            captured["request_url"] = request.full_url
            return self._ReadResponse(
                b'{"status":"completed","output":[{"type":"message","role":"assistant",'
                b'"content":[{"type":"output_text","text":"ok"}]}]}'
            )

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            client.chat(
                messages=[{"role": "user", "content": "real"}],
                model="real-model",
                options={
                    "stream": True,
                    "model": "wrong-model",
                    "messages": [],
                    "input": "wrong-input",
                    "store": True,
                    "temperature": 0.2,
                    "max_tokens": 42,
                    "reasoning_effort": "medium",
                },
            )
        self.assertEqual(captured["stream"], False)
        self.assertEqual(captured["request_url"], "http://x/v1/responses")
        self.assertEqual(captured["model"], "real-model")
        self.assertEqual(
            captured["input"],
            [{"type": "message", "role": "user", "content": "real"}],
        )
        self.assertFalse(captured["store"])
        self.assertNotIn("messages", captured)
        self.assertEqual(captured["temperature"], 0.2)
        self.assertEqual(captured["max_output_tokens"], 42)
        self.assertEqual(
            captured["reasoning"],
            {"effort": "medium", "summary": "auto"},
        )

    def test_stream_chat_forces_stream_true_after_options(self):
        captured: dict = {}

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            return self._StreamResponse()

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            list(
                client.stream_chat(
                    messages=[{"role": "user", "content": "real"}],
                    model="real-model",
                    options={
                        "stream": False,
                        "model": "wrong-model",
                        "messages": [],
                        "store": True,
                    },
                )
            )
        self.assertEqual(captured["stream"], True)
        self.assertEqual(captured["model"], "real-model")
        self.assertEqual(
            captured["input"],
            [{"type": "message", "role": "user", "content": "real"}],
        )
        self.assertFalse(captured["store"])

    def test_chat_sends_explicit_tools_after_options(self):
        captured: dict = {}
        tools = [{"type": "function", "function": {"name": "status", "parameters": {}}}]

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            return self._ReadResponse(
                b'{"status":"completed","output":[{"type":"message","role":"assistant",'
                b'"content":[{"type":"output_text","text":"ok"}]}]}'
            )

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            client.chat(
                messages=[{"role": "user", "content": "real"}],
                model="real-model",
                tools=tools,
                tool_choice={"type": "function", "function": {"name": "status"}},
                options={"tools": [], "tool_choice": "none"},
            )
        self.assertEqual(
            captured["tools"],
            [{"type": "function", "name": "status", "parameters": {}}],
        )
        self.assertEqual(
            captured["tool_choice"], {"type": "function", "name": "status"}
        )

    def test_chat_maps_multimodal_parts_json_format_and_timeout(self):
        captured: dict = {}

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            captured["request_timeout"] = timeout
            return self._ReadResponse(
                b'{"status":"completed","output":[{"type":"message",'
                b'"role":"assistant","content":[{"type":"output_text",'
                b'"text":"{}"}]}]}'
            )

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            client.chat(
                messages=[
                    {"role": "system", "content": "Extract JSON."},
                    {
                        "role": "user",
                        "content": [
                            {"type": "text", "text": "Read this image."},
                            {
                                "type": "image_url",
                                "image_url": {
                                    "url": "data:image/png;base64,AAAA",
                                    "detail": "high",
                                },
                            },
                        ],
                    },
                ],
                model="vision-model",
                options={"response_format": {"type": "json_object"}},
                timeout=123.0,
            )

        self.assertEqual(captured["instructions"], "Extract JSON.")
        self.assertEqual(
            captured["input"],
            [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "Read this image."},
                        {
                            "type": "input_image",
                            "image_url": "data:image/png;base64,AAAA",
                            "detail": "high",
                        },
                    ],
                }
            ],
        )
        self.assertEqual(captured["text"], {"format": {"type": "json_object"}})
        self.assertEqual(captured["request_timeout"], 123.0)


class ResponsesNormalizationTest(unittest.TestCase):
    """Typed Responses output is normalized for the existing daemon/UI."""

    class _FakeResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    def test_chat_includes_reasoning_when_provider_emits_it(self):
        payload = (
            b'{"status":"completed","output":['
            b'{"type":"reasoning","summary":[{"type":"summary_text",'
            b'"text":"thinking summary"}]},'
            b'{"type":"message","role":"assistant","content":['
            b'{"type":"output_text","text":"hi"}]}],'
            b'"usage":{"input_tokens":1,"output_tokens":2}}'
        )
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(payload),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            result = client.chat(
                messages=[{"role": "user", "content": "x"}], model="m"
            )
        self.assertEqual(result["content"], "hi")
        self.assertEqual(result["reasoning"], "thinking summary")
        self.assertEqual(result["finish_reason"], "stop")
        self.assertEqual(result["usage"]["input_tokens"], 1)

    def test_chat_normalizes_function_call_items(self):
        payload = (
            b'{"status":"completed","output":[{"type":"function_call",'
            b'"id":"fc_1","call_id":"call_1","name":"status",'
            b'"arguments":"{}"}]}'
        )
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(payload),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            result = client.chat(
                messages=[{"role": "user", "content": "x"}], model="m"
            )
        self.assertEqual(result["finish_reason"], "tool_calls")
        self.assertEqual(result["tool_calls"][0]["id"], "call_1")
        self.assertEqual(result["tool_calls"][0]["function"]["name"], "status")

    def test_stream_chat_normalizes_semantic_text_and_reasoning_events(self):
        class _StreamResponse:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def __iter__(self):
                yield (
                    b'data: {"type":"response.reasoning_summary_text.delta",'
                    b'"delta":"thinking summary"}\n'
                )
                yield b"\n"
                yield (
                    b'data: {"type":"response.output_text.delta","delta":"hi"}\n'
                )
                yield b"\n"
                yield (
                    b'data: {"type":"response.completed","response":'
                    b'{"status":"completed","output":[{"type":"reasoning",'
                    b'"summary":[{"type":"summary_text","text":"thinking summary"}]},'
                    b'{"type":"message","role":"assistant","content":'
                    b'[{"type":"output_text","text":"hi"}]}]}}\n'
                )
                yield b"\n"

        with patch("urllib.request.urlopen", return_value=_StreamResponse()):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            chunks = list(
                client.stream_chat(
                    messages=[{"role": "user", "content": "x"}], model="m"
                )
            )
        self.assertEqual(chunks[0].delta["reasoning"], "thinking summary")
        self.assertEqual(chunks[1].delta["content"], "hi")
        self.assertEqual(chunks[2].finish_reason, "stop")
        self.assertEqual(chunks[2].response_output[0]["type"], "reasoning")

    def test_stream_chat_uses_completed_payload_when_deltas_are_absent(self):
        class _StreamResponse:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def __iter__(self):
                yield (
                    b'data: {"type":"response.completed","response":'
                    b'{"status":"completed","output":[{"type":"reasoning",'
                    b'"summary":[{"type":"summary_text","text":"summary"}]},'
                    b'{"type":"message","role":"assistant","content":'
                    b'[{"type":"output_text","text":"answer"}]}]}}\n'
                )
                yield b"\n"

        with patch("urllib.request.urlopen", return_value=_StreamResponse()):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            chunks = list(
                client.stream_chat(
                    messages=[{"role": "user", "content": "x"}], model="m"
                )
            )
        self.assertEqual(chunks[0].delta["reasoning"], "summary")
        self.assertEqual(chunks[0].delta["content"], "answer")
        self.assertEqual(chunks[0].finish_reason, "stop")

    def test_stream_chat_accumulates_function_call_arguments(self):
        class _StreamResponse:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def __iter__(self):
                yield (
                    b'data: {"type":"response.output_item.added","output_index":0,'
                    b'"item":{"type":"function_call","call_id":"call_1",'
                    b'"name":"status","arguments":""}}\n'
                )
                yield b"\n"
                yield (
                    b'data: {"type":"response.function_call_arguments.delta",'
                    b'"output_index":0,"delta":"{}"}\n'
                )
                yield b"\n"
                yield (
                    b'data: {"type":"response.completed","response":'
                    b'{"status":"completed","output":[{"type":"function_call",'
                    b'"call_id":"call_1","name":"status","arguments":"{}"}]}}\n'
                )
                yield b"\n"

        with patch("urllib.request.urlopen", return_value=_StreamResponse()):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            chunks = list(
                client.stream_chat(
                    messages=[{"role": "user", "content": "x"}], model="m"
                )
            )
        self.assertEqual(chunks[-1].delta["tool_calls"][0]["id"], "call_1")
        self.assertEqual(
            chunks[-1].delta["tool_calls"][0]["function"]["arguments"],
            "{}",
        )
        self.assertEqual(chunks[-1].finish_reason, "tool_calls")

    def test_chat_omits_reasoning_when_provider_does_not_emit_it(self):
        payload = (
            b'{"status":"completed","output":[{"type":"message",'
            b'"role":"assistant","content":'
            b'[{"type":"output_text","text":"plain"}]}]}'
        )
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(payload),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            result = client.chat(
                messages=[{"role": "user", "content": "x"}], model="m"
            )
        self.assertEqual(result["content"], "plain")
        self.assertNotIn("reasoning", result)

    def test_stream_chat_maps_top_level_error_event(self):
        class _StreamResponse:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def __iter__(self):
                yield (
                    b'data: {"type":"error","code":"rate_limit_exceeded",'
                    b'"message":"slow down"}\n\n'
                )

        with patch("urllib.request.urlopen", return_value=_StreamResponse()):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            with self.assertRaises(AppError) as ctx:
                list(
                    client.stream_chat(
                        messages=[{"role": "user", "content": "x"}], model="m"
                    )
                )
        self.assertEqual(ctx.exception.code, "ai_rate_limited")
        self.assertTrue(ctx.exception.retryable)


class StreamChatErrorMappingTest(unittest.TestCase):
    """Read-time socket failures during a stream must map to retryable
    `ai_unavailable`, not bubble up as raw `URLError`/`OSError` and
    surface as non-retryable `internal_error` from the daemon thread."""

    class _ResponseRaisingMidIteration:
        """Fake urlopen response that yields one SSE event then breaks.

        urllib's response object iterates line-by-line, so the mock needs
        to surface the data line and the event-boundary blank line as
        separate iterations before raising — otherwise the parser is
        still waiting for the boundary when the timeout fires.
        """

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def __iter__(self):
            yield b'data: {"type":"response.output_text.delta","delta":"hi"}\n'
            yield b"\n"
            raise socket.timeout("read timed out mid-stream")

    def test_socket_timeout_mid_stream_maps_to_ai_unavailable(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._ResponseRaisingMidIteration(),
        ):
            client = OpenAIResponsesClient(base_url="http://x/v1")
            it = client.stream_chat(
                messages=[{"role": "user", "content": "hi"}], model="m"
            )
            # The first delta should arrive normally; the second iteration
            # raises the mapped AppError.
            first = next(it)
            self.assertEqual(first.delta.get("content"), "hi")
            with self.assertRaises(AppError) as ctx:
                next(it)
            self.assertEqual(ctx.exception.code, "ai_unavailable")
            self.assertTrue(ctx.exception.retryable)


class DaemonAiTestConnectionTest(unittest.TestCase):
    class _FakeClient:
        def list_models(self, *, strict: bool = False):
            if not strict:
                raise AssertionError("ai.test_connection must use strict model listing")
            return [{"id": "mlx-local", "owned_by": "local"}]

    def _ctx(self, conn):
        return daemon_runtime.DaemonContext(
            conn=conn,
            data_root="",
            runtime_config={},
            active_ai_chats=daemon_runtime.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon_runtime.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=None,
            freshness_stop_event=threading.Event(),
        )

    def test_connection_accepts_transient_api_key_before_save(self):
        captured: dict[str, object] = {}

        def fake_client_factory(**kwargs):
            captured.update(kwargs)
            return self._FakeClient()

        with tempfile.TemporaryDirectory(prefix="kassiber-ai-test-connection-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with patch(
                    "kassiber.daemon.ai_client_for_locator",
                    side_effect=fake_client_factory,
                ):
                    envelope, should_shutdown = daemon_runtime.handle_request(
                        self._ctx(conn),
                        {
                            "kind": "ai.test_connection",
                            "request_id": "test-1",
                            "args": {
                                "base_url": "http://127.0.0.1:8000/v1",
                                "api_key": "sk-unsaved-local",
                            },
                        },
                        out=None,
                    )
                self.assertFalse(should_shutdown)
                self.assertEqual(envelope["kind"], "ai.test_connection")
                self.assertEqual(envelope["data"]["model_count"], 1)
                self.assertEqual(captured["base_url"], "http://127.0.0.1:8000/v1")
                self.assertEqual(captured["api_key"], "sk-unsaved-local")
                self.assertEqual(captured["timeout"], 10.0)
            finally:
                conn.close()


class ProvidersCrudTest(unittest.TestCase):
    """SQLite-backed CRUD round-trip; the API key never leaks into output."""

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-")
        cls.data_root = Path(cls._tmp.name) / "data"

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _conn(self):
        return open_db(str(self.data_root))

    def test_seed_inserts_local_providers_when_empty(self):
        conn = self._conn()
        try:
            providers = list_db_ai_providers(conn)
            by_name = {provider["name"]: provider for provider in providers}
            self.assertEqual(set(by_name), {"ollama", "omlx"})
            self.assertEqual(by_name["ollama"]["kind"], "local")
            self.assertEqual(by_name["ollama"]["base_url"], "http://localhost:11434/v1")
            self.assertEqual(by_name["omlx"]["kind"], "local")
            self.assertEqual(by_name["omlx"]["base_url"], "http://127.0.0.1:8000/v1")
        finally:
            conn.close()

    def test_seed_can_use_container_host_ollama_base_url(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-seed-host-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with patch.dict(
                    os.environ,
                    {"KASSIBER_DEFAULT_AI_BASE_URL": "http://host.docker.internal:11434/v1"},
                ):
                    seed_default_ai_provider_if_empty(conn)
                providers = list_db_ai_providers(conn)
                by_name = {provider["name"]: provider for provider in providers}
                self.assertEqual(
                    by_name["ollama"]["base_url"],
                    "http://host.docker.internal:11434/v1",
                )
                self.assertEqual(by_name["omlx"]["base_url"], "http://127.0.0.1:8000/v1")
            finally:
                conn.close()

    def test_seed_can_select_omlx_as_default_provider(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-seed-omlx-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with patch.dict(
                    os.environ,
                    {
                        "KASSIBER_DEFAULT_AI_PROVIDER": "omlx",
                        "KASSIBER_DEFAULT_AI_BASE_URL": "http://127.0.0.1:8000/v1",
                    },
                ):
                    seed_default_ai_provider_if_empty(conn)
                payload = list_with_default(conn)
                self.assertEqual(payload["default"], "omlx")
                by_name = {provider["name"]: provider for provider in payload["providers"]}
                self.assertEqual(by_name["omlx"]["base_url"], "http://127.0.0.1:8000/v1")
                self.assertEqual(by_name["ollama"]["base_url"], "http://localhost:11434/v1")
            finally:
                conn.close()

    def test_seed_does_not_recreate_after_delete(self):
        # Use a fresh DB so the previous test's state doesn't bleed in.
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-seed-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                seed_default_ai_provider_if_empty(conn)
                clear_default_ai_provider(conn)
                delete_db_ai_provider(conn, "ollama")
                seed_default_ai_provider_if_empty(conn)
                self.assertNotIn(
                    "ollama",
                    {provider["name"] for provider in list_db_ai_providers(conn)},
                )
            finally:
                conn.close()

    def test_create_and_get_with_remote_kind(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-crud-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                created = create_db_ai_provider(
                    conn,
                    "OpenAI",
                    "https://api.openai.com/v1",
                    api_key="sk-secret",
                    default_model="gpt-4o-mini",
                    kind="remote",
                    notes="Cloud OpenAI.",
                )
                self.assertEqual(created["name"], "openai")
                self.assertEqual(created["display_name"], "OpenAI")
                self.assertEqual(created["kind"], "remote")
                self.assertIsNone(created["acknowledged_at"])

                fetched = get_db_ai_provider(conn, "openai")
                self.assertEqual(fetched["api_key"], "sk-secret")

                redacted = redact_ai_provider_for_output(fetched, default_name="ollama")
                self.assertNotIn("api_key", redacted)
                self.assertEqual(redacted["display_name"], "OpenAI")
                self.assertTrue(redacted["has_api_key"])
                self.assertEqual(redacted["secret_ref"]["store_id"], "sqlcipher_inline")
                self.assertEqual(redacted["secret_ref"]["state"], "ok")
                self.assertFalse(redacted["is_default"])
                self.assertFalse(redacted["supports_reasoning_effort"])

                with self.assertRaises(AppError) as ctx:
                    require_ai_provider_acknowledged(fetched)
                self.assertEqual(ctx.exception.code, "ai_remote_ack_required")

                acknowledged = update_db_ai_provider(
                    conn,
                    "openai",
                    {"acknowledged": True},
                )
                require_ai_provider_acknowledged(acknowledged)
            finally:
                conn.close()

    def test_update_clears_and_sets_fields(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-update-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "openrouter",
                    "https://openrouter.ai/api/v1",
                    api_key="sk-old",
                    kind="remote",
                )
                updated = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"api_key": "sk-new", "default_model": "anthropic/claude-3.5-sonnet"},
                )
                self.assertEqual(updated["api_key"], "sk-new")
                self.assertEqual(updated["secret_ref"]["state"], "ok")
                self.assertEqual(updated["default_model"], "anthropic/claude-3.5-sonnet")

                cleared = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"clear": ["api_key", "default_model"]},
                )
                self.assertIsNone(cleared["api_key"])
                self.assertEqual(cleared["secret_ref"]["state"], "missing")
                self.assertIsNone(cleared["default_model"])
            finally:
                conn.close()

    def test_update_clears_secret_when_base_url_changes_without_rotation(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-origin-change-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "openrouter",
                    "https://openrouter.ai/api/v1",
                    api_key="sk-old",
                    kind="remote",
                )
                updated = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"base_url": "https://example.invalid/v1"},
                )
                self.assertEqual(updated["base_url"], "https://example.invalid/v1")
                self.assertIsNone(updated["api_key"])
                self.assertEqual(updated["secret_ref"], {
                    "store_id": "sqlcipher_inline",
                    "service": updated["secret_ref"]["service"],
                    "account": "openrouter",
                    "state": "missing",
                    "created_at": updated["secret_ref"]["created_at"],
                    "rotated_at": updated["secret_ref"]["rotated_at"],
                })

                rotated = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"base_url": "https://api.openai.com/v1", "api_key": "sk-new"},
                )
                self.assertEqual(rotated["base_url"], "https://api.openai.com/v1")
                self.assertEqual(rotated["api_key"], "sk-new")
                self.assertEqual(rotated["secret_ref"]["state"], "ok")
            finally:
                conn.close()

    def test_narrow_set_api_key_updates_secret_ref_without_echo(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-secret-ref-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "openrouter",
                    "https://openrouter.ai/api/v1",
                    kind="remote",
                )
                updated = set_db_ai_provider_api_key(conn, "openrouter", "sk-rotated")
                self.assertEqual(get_ai_provider_api_key_for_use(updated), "sk-rotated")
                redacted = redact_ai_provider_for_output(updated)
                encoded = json.dumps(redacted)
                self.assertNotIn("sk-rotated", encoded)
                self.assertEqual(redacted["secret_ref"], {"store_id": "sqlcipher_inline", "state": "ok"})

                cleared = set_db_ai_provider_api_key(conn, "openrouter", None)
                self.assertIsNone(get_ai_provider_api_key_for_use(cleared))
                self.assertEqual(cleared["secret_ref"]["state"], "missing")
            finally:
                conn.close()

    def test_native_secret_ref_clears_inline_key_bytes(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-native-ref-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "openrouter",
                    "https://openrouter.ai/api/v1",
                    api_key="sk-inline-before-move",
                    kind="remote",
                )
                updated = set_db_ai_provider_native_secret_ref(
                    conn,
                    "openrouter",
                    store_id="macos_keychain",
                    service=ai_provider_secret_service_id(str((Path(tmp) / "data").resolve())),
                    account="openrouter",
                )
                self.assertIsNone(updated["api_key"])
                self.assertEqual(updated["secret_ref"]["store_id"], "macos_keychain")
                self.assertEqual(updated["secret_ref"]["state"], "ok")
                raw = conn.execute(
                    "SELECT api_key FROM ai_providers WHERE name = ?",
                    ("openrouter",),
                ).fetchone()["api_key"]
                self.assertIsNone(raw)
                redacted = redact_ai_provider_for_output(updated)
                self.assertTrue(redacted["has_api_key"])
                self.assertNotIn("sk-inline-before-move", json.dumps(redacted))
            finally:
                conn.close()

    def test_os_backed_secret_ref_fails_with_repair_details(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-secret-ref-missing-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "cloud",
                    "https://example.test/v1",
                    kind="remote",
                )
                conn.execute(
                    """
                    INSERT INTO ai_provider_secret_refs(
                        provider_name, store_id, service, account, state,
                        created_at, rotated_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "cloud",
                        "macos_keychain",
                        ai_provider_secret_service_id(str((Path(tmp) / "data").resolve())),
                        "cloud",
                        "missing",
                        "2026-05-13T00:00:00Z",
                        None,
                    ),
                )
                conn.commit()
                provider = get_db_ai_provider(conn, "cloud")
                with self.assertRaises(AppError) as ctx:
                    get_ai_provider_api_key_for_use(provider)
                self.assertEqual(ctx.exception.code, "secret_ref_unavailable")
                self.assertEqual(ctx.exception.details["refs"][0]["store_id"], "macos_keychain")
                self.assertEqual(ctx.exception.details["refs"][0]["state"], "missing")
            finally:
                conn.close()

    def test_os_backed_ok_ref_resolves_through_secret_resolver(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-secret-ref-ok-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "cloud",
                    "https://example.test/v1",
                    kind="remote",
                )
                conn.execute(
                    """
                    INSERT INTO ai_provider_secret_refs(
                        provider_name, store_id, service, account, state,
                        created_at, rotated_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "cloud",
                        "macos_keychain",
                        ai_provider_secret_service_id(str((Path(tmp) / "data").resolve())),
                        "cloud",
                        "ok",
                        "2026-05-13T00:00:00Z",
                        None,
                    ),
                )
                conn.commit()

                provider = get_db_ai_provider(conn, "cloud")
                redacted = redact_ai_provider_for_output(provider)
                self.assertTrue(redacted["has_api_key"])
                self.assertEqual(
                    redacted["secret_ref"],
                    {"store_id": "macos_keychain", "state": "ok"},
                )
                self.assertEqual(
                    get_ai_provider_api_key_for_use(
                        provider,
                        conn=conn,
                        secret_resolver=lambda ref: "sk-native",
                    ),
                    "sk-native",
                )
            finally:
                conn.close()

    def test_os_backed_ok_ref_without_resolver_persists_unavailable(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-secret-ref-unavailable-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "cloud",
                    "https://example.test/v1",
                    kind="remote",
                )
                conn.execute(
                    """
                    INSERT INTO ai_provider_secret_refs(
                        provider_name, store_id, service, account, state,
                        created_at, rotated_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "cloud",
                        "macos_keychain",
                        ai_provider_secret_service_id(str((Path(tmp) / "data").resolve())),
                        "cloud",
                        "ok",
                        "2026-05-13T00:00:00Z",
                        None,
                    ),
                )
                conn.commit()

                provider = get_db_ai_provider(conn, "cloud")
                with self.assertRaises(AppError) as ctx:
                    get_ai_provider_api_key_for_use(provider, conn=conn)
                self.assertEqual(ctx.exception.code, "secret_ref_unavailable")
                self.assertEqual(
                    ctx.exception.details["refs"][0]["state"], "unavailable"
                )
                persisted = conn.execute(
                    "SELECT state FROM ai_provider_secret_refs WHERE provider_name = ?",
                    ("cloud",),
                ).fetchone()["state"]
                self.assertEqual(persisted, "unavailable")
            finally:
                conn.close()

    def test_default_pointer_roundtrip(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-default-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "remote-1",
                    "https://example.test/v1",
                    api_key="sk-x",
                    kind="remote",
                )
                set_default_ai_provider(conn, "remote-1")
                self.assertEqual(resolve_ai_provider(conn)["name"], "remote-1")

                clear_default_ai_provider(conn)
                with self.assertRaises(AppError) as ctx:
                    resolve_ai_provider(conn)
                self.assertEqual(ctx.exception.code, "ai_provider_not_configured")
            finally:
                conn.close()

    def test_list_with_default_payload_shape(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-list-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                payload = list_with_default(conn)
                self.assertIn("providers", payload)
                self.assertIn("default", payload)
                # Seeded ollama is the default.
                self.assertEqual(payload["default"], "ollama")
                self.assertEqual(payload["providers"][0]["name"], "ollama")
                # No raw api_key in any redacted row.
                for row in payload["providers"]:
                    self.assertNotIn("api_key", row)
                    self.assertIn("secret_ref", row)
            finally:
                conn.close()

    def test_acknowledge_local_kind_is_implicit(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-ack-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                created = create_db_ai_provider(
                    conn,
                    "local-ollama",
                    "http://127.0.0.1:11434/v1",
                    kind="local",
                )
                self.assertIsNotNone(created["acknowledged_at"])
            finally:
                conn.close()

    def test_local_kind_requires_loopback_endpoint(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-local-loopback-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError) as ctx:
                    create_db_ai_provider(
                        conn,
                        "off-device",
                        "https://api.example/v1",
                        kind="local",
                    )
                self.assertEqual(ctx.exception.code, "validation")

                created = create_db_ai_provider(
                    conn,
                    "remote-device",
                    "https://api.example/v1",
                    kind="remote",
                )
                self.assertIsNone(created["acknowledged_at"])
                with self.assertRaises(AppError) as ack_ctx:
                    require_ai_provider_acknowledged(created)
                self.assertEqual(ack_ctx.exception.code, "ai_remote_ack_required")
            finally:
                conn.close()

    def test_cli_provider_requires_off_device_kind(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-cli-kind-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError) as ctx:
                    create_db_ai_provider(
                        conn,
                        "claude-cli",
                        "claude-cli://default",
                        kind="local",
                    )
                self.assertEqual(ctx.exception.code, "validation")

                created = create_db_ai_provider(
                    conn,
                    "codex-cli",
                    "codex-cli://default",
                    kind="remote",
                    default_model="default",
                )
                self.assertEqual(created["base_url"], "codex-cli://default")
                self.assertEqual(created["kind"], "remote")
                self.assertIsNone(created["acknowledged_at"])
            finally:
                conn.close()

    def test_delete_refuses_active_default(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-delete-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                # ollama is the seeded default
                with self.assertRaises(AppError) as ctx:
                    delete_db_ai_provider(conn, "ollama")
                self.assertEqual(ctx.exception.code, "conflict")
            finally:
                conn.close()

    def test_invalid_kind_rejected(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-kind-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError) as ctx:
                    create_db_ai_provider(
                        conn,
                        "x",
                        "http://x/v1",
                        kind="invalid",
                    )
                self.assertEqual(ctx.exception.code, "validation")
            finally:
                conn.close()

    def test_invalid_base_url_rejected(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-url-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError):
                    create_db_ai_provider(conn, "x", "no-scheme", kind="local")
            finally:
                conn.close()


class ChatHistorySeedTest(unittest.TestCase):
    """A branched/edited fork must round-trip its full seeded transcript."""

    def test_seeded_prefix_round_trips_via_get_session(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-chat-seed-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                workspace = core_accounts.create_workspace(
                    conn, "Demo", commit=False
                )
                profile = core_accounts.create_profile(
                    conn,
                    workspace["id"],
                    "Main",
                    "EUR",
                    None,
                    "generic",
                    365,
                    commit=False,
                )
                session = core_chat_history.create_session(
                    conn,
                    workspace["id"],
                    profile["id"],
                    title="branched chat",
                    provider="ollama",
                    model="qwen",
                    commit=False,
                )
                # Backfill the fork's seed (prior turns before the new prompt),
                # then append the live exchange — mirrors _persist_ai_chat_exchange
                # for a null-session branch/edit. Non user/assistant or empty
                # content is skipped.
                core_chat_history.append_messages(
                    conn,
                    session["id"],
                    [
                        {"role": "user", "content": "seed question"},
                        {"role": "assistant", "content": "seed answer"},
                        {"role": "system", "content": "ignored"},
                        {"role": "assistant", "content": ""},
                    ],
                    commit=False,
                )
                core_chat_history.append_exchange(
                    conn,
                    profile["id"],
                    session["id"],
                    user_content="follow-up",
                    assistant_content="reply",
                    commit=True,
                )

                stored = core_chat_history.get_session(
                    conn, profile["id"], session["id"]
                )
                self.assertEqual(stored["message_count"], 4)
                self.assertEqual(
                    [(m["role"], m["content"]) for m in stored["messages"]],
                    [
                        ("user", "seed question"),
                        ("assistant", "seed answer"),
                        ("user", "follow-up"),
                        ("assistant", "reply"),
                    ],
                )
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()

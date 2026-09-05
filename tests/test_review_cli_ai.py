"""Public CLI portability and bounded AI review schemas over the real service."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from kassiber.ai.prompt import build_responses_tools
from kassiber.ai.tools import get_tool, REVIEW_TOOL_NAMES, redact_ai_tool_result
from kassiber.daemon import _validate_ai_tool_arguments
from kassiber.cli.chat import _build_chat_args
from kassiber.cli.main import build_parser
from kassiber.db import open_db
from kassiber.errors import AppError
from tests import test_review_workflow


class ReviewCliAiTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        conn = open_db(self.tmp.name)
        test_review_workflow.ReviewWorkflowTest.seed(conn)
        conn.close()

    def cli(self, *args, success=True):
        result = subprocess.run(
            [sys.executable, '-m', 'kassiber', '--data-root', str(self.root), '--machine', *args],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def test_cli_paged_inspection_portable_preview_apply_and_receipt(self):
        scope = ('--workspace', 'w', '--profile', 'p')
        first = self.cli('review', 'cases', '--limit', '1', *scope)
        self.assertEqual(first['kind'], 'review.cases')
        data = first['data']
        self.assertEqual(data['cases'][0]['transaction_id'], 'a')
        second = self.cli('review', 'cases', '--cursor', data['next_cursor'], *scope)['data']
        self.assertEqual(second['cases'][0]['transaction_id'], 'b')
        self.assertIsNone(second['next_cursor'])
        operations = [
            {'type': 'price_override', 'transaction_id': 'a', 'fiat_rate': '20000', 'reason': 'Invoice reviewed'},
            {'type': 'exclude', 'transaction_id': 'b', 'reason': 'Duplicate source reviewed'},
        ]
        source = self.root / 'operations.json'
        source.write_text(json.dumps(operations))
        plan = self.cli('review', 'plan', '--operations-file', str(source),
                        '--expected-input-version', str(data['input_version']), *scope)
        artifact = plan['data']
        self.assertEqual(plan['kind'], 'review.plan')
        self.assertEqual(artifact['before']['quarantine_count'], 2)
        self.assertEqual(artifact['after']['quarantine_count'], 0)
        # The exact CLI artifact must be accepted by the AI schema too.
        _validate_ai_tool_arguments(get_tool('ui.review.plan'), {
            'operations': operations, 'expected_input_version': data['input_version'],
        })
        _validate_ai_tool_arguments(get_tool('ui.review.apply'), {
            'artifact': artifact, 'idempotency_key': 'cli-review',
        })
        self.assertEqual(redact_ai_tool_result(artifact), artifact)
        untouched = self.cli('review', 'cases', *scope)['data']
        self.assertEqual(len(untouched['cases']), 2)
        source.write_text(json.dumps(plan))
        applied = self.cli('review', 'apply', '--artifact-file', str(source), '--idempotency-key', 'cli-review', *scope)['data']
        self.assertEqual(applied['status'], 'verified')
        self.assertTrue(applied['verification']['report_ready'])
        receipt = self.cli('review', 'receipt', '--receipt-id', applied['id'], *scope)['data']
        self.assertEqual(receipt, applied)
        retried = self.cli('review', 'apply', '--artifact-file', str(source), '--idempotency-key', 'cli-review', *scope)['data']
        self.assertEqual(retried, receipt)
        self.assertEqual(self.cli('review', 'cases', *scope)['data']['cases'], [])

    def test_chat_default_budget_defers_to_selected_capability(self):
        args = build_parser().parse_args(["chat", "quarantine"])
        self.assertNotIn("tool_loop_max_iterations", _build_chat_args(args, []))
        args = build_parser().parse_args(["chat", "--tool-loop-max-iterations", "4", "quarantine"])
        self.assertEqual(_build_chat_args(args, [])["tool_loop_max_iterations"], 4)

    def test_cli_rejects_malformed_explicit_files(self):
        source = self.root / 'bad.json'
        source.write_text('{broken')
        error = self.cli('review', 'plan', '--operations-file', str(source), '--expected-input-version', '0',
                         '--workspace', 'w', '--profile', 'p', success=False)
        self.assertEqual(error['error']['code'], 'validation')

    def test_quarantine_pack_is_complete_and_bounded_for_both_profiles(self):
        for profile in ('core', 'scoped'):
            for question in ('Untersuche alle Quarantäne-Fälle mit Blockchain und Exports',
                             'Investigate quarantine using wallet evidence and report what remains'):
                schemas = build_responses_tools([{'role': 'user', 'content': question}], profile=profile)
                names = {get_tool(schema['name']).name for schema in schemas}
                self.assertTrue(REVIEW_TOOL_NAMES <= names)
                self.assertLessEqual(len(names), 40)
                self.assertNotIn('ui.reports.export', names)
                self.assertNotIn('ui.transfers.bulk_pair', names)
                self.assertNotIn('ui.source_funds.assemble', names)
        self.assertTrue(get_tool('ui.review.apply').requires_consent)
        self.assertFalse(get_tool('ui.review.plan').requires_consent)
        ordinary = {s['name'] for s in build_responses_tools([{'role':'user','content':'Show total balance'}])}
        self.assertNotIn('ui_review_apply', ordinary)

    def test_review_checkpoint_survives_repeated_terse_continuations(self):
        checkpoint = 'Paused.\n```json\n{"review_checkpoint":{"input_version":4,"next_cursor":"page-2"}}\n```'
        messages = [{'role': 'user', 'content': 'Investigate quarantine'}]
        for prompt in ('continue', 'weiter', 'fortsetzen', 'continue', 'Bitte weiter!'):
            messages.extend([
                {'role': 'assistant', 'content': checkpoint},
                {'role': 'user', 'content': prompt},
            ])
            for profile in ('core', 'scoped'):
                names = {get_tool(tool['name']).name for tool in build_responses_tools(messages, profile=profile)}
                self.assertTrue(REVIEW_TOOL_NAMES <= names)
                self.assertLessEqual(len(names), 40)

    def test_continuation_requires_last_assistant_valid_bounded_checkpoint(self):
        checkpoint = '```json\n{"review_checkpoint":{"receipt_ids":["receipt-1"]}}\n```'
        invalid = (
            'Ordinary answer',
            '```json\n{"review_checkpoint":[]}\n```',
            '```json\n{"review_checkpoint":broken}\n```',
            '```json\n{"review_checkpoint":{}}\n```',
            checkpoint + ('x' * 16_384),
        )
        for content in invalid:
            # An older valid checkpoint must not revive an unrelated last answer.
            messages = [{'role': 'assistant', 'content': checkpoint},
                        {'role': 'assistant', 'content': content},
                        {'role': 'user', 'content': 'continue'}]
            names = {tool['name'] for tool in build_responses_tools(messages, profile='core')}
            self.assertNotIn('ui_review_apply', names)
        names = {tool['name'] for tool in build_responses_tools([
            {'role': 'assistant', 'content': checkpoint},
            {'role': 'user', 'content': 'Show total balance'},
        ], profile='core')}
        self.assertNotIn('ui_review_apply', names)

    def test_ai_schema_rejects_gap_and_revision_privilege_expansion(self):
        tool = get_tool('ui.review.plan')
        for operation in (
            {'type':'custody_gap', 'request': {'action':'activate'}},
            {'type':'custody_component', 'request': {'action':'revise', 'components': []}},
            {'type':'exclude', 'transaction_id':'a', 'reason':'reviewed', 'workspace':'another'},
        ):
            with self.subTest(operation=operation), self.assertRaises(AppError):
                _validate_ai_tool_arguments(tool, {'operations':[operation], 'expected_input_version':0})
        component = {'component_type':'manual_bridge', 'conversion_reviewed': True, 'legs':[
            {'id':'source','role':'source','amount_msat':'1000'},
            {'id':'sink','role':'destination','amount_msat':'1000'},
        ]}
        with self.assertRaises(AppError):
            _validate_ai_tool_arguments(tool, {'operations':[{'type':'custody_component','request':{
                'action':'create','components':[component]}}], 'expected_input_version':0})
        for limit in (0, 101):
            with self.assertRaises(AppError):
                _validate_ai_tool_arguments(get_tool('ui.review.cases'), {'limit':limit})

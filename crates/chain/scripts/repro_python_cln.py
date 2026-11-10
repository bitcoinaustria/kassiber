import json
from pathlib import Path
import subprocess
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.lightning import cln
from kassiber.errors import AppError

SECRET = 'SYNTHETIC_CLN_SECRET'
with patch.object(cln.egress_policy, 'require_online'), patch.object(cln.subprocess, 'run', return_value=subprocess.CompletedProcess([], 7, '', SECRET)):
    try:
        cln.call_core_lightning({'rpc_file': SECRET}, 'getinfo')
    except AppError as error:
        stderr_disclosure = SECRET in error.details['stderr']
        command_disclosure = SECRET in str(error.details['command'])
try:
    cln._parse_msat(SECRET)
except AppError as error:
    amount_disclosure = SECRET in str(error)
# Round 8 fixed two former quirks (43ba65200, 4f2854c61); these pin the fixes.
point = cln._channel_funding_outpoint({'funding': {'txid': 'ab'*32, 'output': 0}})
NOW_ISO, TWO_DAYS_AGO = '2025-01-10T00:00:00Z', 1736467200 - 2 * 86400
raw = cln.fetch_core_lightning_snapshot({}, rpc_call=lambda method, args: {'forwards': [{'status': 'settled', 'fee_msat': 2000, 'received_time': TWO_DAYS_AGO}]} if method == 'listforwards' else {})
with patch.object(cln, 'now_iso', lambda: NOW_ISO):
    one = cln.build_node_snapshot(raw, window_days=1)
    year = cln.build_node_snapshot(raw, window_days=365)
result = {
    'stderr_discloses_response': stderr_disclosure,
    'command_discloses_rpc_path': command_disclosure,
    'amount_error_discloses_value': amount_disclosure,
    'nested_output_zero_is_kept': point == 'ab' * 32 + ':0',
    'window_bounds_routing': (one.routing.routing_revenue_sat, year.routing.routing_revenue_sat) == (0, 2),
}
print(json.dumps(result, sort_keys=True))

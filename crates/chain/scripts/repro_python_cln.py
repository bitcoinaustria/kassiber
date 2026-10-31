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
point = cln._channel_funding_outpoint({'funding': {'txid': 'ab'*32, 'output': 0}})
raw = cln.fetch_core_lightning_snapshot({}, rpc_call=lambda method, args: {'forwards': [{'status': 'settled', 'fee_msat': 2000, 'received_time': 1}]} if method == 'listforwards' else {})
one = cln.build_node_snapshot(raw, window_days=1)
year = cln.build_node_snapshot(raw, window_days=365)
result = {
    'stderr_discloses_response': stderr_disclosure,
    'command_discloses_rpc_path': command_disclosure,
    'amount_error_discloses_value': amount_disclosure,
    'nested_output_zero_is_missing': point is None,
    'window_is_label_only': one.routing.routing_revenue_sat == year.routing.routing_revenue_sat == 2,
}
assert all(result.values())
print(json.dumps(result, sort_keys=True))

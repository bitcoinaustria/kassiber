"""Execute the shell funding decisions without Docker or real wallet material."""
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class LightningFundingTest(unittest.TestCase):
    def run_shell(self, body: str, *, bootstrap: bool = False, scenario: bool = False):
        common = (ROOT / "dev/regtest/lightning-common.sh").read_text()
        definitions = common
        if bootstrap:
            code = (ROOT / "dev/regtest/lightning-business-bootstrap.sh").read_text()
            code = "\n".join(line for line in code.splitlines() if not line.startswith("source "))
            definitions += "\n" + code.rsplit('main "$@"', 1)[0]
        if scenario:
            code = (ROOT / "dev/regtest/lightning-business-scenario.sh").read_text()
            # Only the pure funding helper; setup at script scope owns files.
            definitions += "\nensure_actor_wallet_funds() {" + code.split("ensure_actor_wallet_funds() {", 1)[1].split("\nrun_mainchain_topups()", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            return subprocess.run(
                ["bash", "-c", definitions + "\n" + body],
                cwd=directory, text=True, capture_output=True,
            )

    def test_explicit_historical_actor_funds_late_height_faucet(self):
        result = self.run_shell('''
ensure_faucet_wallet() { :; }
mine_to_faucet() { echo "$1" >> mined; }
btc() {
  case "$*" in
    getblockcount) echo 1208;;
    *getbalance) if [ -f funded ]; then echo 20; else echo 4; fi;;
    *getnewaddress*) echo regtest-address;;
    '-rpcwallet=generated-external sendtoaddress regtest-address 20') touch funded;;
    *) return 9;;
  esac
}
export KASSIBER_REGTEST_LIGHTNING_FUNDING_WALLET=generated-external
ensure_faucet_funds || exit $?
[ "$(cat mined)" = 1 ]
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_subsidy_exhaustion_is_not_reported_as_funded(self):
        result = self.run_shell('''
ensure_faucet_wallet() { :; }
mine_to_faucet() { :; }
btc() { case "$*" in getblockcount) echo 1208;; *getbalance) echo 4;; *) return 9;; esac; }
unset KASSIBER_REGTEST_LIGHTNING_FUNDING_WALLET
ensure_faucet_funds
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("needs at least 20 regtest BTC", result.stderr)

    def test_rpc_failure_inside_conditional_does_not_print_success(self):
        for function in ["fund_node_if_needed cln_router", "fund_lnd_if_needed"]:
            result = self.run_shell('''
cln_onchain_sat() { echo 0; }
cln_any_onchain_sat() { echo 0; }
lnd_onchain_sat() { echo 0; }
lnd_any_onchain_sat() { echo 0; }
cln_new_address() { echo regtest-address; }
lnd_new_address() { echo regtest-address; }
btc() { return 1; }
if ''' + function + '''; then exit 99; else exit $?; fi
''', bootstrap=True)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertNotIn("Funded", result.stdout)

    def test_bootstrap_stops_on_funding_failure(self):
        result = self.run_shell('''
wait_for_cln() { :; }
wait_for_lnd() { :; }
ensure_faucet_wallet() { :; }
ensure_faucet_funds() { :; }
fund_node_if_needed() { return 2; }
wait_for_node_funds() { echo should-not-wait; }
main
''', bootstrap=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("should-not-wait", result.stdout)

    def test_channel_utxo_failure_is_not_ignored(self):
        result = self.run_shell('''
cln_onchain_sat() { echo 0; }
cln_new_address() { echo regtest-address; }
btc() { return 1; }
if ensure_channel_funding_utxo cln_merchant 7500000; then exit 99; else exit $?; fi
''', bootstrap=True)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertNotIn("Funded", result.stdout)

    def test_actor_funding_failure_is_not_reported_as_success(self):
        result = self.run_shell('''
ensure_core_wallet() { :; }
wallet_balance_sat() { echo 0; }
btc() { case "$*" in *getnewaddress*) echo regtest-address;; *) return 1;; esac; }
if ensure_actor_wallet_funds synthetic-actor 100000; then exit 99; else exit $?; fi
''', scenario=True)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertNotIn("Funded", result.stdout)

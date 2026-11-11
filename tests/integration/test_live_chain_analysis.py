"""Disposable Core consensus/index oracle, never a synthetic accounting book.

Every outpoint and signature below is created by independent Core wallets. Only
this fixture signs/broadcasts; the product's analysis calls remain read-only.
The product is the Rust `kassiber` binary (`tests/integration/cli.py`): the
CLI for one-shot calls and `kassiber daemon` for queries that take a full
query document and for the recurring source, which only an unlocked daemon
runs. Standard library only (docs/plan/22-python-removal.md, decision D2).
"""
from __future__ import annotations

import base64
import json
import os
import queue
import subprocess
import tempfile
import threading
import time
import unittest
import uuid
from decimal import ROUND_HALF_EVEN, Context, Decimal, localcontext
from pathlib import Path
from urllib import error, request

from tests.integration import cli
from tests.integration.book import open_book
from tests.integration.env import skip_unless_integration
from tests.integration.psbt import Psbt

RPC_TIMEOUT = float(os.environ.get("KASSIBER_REGTEST_RPC_TIMEOUT", "300"))
PASSPHRASE = "disposable-core-integration"
# Raw stack material and wallet derivations that must never persist.
FORBIDDEN_KEYS = {"witness", "txinwitness", "scriptSig", "raw_hex", "preimage", "bip32_derivs", "xpubs"}


def _rpc(url, username, password, method, params=None, wallet=None):
    endpoint = url.rstrip("/") + (f"/wallet/{wallet}" if wallet else "")
    payload = json.dumps({"jsonrpc": "1.0", "id": "kassiber-chain-analysis", "method": method,
                          "params": [] if params is None else params}).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    try:
        with request.urlopen(req, timeout=RPC_TIMEOUT) as response:
            decoded = json.loads(response.read().decode("utf-8"))
    except error.HTTPError as exc:
        try:
            decoded = json.loads(exc.read().decode("utf-8"))
        except ValueError:
            raise AssertionError(f"Core {method} failed ({exc.code}): no JSON-RPC error body") from exc
    if decoded.get("error"):
        raise AssertionError(f"Core {method} failed: {decoded['error']}")
    return decoded.get("result")


def _msat(btc):
    value = Decimal(str(btc)) * 100_000_000_000
    if value != value.to_integral_value():
        raise AssertionError("Core returned a non-integral millisatoshi amount")
    return str(int(value))


def _feature(snapshot, code):
    return next(row for row in snapshot["features"] if row["code"] == code)


def _without_derivations(encoded):
    """Receiver response removes wallet metadata, as required by BIP78."""
    return Psbt.from_base64(encoded).without_derivations().to_base64()


def _guard_capable() -> bool:
    """Whether the binary carries the test socket guard (a test-harness build).

    A release build refuses an armed `KASSIBER_TEST_NO_EGRESS` with exit 78.
    """
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_TEST_NO_EGRESS")}
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    result = subprocess.run([*cli.prefix(), "--version"], env=env, capture_output=True, timeout=60)
    return result.returncode != 78


class Daemon:
    """One `kassiber daemon` session; requests are answered in order."""

    def __init__(self, data_root: Path, env: dict[str, str], *, passphrase: str | None = None):
        args = ["--data-root", str(data_root)]
        if passphrase is not None:
            args += ["--db-passphrase-fd", cli.InputFd(passphrase.encode() + b"\n")]
        self.process = cli.popen([*args, "daemon"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, text=True, env=env)
        self.lines: queue.Queue = queue.Queue()
        threading.Thread(target=lambda: [self.lines.put(line) for line in self.process.stdout], daemon=True).start()
        self.count = 0
        if passphrase is not None:
            unlocked = self.call("daemon.unlock", {"auth_response": {"passphrase_secret": passphrase}})
            if not unlocked.get("unlocked"):
                raise AssertionError(f"daemon did not unlock the book: {unlocked}")

    def call(self, kind, args=None):
        self.count += 1
        request_id = f"live-{self.count}"
        message = {"request_id": request_id, "kind": kind}
        if args is not None:
            message["args"] = args
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        while True:
            try:
                record = json.loads(self.lines.get(timeout=180))
            except queue.Empty:
                raise AssertionError(f"daemon gave no answer to {kind}") from None
            if record.get("request_id") != request_id:
                continue
            if record.get("kind") != kind:
                raise AssertionError(f"daemon {kind} failed: {record}")
            return record.get("data") or {}

    def close(self):
        if self.process.poll() is None:
            try:
                self.call("daemon.shutdown")
            except AssertionError:
                pass
            self.process.stdin.close()
            try:
                self.process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdout.close()
        self.process.stderr.close()


@skip_unless_integration
@unittest.skipUnless(os.environ.get("KASSIBER_DISPOSABLE_CHAIN_ANALYSIS") == "1",
                     "requires the dedicated disposable chain-analysis lane")
class LiveChainAnalysisTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.url = os.environ["KASSIBER_REGTEST_CORE_URL"]
        self.username = os.environ["KASSIBER_REGTEST_RPC_USER"]
        self.password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
        self.names = []
        self.addCleanup(self._unload_wallets)
        self.assertEqual(self.rpc("getblockchaininfo")["chain"], "regtest")
        self.assertIn("txospenderindex", self.rpc("getindexinfo"))
        self.miner = self.wallet()
        self.mining_address = self.address(self.miner)
        self.mine(101)
        self.directory = tempfile.TemporaryDirectory(prefix="kassiber-live-chain-analysis-")
        self.addCleanup(self.directory.cleanup)
        self.base = Path(os.path.realpath(self.directory.name))
        self.data_root = self.base / "book"
        self.env = {key: value for key, value in os.environ.items()
                    if key not in {"XDG_DATA_HOME", "KASSIBER_NO_EGRESS", "KASSIBER_TEST_NO_EGRESS"}}
        self.env["TMPDIR"] = str(self.base)
        self.passphrase: str | None = None
        self.chain_instance = str(uuid.uuid4())

    def create_book(self, *, encrypted=False):
        """A bound regtest book with the disposable node as its only backend."""
        self.cli("init")
        if encrypted:
            self.cli("secrets", "init", "--new-passphrase-fd", cli.InputFd(PASSPHRASE.encode() + b"\n"))
            self.passphrase = PASSPHRASE
        self.cli("workspaces", "create", "Live investigation")
        self.cli("profiles", "create", "--workspace", "Live investigation", "Regtest")
        self.cli("backends", "create", "node", "--kind", "bitcoinrpc", "--url", self.url,
                 "--chain", "bitcoin", "--network", "regtest",
                 "--username-fd", cli.InputFd(self.username.encode()),
                 "--password-fd", cli.InputFd(self.password.encode()))
        binding = ("--environment", "regtest", "--chain-instance-id", self.chain_instance)
        reviewed = self.cli("networks", "plan", *binding)
        self.assertTrue(reviewed["can_apply"], reviewed)
        self.cli("networks", "bind", *binding, "--plan-id", reviewed["plan_id"])

    # -- product ----------------------------------------------------------

    def cli(self, *args, env=None):
        prefix = ["--machine", "--data-root", str(self.data_root)]
        if self.passphrase is not None:
            prefix += ["--db-passphrase-fd", cli.InputFd(self.passphrase.encode() + b"\n")]
        result = cli.run([*prefix, *args], env=env or self.env, capture_output=True, text=True, timeout=600)
        if result.returncode != 0:
            raise AssertionError(f"kassiber {' '.join(map(str, args))} failed ({result.returncode})\n"
                                 f"stdout={result.stdout}\nstderr={result.stderr[-4000:]}")
        return json.loads(result.stdout)["data"]

    def daemon(self, env=None) -> Daemon:
        session = Daemon(self.data_root, env or self.env, passphrase=self.passphrase)
        self.addCleanup(session.close)
        return session

    def run_analysis(self, query, env=None):
        """The full query document through the daemon, as the desktop sends it."""
        session = Daemon(self.data_root, env or self.env, passphrase=self.passphrase)
        try:
            return session.call("ui.chain_analysis.query", query)
        finally:
            session.close()

    def privacy_mirror(self, env=None):
        """The local (unredacted) mirror the desktop shows; `reports
        privacy-mirror` prints the redacted one, whose investigation subjects
        are opaque references."""
        session = Daemon(self.data_root, env or self.env, passphrase=self.passphrase)
        try:
            return session.call("ui.reports.privacy_mirror", {})
        finally:
            session.close()

    def psbt_file(self, encoded):
        path = self.base / f"psbt-{uuid.uuid4().hex[:8]}.psbt"
        path.write_text(encoded, encoding="ascii")
        return path

    def analyze_psbt(self, encoded):
        return self.cli("chain-analysis", "psbt", "analyze", "--network", "regtest", "--file", self.psbt_file(encoded))

    def compare_psbts(self, before, after, payjoin):
        return self.cli("chain-analysis", "psbt", "compare", "--network", "regtest",
                        "--before", self.psbt_file(before), "--after", self.psbt_file(after),
                        "--payjoin", json.dumps(payjoin))

    def offline_env(self):
        """Reads that must not contact even the configured loopback node.

        A test-harness build gets the strict test socket guard, which aborts
        the process on any attempt, loopback included (the Python test patched
        `socket.connect` and `getaddrinfo` the same way). A release build
        refuses that guard, so it gets the product kill switch instead, which
        refuses every destination. Either way the read must still succeed.
        """
        env = dict(self.env)
        env["KASSIBER_NO_EGRESS"] = "1"
        if _guard_capable():
            env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        return env

    def table_rows(self, table):
        conn = open_book(self.data_root, readonly=True)
        try:
            return [tuple(row) for row in conn.execute('SELECT * FROM "' + table.replace('"', '""') + '"')]
        finally:
            conn.close()

    def count(self, table):
        conn = open_book(self.data_root, readonly=True)
        try:
            return conn.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
        finally:
            conn.close()

    # -- Core -------------------------------------------------------------

    def rpc(self, method, params=None, wallet=None):
        return _rpc(self.url, self.username, self.password, method, params, wallet)

    def wallet(self):
        name = f"chain-analysis-{uuid.uuid4().hex[:12]}"
        self.rpc("createwallet", [name])
        self.names.append(name)
        return name

    def address(self, wallet, kind="bech32"):
        return self.rpc("getnewaddress", ["", kind], wallet)

    def mine(self, count=1):
        self.rpc("generatetoaddress", [count, self.mining_address])
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            rows, tip = self.rpc("getindexinfo"), self.rpc("getblockcount")
            if all(rows.get(name, {}).get("synced") and rows[name].get("best_block_height", -1) >= tip
                   for name in ("txindex", "txospenderindex")):
                return
            time.sleep(0.1)
        self.fail("Core transaction/spender indexes did not reach the mined tip")

    def fund(self, wallet, amount=1, kind="bech32"):
        address = self.address(wallet, kind)
        txid = self.rpc("sendtoaddress", [address, amount], self.miner)
        self.mine()
        rows = self.rpc("listunspent", [1, 9999999, [address]], wallet)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["txid"], txid)
        return {"txid": txid, "vout": rows[0]["vout"]}

    def create_psbt(self, inputs, outputs, *, sequence=0xFFFFFFFD, locktime=0):
        return self.rpc("createpsbt", [[{**row, "sequence": sequence} for row in inputs], outputs, locktime])

    def process(self, encoded, *wallets, sign=True):
        for wallet in wallets:
            encoded = self.rpc("walletprocesspsbt", [encoded, sign, "ALL", True], wallet)["psbt"]
        return encoded

    def broadcast(self, encoded):
        final = self.rpc("finalizepsbt", [encoded])
        self.assertTrue(final["complete"])
        accepted = self.rpc("testmempoolaccept", [[final["hex"]]])[0]
        self.assertTrue(accepted["allowed"], accepted)
        txid = self.rpc("sendrawtransaction", [final["hex"]])
        self.mine()
        self.assertGreater(self.rpc("getrawtransaction", [txid, 1])["confirmations"], 0)
        return txid, self.rpc("decoderawtransaction", [final["hex"]])

    # -- product flows ----------------------------------------------------

    def acquire(self, subject, *, direction="backward", depth=1, maximum=1):
        plan = self.cli("chain-analysis", "acquire", "plan", "--backend", "node", "--chain", "bitcoin",
                        "--network", "regtest", "--direction", direction, "--depth", str(depth),
                        "--max-transactions", str(maximum), subject)
        result = self.cli("chain-analysis", "acquire", "apply", "--plan", json.dumps(plan))
        self.assertLessEqual(result["request_count"], plan["effects"]["max_requests"])
        self.assertLessEqual(result["acquired_count"], maximum)
        return result

    def query(self, subject, **args):
        return self.run_analysis({"mode": "trace", "subject": subject, "chain": "bitcoin", "network": "regtest",
                                  "direction": "forward", "depth": 12, **args})

    def assert_reference_only(self):
        for table in ("transactions", "wallets", "chain_observation_provenance", "custody_components"):
            self.assertEqual(self.count(table), 0, table)
        # The derived index the analysis reads: an investigation alone never
        # attaches a wallet to a node.
        overview = self.run_analysis({"mode": "overview", "chain": "bitcoin", "network": "regtest",
                                      "node_limit": 2000, "edge_limit": 6000})
        self.assertTrue(overview["nodes"])
        self.assertTrue(all(not node.get("wallet_ids") for node in overview["nodes"]))
        # Signature categories survive in features, but no raw stack material
        # or wallet derivations can survive reference-observation persistence.
        def inspect(value):
            if isinstance(value, dict):
                self.assertFalse(FORBIDDEN_KEYS & value.keys())
                for child in value.values():
                    inspect(child)
            elif isinstance(value, list):
                for child in value:
                    inspect(child)
        conn = open_book(self.data_root, readonly=True)
        try:
            payloads = [row[0] for row in conn.execute("SELECT payload_json FROM chain_analysis_observations")]
        finally:
            conn.close()
        self.assertTrue(payloads)
        for payload in payloads:
            inspect(json.loads(payload))
            self.assertNotIn(self.password, payload)
        # Mirror must summarize acquired observations without contacting even
        # the configured loopback node. An investigation alone cannot invent
        # wallet ownership, and public workbench handoffs use the same snapshot.
        offline = self.offline_env()
        mirror = self.privacy_mirror(offline)
        self.assertEqual(mirror["summary"]["status"], "unavailable")
        self.assertEqual(mirror["summary"]["owned_output_count"], 0)
        for finding in mirror["findings"]:
            target = self.run_analysis(finding["investigation"]["query"], env=offline)
            self.assertEqual(target["snapshot_id"], mirror["investigation"]["snapshot_id"])

    # -- tests ------------------------------------------------------------

    def test_authorized_block_range_resumes_and_reconciles_real_core_reorg(self):
        # A recurring source runs only in an unlocked encrypted book's daemon.
        self.create_book(encrypted=True)
        start = self.rpc("getblockcount")
        self.mine(2)
        tip = self.rpc("getblockcount")
        previous_tip = self.rpc("getblockhash", [tip])
        previous_coinbase = self.rpc("getblock", [previous_tip])["tx"][0]
        spec = {"backend": "node", "network": "regtest", "chain_instance_id": self.chain_instance,
                "mode": "blocks", "start_height": start, "end_height": tip, "blocks_per_run": 2,
                "max_requests": 200, "max_bytes": 100_000_000}
        reviewed = self.cli("chain-analysis", "sources", "plan", "--document", json.dumps(spec))
        source = self.cli("chain-analysis", "sources", "authorize", "--plan", json.dumps(reviewed))
        session = self.daemon()

        def run_source():
            """Queue a pass and return every grant state the worker passes through until it rests."""
            session.call("ui.chain_analysis.sources.run", {"id": source["id"]})
            states, deadline = [], time.monotonic() + 120
            while time.monotonic() < deadline:
                time.sleep(0.25)
                row = next(item for item in session.call("ui.chain_analysis.sources.list")["items"]
                           if item["id"] == source["id"])
                state = {key: row[key] for key in ("cursor_height", "verified_tip_height", "last_code", "status")}
                if not states or states[-1] != state:
                    states.append(state)
                if row["last_code"] in {"range_complete"} and row["next_run_at"] > time.time() + 60:
                    return states
            self.fail(f"the source never came to rest: {states}")

        # Each pass covers at most `blocks_per_run` blocks and resumes from
        # its cursor; the worker requeues itself until the range is done.
        states = run_source()
        progressed = [state for state in states if state["cursor_height"] is not None]
        self.assertEqual(progressed[0]["cursor_height"], start + 1, states)
        self.assertEqual(progressed[-1]["cursor_height"], tip, states)
        self.assertEqual(progressed[-1]["verified_tip_height"], tip, states)
        self.assertEqual(self.cli("transactions", "list", "--limit", "100"), [])
        self.rpc("invalidateblock", [previous_tip])
        # A new coinbase destination forces a distinct replacement even when
        # both blocks are mined within the same second.
        self.mining_address = self.address(self.miner)
        self.mine(1)
        replacement = self.rpc("getblockhash", [tip])
        self.assertNotEqual(replacement, previous_tip)
        replacement_coinbase = self.rpc("getblock", [replacement])["tx"][0]
        states = run_source()
        changed = next((state for state in states if state["last_code"] == "reorg_reconciling"), None)
        self.assertIsNotNone(changed, states)
        self.assertIsNone(changed["verified_tip_height"])
        self.assertEqual(states[-1]["cursor_height"], tip, states)
        self.assertEqual(states[-1]["verified_tip_height"], tip, states)
        # The replaced block's observations are inactive: the public snapshot
        # holds the three active range blocks' coinbases, the replacement's
        # and not the invalidated tip's.
        snapshot = session.call("ui.chain_analysis.query", {"mode": "overview", "observer": "public",
                                                           "chain": "bitcoin", "network": "regtest"})
        self.assertTrue(snapshot["nodes"])
        txids = {node.get("txid") for node in snapshot["nodes"] if node.get("kind") == "transaction"}
        self.assertEqual(len(txids), 3, txids)
        self.assertIn(replacement_coinbase, txids)
        self.assertNotIn(previous_coinbase, txids)
        self.assertEqual(self.cli("transactions", "list", "--limit", "100"), [])

    def test_mirror_personal_relevance_uses_core_owned_output_and_shared_evidence(self):
        self.create_book()
        alice, bob = self.wallet(), self.wallet()
        owned, counterparty = self.fund(alice), self.fund(bob)
        signed = self.process(self.create_psbt([owned, counterparty],
                                               [{self.address(alice): 1.4}, {self.address(bob): 0.5999}]), alice, bob)
        txid, _ = self.broadcast(signed)
        self.acquire(txid)
        funding = self.rpc("getrawtransaction", [owned["txid"], 1])
        output = funding["vout"][owned["vout"]]
        self.assertTrue(self.rpc("getaddressinfo", [output["scriptPubKey"]["address"]], alice)["ismine"])
        # Core is the ownership truth here. The wallet is created through the
        # CLI; its one owned output is written as a fixture row because only a
        # wallet sync would otherwise create it, and a sync would also import
        # the spend as accounting data this mirror check must not depend on.
        wallet = self.cli("wallets", "create", "--label", "Alice", "--kind", "custom",
                          "--chain", "bitcoin", "--network", "regtest")
        conn = open_book(self.data_root)
        try:
            profile = conn.execute("SELECT id, workspace_id FROM profiles").fetchone()
            now = "2026-09-06T12:00:00Z"
            conn.execute(
                "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,"
                "outpoint,confirmation_status,script_pubkey,spent_by,first_seen_at,last_seen_at) "
                "VALUES('owned',?,?,?,'bitcoin','regtest','BTC',?,?,?,?,'confirmed',?,?,?,?)",
                (profile["workspace_id"], profile["id"], wallet["id"], int(_msat(output["value"])), owned["txid"],
                 owned["vout"], f"{owned['txid']}:{owned['vout']}", output["scriptPubKey"]["hex"], txid, now, now),
            )
            conn.commit()
            tables = [row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
                      if not row[0].startswith(("chain_index_", "sqlite_"))]
        finally:
            conn.close()

        def source_state():
            return {table: sorted(self.table_rows(table), key=repr) for table in tables}

        before = source_state()
        offline = self.offline_env()
        mirror = self.privacy_mirror(offline)
        common_input = next(row for row in mirror["findings"] if row["code"] == "common_input_control")
        self.assertEqual(common_input["relevance"], "own_spend")
        self.assertIn("assumes_no_undetected_collaboration", common_input["assumptions"])
        self.assertEqual(mirror["summary"]["owned_output_count"], 1)
        # A cold read may materialize the derived index, never source data.
        self.assertEqual(source_state(), before)
        conn = open_book(self.data_root, readonly=True)
        try:
            all_tables = tables + [row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'chain_index_%'")]
        finally:
            conn.close()
        warm_before = {table: sorted(self.table_rows(table), key=repr) for table in all_tables}
        warm = self.privacy_mirror(offline)
        self.assertEqual(warm["investigation"]["snapshot_id"], mirror["investigation"]["snapshot_id"])
        # We know this fixture is collaborative because both Core wallets
        # signed it. The public observer only gets a conditional hypothesis.
        self.assertNotEqual(common_input["authority"], "observed")
        reopened = self.run_analysis(common_input["investigation"]["query"], env=offline)
        self.assertEqual(reopened["snapshot_id"], common_input["investigation"]["snapshot_id"])
        # A warm read writes nothing, the derived index included (the Python
        # test compared `total_changes`; this compares every table's rows).
        self.assertEqual({table: sorted(self.table_rows(table), key=repr) for table in all_tables}, warm_before)
        self.assertEqual(source_state(), before)

    def test_missing_intermediate_is_a_frontier_then_acquisition_recovers_exact_path_and_case(self):
        self.create_book()
        middle, destination, final_wallet = self.wallet(), self.wallet(), self.wallet()
        funding = self.fund(middle)["txid"]
        spend = self.rpc("sendtoaddress", [self.address(destination), 0.7], middle)
        self.mine()
        onward = self.rpc("sendtoaddress", [self.address(final_wallet), 0.5], destination)
        self.mine()
        # Observe only both endpoints. A valid outpoint on the last transaction
        # cannot conjure the omitted transaction's inputs or an ownership link.
        limited = self.acquire(funding, direction="forward")
        self.assertFalse(limited["complete"])
        self.assertIn("transaction_limit", {row["reason"] for row in limited["frontier"]})
        self.acquire(onward)
        path_args = {"mode": "path", "target": onward, "observer": "public"}
        before = self.query(funding, **path_args)
        self.assertEqual(before["paths"], [])
        self.assertTrue(before["frontier"])
        case = self.cli("chain-analysis", "cases", "save", "--title", "Missing intermediate",
                        "--query", json.dumps(before["query"]), "--expected-snapshot-id", before["snapshot_id"])
        observed = self.acquire(funding, direction="forward", depth=4, maximum=10)
        self.assertTrue(observed["complete"], observed)
        self.assertEqual(set(observed["transaction_ids"]), {funding, spend, onward})
        after = self.query(funding, **path_args)
        self.assertEqual(len(after["paths"]), 1)
        path = after["paths"][0]
        self.assertEqual(path["kind"], "physical")
        self.assertEqual(len(path["edge_ids"]), 4)
        edges = {row["id"]: row for row in after["edges"]}
        self.assertEqual([edges[ident]["kind"] for ident in path["edge_ids"]], ["creates", "spends", "creates", "spends"])
        self.assertEqual(after["clusters"], [])
        comparison = self.cli("chain-analysis", "cases", "compare", case["id"])
        self.assertTrue(comparison["changed"])
        self.assertIn(spend, {row.get("txid") for row in comparison["added_nodes"]})
        self.assertEqual(self.cli("chain-analysis", "cases", "get", case["id"])["result"]["paths"], [])
        # Every call is a fresh process that reopens the book: feature
        # evidence survives witness discard.
        snapshot = after["snapshot_id"]
        self.assertEqual(self.run_analysis(after["query"])["snapshot_id"], snapshot)
        trace = self.query(funding, observer="public")
        later_case = self.cli("chain-analysis", "cases", "save", "--title", "Before another confirmed spend",
                              "--query", json.dumps(trace["query"]), "--expected-snapshot-id", trace["snapshot_id"])
        tail = self.rpc("sendtoaddress", [self.address(middle), 0.3], final_wallet)
        self.mine()
        self.acquire(funding, direction="forward", depth=5, maximum=10)
        later = self.cli("chain-analysis", "cases", "compare", later_case["id"])
        self.assertTrue(later["changed"])
        self.assertIn(tail, {row.get("txid") for row in later["added_nodes"]})
        self.assertNotIn(tail, {row.get("txid") for row in
                                self.cli("chain-analysis", "cases", "get", later_case["id"])["result"]["nodes"]})
        self.assert_reference_only()

    def test_core_psbt_and_persisted_feature_parity_across_four_input_script_types(self):
        self.create_book()
        wallets = [self.wallet() for _ in range(4)]
        inputs = [self.fund(wallet, kind=kind)
                  for wallet, kind in zip(wallets, ("legacy", "p2sh-segwit", "bech32", "bech32m"))]
        outputs = [{self.address(self.miner, kind): amount}
                   for kind, amount in (("bech32m", 1), ("bech32", 1), ("legacy", 1.9999))]
        empty = self.create_psbt(inputs, outputs, locktime=self.rpc("getblockcount"))
        unknown = self.analyze_psbt(empty)
        self.assertIsNone(unknown["totals"]["input_msat"])
        self.assertIsNone(unknown["totals"]["fee_msat"])
        self.assertEqual(unknown["coverage"]["missing_input_indices"], [0, 1, 2, 3])
        signed = self.process(empty, *wallets)
        parsed = self.analyze_psbt(signed)
        core = self.rpc("decodepsbt", [signed])
        self.assertEqual(parsed["totals"]["fee_msat"], _msat(core["fee"]))
        self.assertEqual(_feature(parsed["features"], "input_script_types")["value"]["counts"],
                         {"p2pkh": 1, "p2sh": 1, "p2wpkh": 1, "p2tr": 1})
        signatures = _feature(parsed["features"], "signature_encodings")["value"]
        self.assertEqual(signatures["count"], 4)
        self.assertEqual(sorted(row["algorithm"] for row in signatures["observations"]),
                         ["ecdsa", "ecdsa", "ecdsa", "schnorr"])
        self.assertTrue(all(row["low_r"] and row["low_s"]
                            for row in signatures["observations"] if row["algorithm"] == "ecdsa"))
        txid, decoded = self.broadcast(signed)
        self.assertEqual(parsed["totals"]["final_vsize"], decoded["vsize"])
        # The product divides at 32 significant digits (the Python package set
        # that as its process-wide decimal context, which this test inherited).
        with localcontext(Context(prec=32, rounding=ROUND_HALF_EVEN)):
            expected_rate = Decimal(parsed["totals"]["fee_msat"]) / 1000 / decoded["vsize"]
        self.assertEqual(Decimal(parsed["totals"]["final_fee_rate_sat_vb"]), expected_rate)
        self.acquire(txid)
        result = self.query(txid)
        snapshot = next(row["features"] for row in result["transaction_features"] if row["subject"].endswith(txid))
        for code in ("transaction_version", "sequences", "absolute_locktime", "input_script_types",
                     "output_script_types", "signature_encodings", "output_value_pattern", "output_ordering"):
            self.assertEqual(_feature(snapshot, code)["value"], _feature(parsed["features"], code)["value"], code)
        self.assertNotIn(signed, json.dumps(result))
        self.assert_reference_only()

    def test_equal_output_joint_transaction_preserves_ambiguity_and_model_assumptions(self):
        self.create_book()
        alice, bob = self.wallet(), self.wallet()
        inputs = [self.fund(alice), self.fund(bob)]
        original = self.create_psbt(inputs, [{self.address(alice): 0.9999}, {self.address(bob): 0.9999}])
        alice_signed = self.process(original, alice)
        self.assertFalse(self.rpc("finalizepsbt", [alice_signed])["complete"])
        signed = self.process(alice_signed, bob)
        txid, _ = self.broadcast(signed)
        self.acquire(txid)
        result = self.query(txid, direction="both", observer="public")
        self.assertEqual(result["clusters"], [])
        self.assertFalse(any(row["kind"] == "hypothesis" for row in result["edges"]))
        self.assertIn("equal_output_groups", {row["code"] for row in result["findings"]})
        # Two equal-input/equal-output participants have three financial-flow
        # partitions, including the all-in-one partition. This is conditional
        # ambiguity, never a measurement of the participants' actual identities.
        entropy = self.cli("chain-analysis", "entropy", "--chain", "bitcoin", "--network", "regtest",
                           "--observer", "public", txid)
        self.assertEqual(entropy["status"], "exact", entropy)
        self.assertEqual(entropy["interpretation_count"], "3")
        self.assertFalse(entropy["deterministic_links"])
        self.assertTrue(entropy["assumptions"])
        with_hypotheses = self.query(txid, direction="both", observer="public", include_hypotheses=True)
        for edge in with_hypotheses["edges"]:
            if edge.get("rule") == "common_input_control":
                self.assertIn("assumes_no_undetected_collaboration", edge["premises"])
                self.assertFalse(edge["accounting_authority"])
        self.assert_reference_only()

    def test_real_payjoin_receiver_signature_fee_and_order_checks_then_core_acceptance(self):
        self.create_book()
        sender, receiver = self.wallet(), self.wallet()
        sender_input, receiver_input = self.fund(sender), self.fund(receiver, 0.25)
        payment, change = self.address(receiver), self.address(sender)
        original = self.process(self.create_psbt([sender_input], [{payment: 0.2}, {change: 0.79999}]), sender)
        # The receiver contributes 0.25 BTC and pays the 1000-sat fee increase.
        unsigned = self.create_psbt([sender_input, receiver_input], [{payment: 0.44999}, {change: 0.79999}])
        proposal = _without_derivations(self.process(self.process(unsigned, sender, sign=False), receiver))
        constraints = {"payment_output_index": 0, "additional_fee_output_index": 1,
                       "max_additional_fee_contribution_msat": "0"}
        before_mempool = self.rpc("getrawmempool")
        compared = self.compare_psbts(original, proposal, constraints)
        self.assertEqual(self.rpc("getrawmempool"), before_mempool)
        self.assertEqual(compared["delta"]["fee_msat"], "1000000")
        self.assertEqual(compared["payjoin"]["status"], "checks_passed", compared["payjoin"])
        self.assertEqual(compared["payjoin"]["receiver_contribution_msat"], "25000000000")
        self.assertIsNone(compared["payjoin"]["safe_to_sign"])
        self.assertFalse(compared["payjoin"]["negotiation_verified"])
        self.assertIsNone(compared["after"]["totals"]["final_fee_rate_sat_vb"])
        # Mutating a receiver-signed proposal must invalidate its signature and
        # output-order checks, even though scripts and aggregate fees still match.
        reordered = Psbt.from_base64(proposal).with_outputs_reversed().to_base64()
        rejected = self.compare_psbts(original, reordered, constraints)
        failed = {row["code"] for row in rejected["payjoin"]["checks"] if row["status"] == "failed"}
        self.assertIn("original_output_order_preserved", failed)
        self.assertIn("receiver_signature_commitments_valid", failed)
        finalized = self.process(proposal, sender)
        txid, decoded = self.broadcast(finalized)
        self.assertEqual(self.analyze_psbt(finalized)["totals"]["final_vsize"], decoded["vsize"])
        self.acquire(txid)
        self.assert_reference_only()

    def test_relative_lock_and_rbf_features_match_core_consensus_readiness(self):
        self.create_book()
        wallet = self.wallet()
        point = self.fund(wallet)
        encoded = self.process(self.create_psbt([point], [{self.address(self.miner): 0.9999}], sequence=2), wallet)
        result = self.analyze_psbt(encoded)
        sequence = _feature(result["features"], "sequences")["value"]
        self.assertTrue(sequence["signals_bip125"])
        self.assertEqual(sequence["relative_locks"], [{"input_index": 0, "unit": "blocks", "minimum": 2}])
        self.assertIn("relative_lock_constraints", {row["code"] for row in result["findings"]})
        final = self.rpc("finalizepsbt", [encoded])
        rejected = self.rpc("testmempoolaccept", [[final["hex"]]])[0]
        self.assertFalse(rejected["allowed"])
        self.assertEqual(rejected["reject-reason"], "non-BIP68-final")
        self.mine()
        txid, _ = self.broadcast(encoded)
        self.acquire(txid)
        # Absolute height locks are a different prerequisite from BIP68. A
        # non-final sequence enables nLockTime without signalling opt-in RBF.
        point = self.fund(wallet)
        height = self.rpc("getblockcount") + 2
        locked = self.process(self.create_psbt([point], [{self.address(self.miner): 0.9999}],
                                               sequence=0xFFFFFFFE, locktime=height), wallet)
        features = self.analyze_psbt(locked)["features"]
        self.assertEqual(_feature(features, "absolute_locktime")["value"],
                         {"value": height, "kind": "height", "enabled": True})
        self.assertFalse(_feature(features, "sequences")["value"]["signals_bip125"])
        self.assertEqual(_feature(features, "sequences")["value"]["relative_locks"], [])
        raw = self.rpc("finalizepsbt", [locked])["hex"]
        rejected = self.rpc("testmempoolaccept", [[raw]])[0]
        self.assertFalse(rejected["allowed"])
        self.assertEqual(rejected["reject-reason"], "non-final")
        self.mine(3)
        txid, _ = self.broadcast(locked)
        self.acquire(txid)
        self.assert_reference_only()

    def _unload_wallets(self):
        for name in reversed(self.names):
            try:
                self.rpc("unloadwallet", [name])
            except Exception:
                # EXIT cleanup still removes this lane's entire disposable node.
                pass


if __name__ == "__main__":
    unittest.main()

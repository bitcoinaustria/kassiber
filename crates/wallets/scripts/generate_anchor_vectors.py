#!/usr/bin/env python3
"""Record Python's compatibility history anchors (rounds 5 and 5b) for Rust.

Each scenario runs the unchanged Python Esplora or Electrum compatibility
adapter against a deterministic in-memory chain. Every backend request and
the reply it received are recorded in order, so the Rust acquisition can
replay the same replies through its loopback fakes and must issue the same
requests and derive the same freshness checkpoint. Nothing touches a network
or a data root; transactions are real serialized Bitcoin transactions, so
Electrum txid verification runs unchanged.

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/wallets/scripts/generate_anchor_vectors.py [--check]
"""
from __future__ import annotations

from contextlib import nullcontext
from dataclasses import replace
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.core import sync_backends as sb  # noqa: E402
from kassiber.core.sync import WalletSyncState  # noqa: E402
from kassiber.errors import AppError  # noqa: E402

OUT = ROOT / "crates/wallets/tests/vectors/python_history_anchors.json"
USED, UNUSED = 2, 1
BASE = 900000


def script(i):
    return "0014" + hashlib.sha256(f"anchor-{i}".encode()).hexdigest()[:40]


def raw_tx(tag, script_hex):
    """A coinbase-shaped transaction paying the script; no parent lookups."""
    spk = bytes.fromhex(script_hex)
    tag = hashlib.sha256(tag.encode()).digest()[:4]
    raw = (
        (2).to_bytes(4, "little") + b"\x01" + b"\x00" * 32 + b"\xff" * 4
        + bytes([len(tag)]) + tag + b"\xff" * 4 + b"\x01"
        + (100000).to_bytes(8, "little") + bytes([len(spk)]) + spk + b"\x00" * 4
    )
    return raw.hex(), hashlib.sha256(hashlib.sha256(raw).digest()).digest()[::-1].hex()


class World:
    """Heights map to fork tags; each block has a hash and an 80-byte header."""

    def __init__(self):
        self.chain = {h: "a" for h in range(BASE - 30, BASE + 1)}
        self.tip = BASE
        self.txs = {i: [(f"tx-{i}", BASE - 10 + i)] for i in range(USED)}
        self.hook = None
        self.lie = {}

    def bhash(self, h):
        return hashlib.sha256(f"{self.chain.get(h, 'a')}-{h}".encode()).hexdigest()

    def header(self, h):
        tag = self.chain.get(h, "a")
        stamp = 1700000000 + h + (0 if tag == "a" else 77)
        return (bytes.fromhex(self.bhash(h)) + b"\x00" * 36 + stamp.to_bytes(4, "little") + b"\x00" * 8).hex()

    def extend(self, n, tag=None):
        for _ in range(n):
            self.tip += 1
            self.chain[self.tip] = tag or self.chain[self.tip - 1]

    def reorg(self, depth, length, tag):
        for h in [h for h in self.chain if h > self.tip - depth]:
            del self.chain[h]
        self.tip -= depth
        self.extend(length, tag)

    def fire(self, i):
        if self.hook and self.hook[0] == i:
            action, self.hook = self.hook[1], None
            action(self)


def esplora_routes(world, log):
    indices = {sb.scriptpubkey_scripthash(script(i)): i for i in range(USED + UNUSED)}

    def tx(i, tag, height):
        _, txid = raw_tx(tag, script(i))
        on_chain = height in world.chain
        return dict(txid=txid, fee=0, vin=[],
                    vout=[dict(scriptpubkey=script(i), value=100000)],
                    status=dict(confirmed=True, block_height=height,
                                block_hash=world.lie.get("block_hash_in_status", world.bhash(height)) if on_chain else "aa" * 32,
                                block_time=1700000000 + height))

    def answer(path):
        if path in world.lie:
            value = world.lie[path]
            value = value(world) if callable(value) else value
            if isinstance(value, Exception):
                raise value
            return value
        if path == "blocks/tip/hash":
            return world.bhash(world.tip)
        if path == "blocks/tip/height":
            return str(world.tip)
        if path.startswith("block-height/"):
            h = int(path.split("/")[-1])
            if h not in world.chain or h > world.tip:
                raise AppError("Block not found", code="backend_error")
            return world.bhash(h)
        parts = path.split("/")
        i = indices[parts[1]]
        world.fire(i)
        entries = world.txs.get(i, [])
        if len(parts) == 2:
            count = len(entries)
            return dict(chain_stats=dict(tx_count=count, funded_txo_count=count, funded_txo_sum=100000 * count,
                                         spent_txo_count=0, spent_txo_sum=0), mempool_stats=dict(tx_count=0))
        if parts[2:] == ["txs", "chain"]:
            return [tx(i, tag, h) for tag, h in sorted(entries, key=lambda e: -e[1])]
        if parts[2:] == ["txs", "mempool"]:
            return []
        assert parts[2:] == ["utxo"], path
        return []

    def route(url, **_):
        path = url.split("example.invalid/", 1)[1]
        try:
            value = answer(path)
        except AppError:
            log.append(dict(path=path, status=404, body="not found"))
            raise
        log.append(dict(path=path, status=200, body=value if isinstance(value, str) else json.dumps(value)))
        return value
    return route


class ElectrumClient:
    def __init__(self, world, log):
        self.world, self.log = world, log
        # Each client context is a new connection (and protocol handshake).
        log.append(dict(connect=True))
        self.indices = {sb.scriptpubkey_scripthash(script(i)): i for i in range(USED + UNUSED)}
        self.raw = {}
        for i, entries in world.txs.items():
            for tag, _ in entries:
                hex_, txid = raw_tx(tag, script(i))
                self.raw[txid] = hex_

    def answer(self, method, params):
        w = self.world
        if (method, json.dumps(params)) in w.lie:
            value = w.lie[(method, json.dumps(params))]
            value = value(w) if callable(value) else value
            if isinstance(value, Exception):
                raise value
            return value
        if method == "blockchain.headers.subscribe":
            return {"height": w.tip, "hex": w.header(w.tip)}
        if method == "blockchain.block.header":
            if params[0] > w.tip or params[0] not in w.chain:
                raise AppError("bad height", code="backend_error")
            return w.header(params[0])
        i = self.indices.get(params[0])
        if method == "blockchain.scripthash.subscribe":
            w.fire(i)
            entries = w.txs.get(i, [])
            return hashlib.sha256(json.dumps(entries).encode()).hexdigest() if entries else None
        if method == "blockchain.scripthash.get_history":
            history = []
            for tag, h in w.txs.get(i, []):
                hex_, txid = raw_tx(tag, script(i))
                self.raw[txid] = hex_
                history.append(dict(tx_hash=txid, height=h))
            return history
        if method == "blockchain.scripthash.listunspent":
            unspent = [dict(tx_hash=raw_tx(tag, script(i))[1], tx_pos=0, height=h, value=100000)
                       for tag, h in w.txs.get(i, [])]
            if i == 1:
                # An unconfirmed output has no header to read.
                unspent.append(dict(tx_hash=raw_tx("mempool", script(i))[1], tx_pos=1, height=0, value=5000))
            return unspent
        assert method == "blockchain.transaction.get", method
        return self.raw[params[0]]

    def call(self, method, params):
        try:
            value = self.answer(method, params)
        except AppError as exc:
            self.log.append(dict(method=method, params=params, error=str(exc)))
            raise
        self.log.append(dict(method=method, params=params, result=value))
        return value

    def batch_call(self, calls):
        return [self.call(m, p) for m, p in calls]


def run(protocol, world, checkpoint):
    log = []
    backend = dict(name="fixture", kind=protocol, url="https://example.invalid/", batch_size=1)
    targets = [dict(script_pubkey=script(i), branch_index=0, address_index=i) for i in range(USED + UNUSED)]
    state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=None, policy_asset_id="",
                            targets=targets, tracked_scripts={t["script_pubkey"]: t for t in targets},
                            history_cache={}, checkpoint=checkpoint)
    route = esplora_routes(world, log)
    patches = dict(http_get_json=route, http_get_text=route,
                   _electrum_client_context=lambda _: nullcontext(ElectrumClient(world, log)))
    saved = {name: getattr(sb, name) for name in patches}
    try:
        for name, value in patches.items():
            setattr(sb, name, value)
        adapter = sb.compatibility_esplora_sync_adapter if protocol == "esplora" else sb.compatibility_electrum_sync_adapter
        _, meta = adapter(backend, {}, state)
    finally:
        for name, value in saved.items():
            setattr(sb, name, value)
    utxos = [{key: row[key] for key in ("txid", "vout", "confirmation_status", "confirmations", "block_height", "block_time")}
             for row in meta.get("utxos") or []]
    return dict(requests=log, freshness_checkpoint=meta["freshness_checkpoint"],
                scripts_changed=meta["scripts_changed"],
                utxos_skipped_unchanged=bool(meta.get("utxos_skipped_unchanged")), utxos=utxos)


def raced(world, height_shift=1):
    """A block arrives after the tip read: script 0 confirms in it."""
    def hook(w):
        w.extend(height_shift)
        w.txs[0] = w.txs[0] + [("raced", w.tip)]
    world.hook = (0, hook)


def raced_twice(world):
    """Two blocks arrive after the tip read; each confirms another script."""
    def hook(w):
        w.extend(2)
        w.txs[0] = w.txs[0] + [("raced", w.tip - 1)]
        w.txs[1] = w.txs[1] + [("raced-later", w.tip)]
    world.hook = (0, hook)


def scenarios(protocol):
    """(name, before-step mutation, checkpoint edit) on a fresh first sync."""
    yield "first_sync", None, None
    yield "same_tip", lambda w: None, None
    yield "extension_reuses", lambda w: w.extend(1), None
    yield "extension_by_many_reuses", lambda w: w.extend(5), None
    yield "taller_reorg_refreshes", lambda w: w.reorg(1, 3, "b"), None
    yield "same_height_reorg_refreshes", lambda w: w.reorg(1, 1, "b"), None
    yield "new_tx_on_extension", lambda w: (w.extend(1), w.txs[1].append(("late", w.tip))), None
    yield "raced_block_advances_anchor", raced, None
    yield "raced_two_blocks_advances_anchor", lambda w: raced(w, 2), None
    yield "raced_two_heights_advance_to_highest", raced_twice, None
    yield "legacy_anchor_version_refreshes", lambda w: w.extend(1), lambda cp: {
        k: v for k, v in cp.items() if k != "compatibility_history_anchor_version"}
    yield "old_anchor_version_refreshes", lambda w: None, lambda cp: dict(
        cp, compatibility_history_anchor_version=cp["compatibility_history_anchor_version"] - 1)
    yield "float_anchor_version_reuses", lambda w: None, lambda cp: dict(
        cp, compatibility_history_anchor_version=float(cp["compatibility_history_anchor_version"]))
    key = "esplora" if protocol == "esplora" else "electrum"
    for name, value in [("missing", None), ("bool", True), ("negative", -1), ("float", 1.0 * BASE),
                        ("too_large", 2**32), ("string", str(BASE))]:
        yield f"saved_height_{name}", lambda w: w.extend(1), lambda cp, v=value: dict(cp, **{f"{key}_tip_height": v})
    if protocol == "esplora":
        yield "tip_hash_unavailable", lambda w: w.lie.update({"blocks/tip/hash": AppError("down", code="backend_error")}), None
        yield "tip_hash_not_canonical", lambda w: w.lie.update({"blocks/tip/hash": " " + "AB" * 32 + "\n"}), None
        yield "tip_height_unavailable", lambda w: (w.extend(1), w.lie.update({"blocks/tip/height": AppError("down", code="backend_error")})), None
        yield "tip_height_not_integer", lambda w: (w.extend(1), w.lie.update({"blocks/tip/height": "tip"})), None
        yield "tip_height_hash_mismatch", lambda w: (w.extend(1), w.lie.update({"blocks/tip/height": str(BASE)})), None
        yield "previous_hash_unavailable", lambda w: (w.extend(1), w.lie.update({f"block-height/{BASE}": AppError("gone", code="backend_error")})), None
        yield "raced_status_hash_off_chain", lambda w: (raced(w), w.lie.update({"block_hash_in_status": "bb" * 32})), None
        yield "raced_initial_tip_reorged", lambda w: (raced(w), w.lie.update({f"block-height/{BASE}": lambda w: "cc" * 32})), None
        yield "raced_height_unavailable", lambda w: (raced(w), w.lie.update({f"block-height/{BASE + 1}": AppError("gone", code="backend_error")})), None
    else:
        yield "subscribe_unavailable", lambda w: w.lie.update({("blockchain.headers.subscribe", "[]"): AppError("down", code="backend_error")}), None
        yield "subscribe_short_header", lambda w: w.lie.update({("blockchain.headers.subscribe", "[]"): lambda w: {"height": w.tip, "hex": "00" * 79}}), None
        yield "subscribe_height_beyond_anchor", lambda w: (w.extend(1), w.lie.update({("blockchain.headers.subscribe", "[]"): lambda w: {"height": 2**32, "hex": w.header(w.tip)}})), None
        yield "subscribe_bool_height", lambda w: w.lie.update({("blockchain.headers.subscribe", "[]"): lambda w: {"height": True, "hex": w.header(w.tip)}}), None
        yield "previous_header_unavailable", lambda w: (w.extend(1), w.lie.update({("blockchain.block.header", json.dumps([BASE])): AppError("gone", code="backend_error")})), None
        yield "raced_header_changes_on_reread", lambda w: (raced(w), w.lie.update({("blockchain.block.header", json.dumps([BASE + 1])): changing_header(BASE + 1)})), None
        yield "raced_initial_header_changes", lambda w: (raced(w), w.lie.update({("blockchain.block.header", json.dumps([BASE])): changing_header(BASE, after=0)})), None


def changing_header(height, after=1):
    """The first `after` reads are honest; later reads return another fork."""
    seen = [0]
    def value(w):
        seen[0] += 1
        return w.header(height) if seen[0] <= after else ("dd" * 32) + w.header(height)[64:]
    return value


def build():
    cases = []
    for protocol in ("esplora", "electrum"):
        for name, mutate, edit in scenarios(protocol):
            world = World()
            first = run(protocol, world, {})
            if mutate is None:
                cases.append(dict(protocol=protocol, name=name, checkpoint={}, **first))
                continue
            checkpoint = first["freshness_checkpoint"]
            if edit is not None:
                checkpoint = edit(checkpoint)
            mutate(world)
            step = run(protocol, world, checkpoint)
            cases.append(dict(protocol=protocol, name=name, checkpoint=checkpoint, **step))
    return json.dumps(cases, ensure_ascii=True, indent=1, sort_keys=True) + "\n"


if __name__ == "__main__":
    text = build()
    if "--check" in sys.argv:
        raise SystemExit(0 if OUT.read_text() == text else "anchor vectors drifted")
    OUT.write_text(text)
    print(f"{len(json.loads(text))} anchor scenarios")

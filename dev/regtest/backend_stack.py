#!/usr/bin/env python3
from __future__ import annotations

import base64
import hashlib
import json
import os
import socketserver
import sys
import threading
import time
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib import error, request
from urllib.parse import parse_qs, unquote, urlparse


SATOSHIS = Decimal("100000000")
DEFAULT_BTC_EUR_PRICE = Decimal("58968.90")
DEFAULT_BTC_USD_PRICE = Decimal("63500.00")
BLECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _blech32_polymod(values: list[int]) -> int:
    generators = (
        0x7D52FBA40BD886,
        0x5E8DBF1A03950C,
        0x1C3A3C74072A18,
        0x385D72FA0E5139,
        0x7093E5A608865B,
    )
    checksum = 1
    for value in values:
        top = checksum >> 55
        checksum = ((checksum & 0x7FFFFFFFFFFFFF) << 5) ^ value
        for index, generator in enumerate(generators):
            if (top >> index) & 1:
                checksum ^= generator
    return checksum


def _blech32_hrp_expand(hrp: str) -> list[int]:
    return [ord(char) >> 5 for char in hrp] + [0] + [ord(char) & 31 for char in hrp]


def _replace_blech32_hrp(address: str, target_hrp: str) -> str:
    """Translate equivalent custom Elements HRPs used by LWK and Elements Core."""

    normalized = str(address).lower()
    separator = normalized.rfind("1")
    if separator < 1 or (str(address).lower() != address and str(address).upper() != address):
        raise ValueError("invalid confidential address")
    encoded = normalized[separator + 1 :]
    if len(encoded) < 12 or any(char not in BLECH32_CHARSET for char in encoded):
        raise ValueError("invalid confidential address")
    data = [BLECH32_CHARSET.index(char) for char in encoded]
    old_hrp = normalized[:separator]
    if _blech32_polymod(_blech32_hrp_expand(old_hrp) + data) != 1:
        raise ValueError("invalid confidential address checksum")
    payload = data[:-12]
    values = _blech32_hrp_expand(target_hrp) + payload + [0] * 12
    polymod = _blech32_polymod(values) ^ 1
    checksum = [(polymod >> (5 * (11 - index))) & 31 for index in range(12)]
    return target_hrp + "1" + "".join(BLECH32_CHARSET[value] for value in payload + checksum)


def _env_int(name: str, default: int) -> int:
    try:
        return int(os.environ.get(name) or default)
    except ValueError:
        return default


class RpcClient:
    def __init__(self, *, env_prefix: str = "BITCOIN", default_url: str = "http://bitcoind:18443") -> None:
        self.url = os.environ.get(f"{env_prefix}_RPC_URL", default_url).rstrip("/")
        self.user = os.environ.get(f"{env_prefix}_RPC_USER") or os.environ.get("BITCOIN_RPC_USER", "kassiber")
        self.password = os.environ.get(f"{env_prefix}_RPC_PASSWORD") or os.environ.get("BITCOIN_RPC_PASSWORD", "")

    def call(self, method: str, params: list[Any] | None = None) -> Any:
        payload = json.dumps(
            {"jsonrpc": "1.0", "id": f"kassiber-regtest-{method}", "method": method, "params": params or []}
        ).encode("utf-8")
        req = request.Request(self.url, data=payload, headers={"Content-Type": "application/json"})
        token = base64.b64encode(f"{self.user}:{self.password}".encode("utf-8")).decode("ascii")
        req.add_header("Authorization", f"Basic {token}")
        with request.urlopen(req, timeout=30) as response:
            body = json.loads(response.read().decode("utf-8"))
        if body.get("error"):
            raise RuntimeError(f"{method} failed: {body['error']}")
        return body.get("result")


def _btc_to_sats(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int((Decimal(str(value)) * SATOSHIS).to_integral_value())
    except Exception:
        return None


def _script_payload(output: dict[str, Any]) -> dict[str, Any]:
    script = output.get("scriptPubKey") if isinstance(output, dict) else {}
    if not isinstance(script, dict):
        script = {}
    result: dict[str, Any] = {
        "scriptpubkey": script.get("hex") or output.get("scriptpubkey") or "",
        "scriptpubkey_asm": script.get("asm") or output.get("scriptpubkey_asm") or "",
        "scriptpubkey_type": script.get("type") or output.get("scriptpubkey_type") or "unknown",
        "value": _btc_to_sats(output.get("value")),
    }
    address = script.get("address")
    if not address and isinstance(script.get("addresses"), list) and script["addresses"]:
        address = script["addresses"][0]
    if address:
        result["scriptpubkey_address"] = address
    return {key: value for key, value in result.items() if value is not None}


class BitcoinIndex:
    def __init__(self, rpc: RpcClient, *, chain: str = "bitcoin") -> None:
        self.rpc = rpc
        self.chain = chain
        self._lock = threading.Lock()
        self._cache_until = 0.0
        self._tip: tuple[int, str, tuple[str, ...]] | None = None
        self._txs: dict[str, dict[str, Any]] = {}
        self._history: dict[str, list[dict[str, Any]]] = {}
        self._utxos: dict[str, list[dict[str, Any]]] = {}

    def raw_tx(self, txid: str) -> dict[str, Any]:
        return self.rpc.call("getrawtransaction", [txid, True])

    def raw_hex(self, txid: str) -> str:
        return str(self.rpc.call("getrawtransaction", [txid, False]))

    def block_header(self, height: int) -> str:
        block_hash = str(self.rpc.call("getblockhash", [int(height)]))
        return str(self.rpc.call("getblockheader", [block_hash, False]))

    def tip_height(self) -> int:
        return int(self.rpc.call("getblockcount"))

    def esplora_tx(self, txid: str) -> dict[str, Any]:
        self._refresh()
        tx = self._txs.get(str(txid)) or self.raw_tx(txid)
        return self._to_esplora(tx)

    def _to_esplora(self, tx: dict[str, Any]) -> dict[str, Any]:
        total_inputs = 0
        total_outputs = 0
        result: dict[str, Any] = {
            "txid": tx.get("txid"),
            "version": tx.get("version"),
            "locktime": tx.get("locktime"),
            "size": tx.get("size"),
            "vsize": tx.get("vsize"),
            "weight": tx.get("weight"),
            "fee": 0,
            "vin": [],
            "vout": [],
            "status": {
                "confirmed": bool(tx.get("blockhash")),
                "block_height": tx.get("blockheight") or tx.get("height"),
                "block_hash": tx.get("blockhash"),
                "block_time": tx.get("blocktime") or tx.get("time"),
            },
        }
        for entry in tx.get("vin") or []:
            if not isinstance(entry, dict):
                continue
            vin: dict[str, Any] = {
                "txid": entry.get("txid"),
                "vout": entry.get("vout"),
                "is_coinbase": "coinbase" in entry,
                "scriptsig": (entry.get("scriptSig") or {}).get("hex") or "",
                "scriptsig_asm": (entry.get("scriptSig") or {}).get("asm") or "",
                "sequence": entry.get("sequence"),
                "witness": list(entry.get("txinwitness") or []),
            }
            if entry.get("txid") and entry.get("vout") is not None:
                prevout = self._prevout(entry.get("txid"), entry.get("vout"))
                if prevout:
                    vin["prevout"] = prevout
                    total_inputs += int(prevout.get("value") or 0)
            else:
                vin["prevout"] = None
            result["vin"].append(vin)
        for output in tx.get("vout") or []:
            if isinstance(output, dict):
                payload = _script_payload(output)
                if self.chain == "liquid":
                    payload.pop("scriptpubkey_address", None)
                total_outputs += int(payload.get("value") or 0)
                result["vout"].append({"n": output.get("n"), **payload})
        if total_inputs:
            result["fee"] = max(0, total_inputs - total_outputs)
        return result

    def _prevout(self, txid: Any, vout: Any) -> dict[str, Any] | None:
        try:
            index = int(vout)
            previous = self.raw_tx(str(txid))
            outputs = previous.get("vout") or []
            if index < 0 or index >= len(outputs) or not isinstance(outputs[index], dict):
                return None
            payload = _script_payload(outputs[index])
            if self.chain == "liquid":
                payload.pop("scriptpubkey_address", None)
            return payload
        except Exception:
            return None

    def _refresh(self) -> None:
        now = time.time()
        if now < self._cache_until:
            return
        with self._lock:
            if now < self._cache_until:
                return
            try:
                height = int(self.rpc.call("getblockcount"))
                tip_hash = str(self.rpc.call("getblockhash", [height]))
                mempool = tuple(sorted(str(txid) for txid in self.rpc.call("getrawmempool") or []))
            except Exception:
                self._cache_until = now + 2
                return
            tip = (height, tip_hash, mempool)
            if tip == self._tip:
                self._cache_until = now + 2
                return
            txs: dict[str, dict[str, Any]] = {}
            history: dict[str, list[dict[str, Any]]] = {}
            spent: set[tuple[str, int]] = set()
            for block_height in range(height + 1):
                block_hash = str(self.rpc.call("getblockhash", [block_height]))
                block = self.rpc.call("getblock", [block_hash, 2])
                for tx in block.get("tx") or []:
                    if isinstance(tx, dict) and tx.get("txid"):
                        tx["blockheight"] = block_height
                        tx["blockhash"] = block_hash
                        tx["blocktime"] = block.get("time")
                        txs[str(tx["txid"])] = tx
                        self._index_tx(
                            history,
                            spent,
                            tx,
                            block_height,
                            known_txs=txs,
                        )
            for txid in mempool:
                tx = self.raw_tx(txid)
                txs[txid] = tx
                self._index_tx(history, spent, tx, 0, known_txs=txs)
            utxos: dict[str, list[dict[str, Any]]] = {}
            for txid, tx in txs.items():
                for output in tx.get("vout") or []:
                    if not isinstance(output, dict):
                        continue
                    n = output.get("n")
                    if not isinstance(n, int) or (txid, n) in spent:
                        continue
                    script_hex = _script_payload(output).get("scriptpubkey")
                    if not script_hex:
                        continue
                    row = {
                        "tx_hash": txid,
                        "tx_pos": n,
                        "height": tx.get("blockheight") or 0,
                        "txid": txid,
                        "vout": n,
                        "value": _btc_to_sats(output.get("value")) or 0,
                        "status": {
                            "confirmed": bool(tx.get("blockhash")),
                            "block_height": tx.get("blockheight"),
                            "block_hash": tx.get("blockhash"),
                            "block_time": tx.get("blocktime") or tx.get("time"),
                        },
                    }
                    for key in protocol_scripthashes(str(script_hex)):
                        utxos.setdefault(key, []).append(row)
            self._tip = tip
            self._txs = txs
            self._history = history
            self._utxos = utxos
            self._cache_until = now + 2

    def _index_tx(
        self,
        history: dict[str, list[dict[str, Any]]],
        spent: set[tuple[str, int]],
        tx: dict[str, Any],
        height: int,
        *,
        known_txs: dict[str, dict[str, Any]] | None = None,
    ) -> None:
        txid = str(tx.get("txid") or "")
        history_keys: set[str] = set()
        for entry in tx.get("vin") or []:
            if isinstance(entry, dict) and entry.get("txid") and entry.get("vout") is not None:
                try:
                    previous_txid = str(entry["txid"])
                    previous_vout = int(entry["vout"])
                    spent.add((previous_txid, previous_vout))
                    previous = (known_txs or {}).get(previous_txid)
                    previous_outputs = (
                        previous.get("vout") or []
                        if isinstance(previous, dict)
                        else []
                    )
                    prevout = (
                        _script_payload(previous_outputs[previous_vout])
                        if 0 <= previous_vout < len(previous_outputs)
                        and isinstance(previous_outputs[previous_vout], dict)
                        else self._prevout(previous_txid, previous_vout)
                    )
                    script_hex = (prevout or {}).get("scriptpubkey")
                    if script_hex:
                        history_keys.update(protocol_scripthashes(str(script_hex)))
                except Exception:
                    pass
        for output in tx.get("vout") or []:
            if not isinstance(output, dict):
                continue
            script_hex = _script_payload(output).get("scriptpubkey")
            if not script_hex:
                continue
            history_keys.update(protocol_scripthashes(str(script_hex)))
        for key in sorted(history_keys):
            history.setdefault(key, []).append({"tx_hash": txid, "height": height})

    def history(self, scripthash: str, *, mempool: bool | None = None) -> list[dict[str, Any]]:
        self._refresh()
        rows = list(self._history.get(scripthash, []))
        if mempool is True:
            return [row for row in rows if int(row.get("height") or 0) <= 0]
        if mempool is False:
            return [row for row in rows if int(row.get("height") or 0) > 0]
        return rows

    def utxos(self, scripthash: str) -> list[dict[str, Any]]:
        self._refresh()
        return list(self._utxos.get(scripthash, []))

    def stats(self, scripthash: str) -> dict[str, Any]:
        self._refresh()
        history = self._history.get(scripthash, [])
        confirmed = {row["tx_hash"] for row in history if int(row.get("height") or 0) > 0}
        mempool = {row["tx_hash"] for row in history if int(row.get("height") or 0) <= 0}
        utxos = self._utxos.get(scripthash, [])
        confirmed_utxos = [row for row in utxos if int(((row.get("status") or {}).get("block_height")) or 0) > 0]
        mempool_utxos = [row for row in utxos if int(((row.get("status") or {}).get("block_height")) or 0) <= 0]
        return {
            "chain_stats": {
                "funded_txo_count": len(confirmed_utxos),
                "funded_txo_sum": sum(int(row.get("value") or 0) for row in confirmed_utxos),
                "spent_txo_count": max(0, len(confirmed) - len(confirmed_utxos)),
                "spent_txo_sum": 0,
                "tx_count": len(confirmed),
            },
            "mempool_stats": {
                "funded_txo_count": len(mempool_utxos),
                "funded_txo_sum": sum(int(row.get("value") or 0) for row in mempool_utxos),
                "spent_txo_count": 0,
                "spent_txo_sum": 0,
                "tx_count": len(mempool),
            },
        }


def electrum_scripthash(script_hex: str) -> str:
    try:
        payload = bytes.fromhex(script_hex)
    except ValueError:
        payload = b""
    return hashlib.sha256(payload).digest()[::-1].hex()


def esplora_scripthash(script_hex: str) -> str:
    try:
        payload = bytes.fromhex(script_hex)
    except ValueError:
        payload = b""
    return hashlib.sha256(payload).hexdigest()


def protocol_scripthashes(script_hex: str) -> tuple[str, ...]:
    """Return the wire representations used by Electrum and Esplora clients."""

    electrum = electrum_scripthash(script_hex)
    esplora = esplora_scripthash(script_hex)
    return (electrum,) if electrum == esplora else (electrum, esplora)


def electrum_status(history: list[dict[str, Any]]) -> str | None:
    if not history:
        return None
    text = "".join(f"{row['tx_hash']}:{row['height']}:" for row in history)
    return hashlib.sha256(text.encode("ascii")).hexdigest()


def _env_decimal(name: str, default: Decimal) -> Decimal:
    try:
        return Decimal(str(os.environ.get(name) or default))
    except Exception:
        return default


def _price_payload(timestamp: int) -> dict[str, Any]:
    eur = _env_decimal("KASSIBER_REGTEST_BTC_EUR_PRICE", DEFAULT_BTC_EUR_PRICE)
    usd = _env_decimal("KASSIBER_REGTEST_BTC_USD_PRICE", DEFAULT_BTC_USD_PRICE)
    return {
        "time": timestamp,
        "EUR": str(eur),
        "USD": str(usd),
        "prices": [{"time": timestamp, "EUR": str(eur), "USD": str(usd)}],
    }


class ApiHandler(BaseHTTPRequestHandler):
    server_version = "KassiberRegtestBackend/1.0"

    def do_OPTIONS(self) -> None:  # noqa: N802
        self.send_response(204)
        self._cors_headers()
        self.send_header("Access-Control-Max-Age", "3600")
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path.rstrip("/")
        if path in {"", "/api"}:
            payload: dict[str, Any] = {
                "ok": True,
                "service": self.server.service_name,
                "chain": self.server.chain,
                "network": self.server.network,
            }
            if self.server.chain == "bitcoin":
                try:
                    info = self.server.index.rpc.call("getblockchaininfo")
                except Exception as exc:
                    self._error(503, f"bitcoind unavailable: {exc}")
                    return
                payload["blocks"] = info.get("blocks")
                payload["headers"] = info.get("headers")
            self._json(
                payload
            )
            return
        if path == "/api/v1/prices":
            self._json(_price_payload(int(time.time())))
            return
        if path == "/api/v1/historical-price":
            query = parse_qs(parsed.query)
            raw_timestamp = (query.get("timestamp") or [""])[0]
            try:
                timestamp = int(raw_timestamp)
            except (TypeError, ValueError):
                timestamp = int(time.time())
            self._json(_price_payload(timestamp))
            return
        prefix = "/api/tx/"
        if path.startswith(prefix):
            suffix = path[len(prefix) :]
            if suffix.endswith("/hex"):
                txid = suffix.rsplit("/", 1)[0]
                self._tx_hex(txid)
            elif suffix.endswith("/raw"):
                txid = suffix.rsplit("/", 1)[0]
                self._tx_raw(txid)
            elif suffix.endswith("/status"):
                txid = suffix.rsplit("/", 1)[0]
                self._tx_status(txid)
            else:
                self._tx_json(suffix)
            return
        if path.startswith("/api/scripthash/"):
            self._scripthash(path)
            return
        if path.startswith("/api/address/"):
            self._address(path)
            return
        if path == "/api/blocks/tip/height":
            try:
                self._text(str(self.server.index.tip_height()))
            except Exception as exc:
                self._error(503, str(exc))
            return
        if path == "/api/blocks/tip/hash":
            try:
                height = self.server.index.tip_height()
                self._text(str(self.server.index.rpc.call("getblockhash", [height])))
            except Exception as exc:
                self._error(503, str(exc))
            return
        if path == "/api/blocks" or path.startswith("/api/blocks/"):
            try:
                requested = (
                    int(path.rsplit("/", 1)[1])
                    if path != "/api/blocks"
                    else self.server.index.tip_height()
                )
                self._json(self._blocks(requested))
            except Exception as exc:
                self._error(404, str(exc))
            return
        if path.startswith("/api/block-height/"):
            try:
                height = int(path.rsplit("/", 1)[1])
                self._text(str(self.server.index.rpc.call("getblockhash", [height])))
            except Exception as exc:
                self._error(404, str(exc))
            return
        if path.startswith("/api/block/") and path.endswith("/header"):
            try:
                block_hash = path.split("/")[3]
                self._text(str(self.server.index.rpc.call("getblockheader", [block_hash, False])))
            except Exception as exc:
                self._error(404, str(exc))
            return
        self._error(404, "not found")

    def _tx_json(self, txid: str) -> None:
        try:
            self._json(self.server.index.esplora_tx(txid))
        except Exception as exc:
            self._error(404, str(exc))

    def _blocks(self, start_height: int) -> list[dict[str, Any]]:
        output = []
        for height in range(int(start_height), max(-1, int(start_height) - 10), -1):
            block_hash = str(self.server.index.rpc.call("getblockhash", [height]))
            header = self.server.index.rpc.call("getblockheader", [block_hash, True])
            block = self.server.index.rpc.call("getblock", [block_hash, 1])
            output.append(
                {
                    "id": block_hash,
                    "height": height,
                    "version": header.get("version"),
                    "timestamp": header.get("time"),
                    "tx_count": block.get("nTx", len(block.get("tx") or [])),
                    "size": block.get("size"),
                    "weight": block.get("weight"),
                    "merkle_root": header.get("merkleroot"),
                    "previousblockhash": header.get("previousblockhash"),
                    "mediantime": header.get("mediantime"),
                    "nonce": header.get("nonce"),
                    "bits": int(str(header.get("bits") or "0"), 16),
                    "difficulty": header.get("difficulty"),
                }
            )
        return output

    def _tx_hex(self, txid: str) -> None:
        try:
            self._text(self.server.index.raw_hex(txid))
        except Exception as exc:
            self._error(404, str(exc))

    def _tx_raw(self, txid: str) -> None:
        try:
            self._binary(bytes.fromhex(self.server.index.raw_hex(txid)))
        except Exception as exc:
            self._error(404, str(exc))

    def _tx_status(self, txid: str) -> None:
        try:
            self._json(self.server.index.esplora_tx(txid).get("status") or {})
        except Exception as exc:
            self._error(404, str(exc))

    def _scripthash(self, path: str) -> None:
        parts = path.split("/")
        if len(parts) < 4:
            self._error(404, "not found")
            return
        # Both Esplora and Electrum wire representations are direct keys in the
        # shared index. Validate the requested key without changing byte order.
        try:
            bytes.fromhex(parts[3])
        except ValueError:
            self._error(400, "invalid scripthash")
            return
        scripthash = parts[3]
        if len(parts) == 4:
            self._json(self.server.index.stats(scripthash))
        elif path.endswith("/txs"):
            self._json(
                [
                    self.server.index.esplora_tx(row["tx_hash"])
                    for row in self.server.index.history(scripthash)
                ]
            )
        elif path.endswith("/txs/mempool"):
            self._json([self.server.index.esplora_tx(row["tx_hash"]) for row in self.server.index.history(scripthash, mempool=True)])
        elif "/txs/chain" in path:
            self._json([self.server.index.esplora_tx(row["tx_hash"]) for row in self.server.index.history(scripthash, mempool=False)])
        elif path.endswith("/utxo"):
            self._json(self.server.index.utxos(scripthash))
        else:
            self._error(404, "not found")

    def _address(self, path: str) -> None:
        parts = path.split("/")
        if len(parts) < 4:
            self._error(404, "not found")
            return
        try:
            address = unquote(parts[3])
            info = self.server.index.rpc.call("validateaddress", [address])
            if not info.get("isvalid", True) and self.server.chain == "liquid" and address.startswith("el1"):
                address = _replace_blech32_hrp(address, "ert")
                info = self.server.index.rpc.call("validateaddress", [address])
            script_hex = str(info.get("scriptPubKey") or info.get("scriptpubkey") or "")
            if not info.get("isvalid", True) or not script_hex:
                raise ValueError("invalid address")
            scripthash = electrum_scripthash(script_hex)
        except Exception as exc:
            self._error(400, f"invalid address: {exc}")
            return
        if len(parts) == 4:
            self._json(self.server.index.stats(scripthash))
        elif path.endswith("/txs"):
            self._json(
                [
                    self.server.index.esplora_tx(row["tx_hash"])
                    for row in self.server.index.history(scripthash)
                ]
            )
        elif path.endswith("/txs/mempool"):
            self._json(
                [
                    self.server.index.esplora_tx(row["tx_hash"])
                    for row in self.server.index.history(scripthash, mempool=True)
                ]
            )
        elif "/txs/chain" in path:
            self._json(
                [
                    self.server.index.esplora_tx(row["tx_hash"])
                    for row in self.server.index.history(scripthash, mempool=False)
                ]
            )
        elif path.endswith("/utxo"):
            self._json(self.server.index.utxos(scripthash))
        else:
            self._error(404, "not found")

    def _cors_headers(self) -> None:
        origin = os.environ.get("KASSIBER_REGTEST_EXPLORER_CORS_ORIGIN", "*").strip()
        if not origin:
            return
        self.send_header("Access-Control-Allow-Origin", origin)
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, Authorization")

    def _json(self, payload: Any, status: int = 200) -> None:
        body = json.dumps(payload, sort_keys=True).encode("utf-8")
        self.send_response(status)
        self._cors_headers()
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _text(self, payload: str, status: int = 200) -> None:
        body = payload.encode("utf-8")
        self.send_response(status)
        self._cors_headers()
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _binary(self, payload: bytes, status: int = 200) -> None:
        self.send_response(status)
        self._cors_headers()
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _error(self, status: int, message: str) -> None:
        self._json({"ok": False, "error": message}, status=status)

    def log_message(self, fmt: str, *args: Any) -> None:
        print(f"{self.server.service_name}: {fmt % args}", file=sys.stderr)


class ApiServer(ThreadingHTTPServer):
    def __init__(self, addr: tuple[str, int], *, service_name: str, chain: str, network: str, index: BitcoinIndex):
        super().__init__(addr, ApiHandler)
        self.service_name = service_name
        self.chain = chain
        self.network = network
        self.index = index


class ElectrumHandler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        for raw_line in self.rfile:
            try:
                req = json.loads(raw_line.decode("utf-8"))
            except json.JSONDecodeError:
                continue
            response = self._response(req)
            self.wfile.write(json.dumps(response, sort_keys=True).encode("utf-8") + b"\n")

    def _response(self, req: dict[str, Any]) -> dict[str, Any]:
        method = str(req.get("method") or "")
        params = req.get("params") if isinstance(req.get("params"), list) else []
        try:
            result = self._call(method, params)
            return {"jsonrpc": "2.0", "id": req.get("id"), "result": result}
        except Exception as exc:
            return {"jsonrpc": "2.0", "id": req.get("id"), "error": {"code": -1, "message": str(exc)}}

    def _call(self, method: str, params: list[Any]) -> Any:
        if method == "server.version":
            self.server.index.rpc.call("getblockchaininfo")
            return ["Kassiber regtest backend", "1.4"]
        if method == "server.banner":
            return f"{self.server.service_name} ({self.server.chain}/{self.server.network})"
        if method == "blockchain.headers.subscribe":
            height = self.server.index.tip_height()
            return {"height": height, "hex": self.server.index.block_header(height)}
        scripthash = str(params[0]) if params else ""
        if method == "blockchain.scripthash.subscribe":
            return electrum_status(self.server.index.history(scripthash))
        if method == "blockchain.scripthash.get_history":
            return self.server.index.history(scripthash)
        if method == "blockchain.scripthash.listunspent":
            return self.server.index.utxos(scripthash)
        if method == "blockchain.scripthash.get_balance":
            utxos = self.server.index.utxos(scripthash)
            return {"confirmed": sum(int(row.get("value") or 0) for row in utxos), "unconfirmed": 0}
        if method == "blockchain.transaction.get":
            txid = str(params[0]) if params else ""
            return self.server.index.raw_hex(txid)
        if method == "blockchain.block.header":
            height = int(params[0]) if params else 0
            return self.server.index.block_header(height)
        raise RuntimeError(f"unsupported method: {method}")


class ElectrumServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True

    def __init__(self, addr: tuple[str, int], *, service_name: str, chain: str, network: str, index: BitcoinIndex):
        super().__init__(addr, ElectrumHandler)
        self.service_name = service_name
        self.chain = chain
        self.network = network
        self.index = index


def _serve(server: Any) -> threading.Thread:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return thread


ALL_SERVICE_NAMES = {
    "bitcoin-mempool",
    "bitcoin-electrum",
    "liquid-mempool",
    "liquid-electrum",
}


def _enabled_services() -> set[str]:
    raw = os.environ.get("KASSIBER_REGTEST_SERVICES", "all")
    if raw.strip().lower() in {"", "all", "*"}:
        return set(ALL_SERVICE_NAMES)
    aliases = {
        "bitcoin-mempool-regtest": "bitcoin-mempool",
        "bitcoin-electrum-regtest": "bitcoin-electrum",
        "liquid-mempool-regtest": "liquid-mempool",
        "liquid-electrum-regtest": "liquid-electrum",
    }
    names: set[str] = set()
    for item in raw.split(","):
        name = aliases.get(item.strip().lower(), item.strip().lower())
        if not name:
            continue
        if name not in ALL_SERVICE_NAMES:
            raise ValueError(f"unknown KASSIBER_REGTEST_SERVICES entry: {item}")
        names.add(name)
    if not names:
        raise ValueError("KASSIBER_REGTEST_SERVICES did not select any services")
    return names


def main() -> int:
    bitcoin_index = BitcoinIndex(RpcClient())
    liquid_index = BitcoinIndex(
        RpcClient(env_prefix="ELEMENTS", default_url="http://elementsd:7041"),
        chain="liquid",
    )
    enabled = _enabled_services()
    services: list[Any] = []
    if "bitcoin-mempool" in enabled:
        services.append(
            ApiServer(
                ("0.0.0.0", _env_int("BITCOIN_MEMPOOL_PORT", 8080)),
                service_name="bitcoin-mempool-regtest",
                chain="bitcoin",
                network="regtest",
                index=bitcoin_index,
            )
        )
    if "bitcoin-electrum" in enabled:
        services.append(
            ElectrumServer(
                ("0.0.0.0", _env_int("BITCOIN_ELECTRUM_PORT", 50001)),
                service_name="bitcoin-electrum-regtest",
                chain="bitcoin",
                network="regtest",
                index=bitcoin_index,
            )
        )
    if "liquid-mempool" in enabled:
        services.append(
            ApiServer(
                ("0.0.0.0", _env_int("LIQUID_MEMPOOL_PORT", 8081)),
                service_name="liquid-mempool-regtest",
                chain="liquid",
                network="elementsregtest",
                index=liquid_index,
            )
        )
    if "liquid-electrum" in enabled:
        services.append(
            ElectrumServer(
                ("0.0.0.0", _env_int("LIQUID_ELECTRUM_PORT", 50011)),
                service_name="liquid-electrum-regtest",
                chain="liquid",
                network="elementsregtest",
                index=liquid_index,
            )
        )
    for service in services:
        _serve(service)
    print(
        json.dumps(
            {
                "ok": True,
                "services": [
                    {"name": service.service_name, "chain": service.chain, "network": service.network, "port": service.server_address[1]}
                    for service in services
                ],
            },
            sort_keys=True,
        ),
        flush=True,
    )
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    raise SystemExit(main())

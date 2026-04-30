from __future__ import annotations

"""Backend-specific wallet sync helpers used by the CLI sync layer."""

import base64
import hashlib
import json
import socket
import ssl
from collections import defaultdict
from decimal import Decimal
from pathlib import Path
from types import MappingProxyType
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .. import __version__
from ..backends import backend_batch_size, backend_timeout, backend_value
from ..db import APP_NAME
from ..envelope import json_ready
from ..errors import AppError
from ..msat import SATS_PER_BTC, dec
from ..time_utils import UNKNOWN_OCCURRED_AT, timestamp_to_iso
from ..util import normalize_chain_value, normalize_network_value, parse_bool, parse_int
from ..wallet_descriptors import (
    branch_limits,
    decode_liquid_transaction,
    derive_descriptor_target,
    derive_descriptor_targets,
    liquid_asset_code,
    liquid_blinding_secret,
    liquid_plan_can_unblind,
)
from .sync import WalletSyncState, normalize_backend_kind
from .wallets import (
    load_wallet_descriptor_plan_from_config,
    normalize_addresses,
    wallet_policy_asset_id,
)

B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
B58_INDEX = {char: index for index, char in enumerate(B58_ALPHABET)}
BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
BECH32_INDEX = {char: index for index, char in enumerate(BECH32_CHARSET)}


def http_get_json(url, timeout=30):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
        },
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc


def http_get_text(url, timeout=30, accept="text/plain"):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": accept,
            "User-Agent": f"{APP_NAME}/{__version__}",
        },
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return response.read().decode("utf-8")
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc


def http_post_json(url, payload, headers=None, timeout=30):
    request = urlrequest.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Accept": "application/json",
            "Content-Type": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
            **(headers or {}),
        },
        method="POST",
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc


def append_url_path(base_url, extra_path):
    parts = urlparse.urlsplit(base_url)
    path = (parts.path or "").rstrip("/")
    full_path = f"{path}/{extra_path.lstrip('/')}" if extra_path else (path or "/")
    return urlparse.urlunsplit((parts.scheme, parts.netloc, full_path, parts.query, parts.fragment))


def parse_socket_backend_url(url, default_scheme="ssl", default_ports=None):
    default_ports = default_ports or {}
    parsed = urlparse.urlsplit(url if "://" in url else f"{default_scheme}://{url}")
    scheme = (parsed.scheme or default_scheme).lower()
    host = parsed.hostname
    port = parsed.port or default_ports.get(scheme)
    if not host or not port:
        raise AppError(f"Invalid backend socket URL: {url}")
    return scheme, host, port


def sha256d(payload):
    return hashlib.sha256(hashlib.sha256(payload).digest()).digest()


def base58check_decode(value):
    number = 0
    for char in value:
        if char not in B58_INDEX:
            raise AppError(f"Unsupported base58 address: {value}")
        number = number * 58 + B58_INDEX[char]
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    leading_zeros = len(value) - len(value.lstrip("1"))
    payload = (b"\x00" * leading_zeros) + raw
    if len(payload) < 5:
        raise AppError(f"Unsupported base58 address: {value}")
    body, checksum = payload[:-4], payload[-4:]
    if sha256d(body)[:4] != checksum:
        raise AppError(f"Invalid base58 checksum for address: {value}")
    return body


def bech32_polymod(values):
    generators = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = ((checksum & 0x1FFFFFF) << 5) ^ value
        for index, generator in enumerate(generators):
            if (top >> index) & 1:
                checksum ^= generator
    return checksum


def bech32_hrp_expand(hrp):
    return [ord(char) >> 5 for char in hrp] + [0] + [ord(char) & 31 for char in hrp]


def bech32_decode(value):
    if value.lower() != value and value.upper() != value:
        raise AppError(f"Invalid bech32 address casing: {value}")
    normalized = value.lower()
    separator = normalized.rfind("1")
    if separator < 1 or separator + 7 > len(normalized):
        raise AppError(f"Unsupported bech32 address: {value}")
    hrp = normalized[:separator]
    data = []
    for char in normalized[separator + 1 :]:
        if char not in BECH32_INDEX:
            raise AppError(f"Unsupported bech32 address: {value}")
        data.append(BECH32_INDEX[char])
    polymod = bech32_polymod(bech32_hrp_expand(hrp) + data)
    if polymod == 1:
        spec = "bech32"
    elif polymod == 0x2BC830A3:
        spec = "bech32m"
    else:
        raise AppError(f"Invalid bech32 checksum for address: {value}")
    return hrp, data[:-6], spec


def convertbits(data, from_bits, to_bits, pad=True):
    accumulator = 0
    bits = 0
    output = []
    max_value = (1 << to_bits) - 1
    max_accumulator = (1 << (from_bits + to_bits - 1)) - 1
    for value in data:
        if value < 0 or value >> from_bits:
            raise AppError("Invalid bit group in address encoding")
        accumulator = ((accumulator << from_bits) | value) & max_accumulator
        bits += from_bits
        while bits >= to_bits:
            bits -= to_bits
            output.append((accumulator >> bits) & max_value)
    if pad:
        if bits:
            output.append((accumulator << (to_bits - bits)) & max_value)
    elif bits >= from_bits or ((accumulator << (to_bits - bits)) & max_value):
        raise AppError("Invalid address padding")
    return output


def address_to_scriptpubkey(address):
    if address.lower().startswith(("bc1", "tb1", "bcrt1")):
        _, data, spec = bech32_decode(address)
        if not data:
            raise AppError(f"Invalid segwit address: {address}")
        version = data[0]
        if version > 16:
            raise AppError(f"Unsupported segwit witness version for address: {address}")
        program = bytes(convertbits(data[1:], 5, 8, pad=False))
        if len(program) < 2 or len(program) > 40:
            raise AppError(f"Invalid segwit program length for address: {address}")
        if version == 0 and spec != "bech32":
            raise AppError(f"Invalid bech32 checksum type for address: {address}")
        if version > 0 and spec != "bech32m":
            raise AppError(f"Invalid bech32m checksum type for address: {address}")
        opcode = 0 if version == 0 else 0x50 + version
        return bytes([opcode, len(program)]) + program
    payload = base58check_decode(address)
    version = payload[0]
    hash160 = payload[1:]
    if len(hash160) != 20:
        raise AppError(f"Unsupported base58 payload length for address: {address}")
    if version in {0x00, 0x6F}:
        return bytes.fromhex("76a914") + hash160 + bytes.fromhex("88ac")
    if version in {0x05, 0xC4}:
        return bytes.fromhex("a914") + hash160 + bytes.fromhex("87")
    raise AppError(f"Unsupported address version for address: {address}")


def output_addresses(vout):
    script = vout.get("scriptPubKey") or {}
    values = []
    if script.get("address"):
        values.append(script["address"])
    values.extend(script.get("addresses") or [])
    return normalize_addresses(values)


def sanitize_wallet_segment(value):
    text = str(value).strip().lower()
    cleaned = []
    for char in text:
        if char.isalnum() or char in {"-", "_", "."}:
            cleaned.append(char)
        else:
            cleaned.append("-")
    sanitized = "".join(cleaned).strip("-")
    return sanitized or APP_NAME


class ElectrumClient:
    def __init__(self, backend):
        self.backend = backend
        self.socket = None
        self.reader = None
        self.request_id = 0

    def __enter__(self):
        scheme, host, port = parse_socket_backend_url(
            self.backend["url"],
            default_scheme="ssl",
            default_ports={"ssl": 50002, "tcp": 50001},
        )
        raw_socket = socket.create_connection((host, port), timeout=backend_timeout(self.backend))
        if scheme in {"ssl", "tls"}:
            context = ssl.create_default_context()
            if parse_bool(backend_value(self.backend, "insecure"), default=False):
                context.check_hostname = False
                context.verify_mode = ssl.CERT_NONE
            raw_socket = context.wrap_socket(raw_socket, server_hostname=host)
        elif scheme != "tcp":
            raise AppError(f"Unsupported Electrum transport '{scheme}'")
        self.socket = raw_socket
        self.reader = raw_socket.makefile("r", encoding="utf-8", newline="\n")
        return self

    def __exit__(self, exc_type, exc, tb):
        if self.reader is not None:
            self.reader.close()
        if self.socket is not None:
            self.socket.close()
        self.reader = None
        self.socket = None
        return False

    def call(self, method, params=None):
        if self.socket is None or self.reader is None:
            raise AppError("Electrum client is not connected")
        self.request_id += 1
        payload = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": self.request_id,
                "method": method,
                "params": params or [],
            }
        ).encode("utf-8") + b"\n"
        self.socket.sendall(payload)
        while True:
            line = self.reader.readline()
            if not line:
                raise AppError(f"Electrum backend '{self.backend['name']}' closed the connection")
            message = json.loads(line)
            if message.get("id") != self.request_id:
                continue
            if message.get("error"):
                error = message["error"]
                if isinstance(error, dict):
                    detail = f"({error.get('code', 'unknown')}): {error.get('message', error)}"
                else:
                    detail = str(error)
                raise AppError(f"Electrum call {method} failed {detail}")
            return message.get("result")

    def batch_call(self, requests):
        if self.socket is None or self.reader is None:
            raise AppError("Electrum client is not connected")
        if not requests:
            return []
        payload_lines = []
        pending = {}
        for index, (method, params) in enumerate(requests):
            self.request_id += 1
            pending[self.request_id] = (index, method)
            payload_lines.append(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": self.request_id,
                        "method": method,
                        "params": params or [],
                    }
                )
            )
        self.socket.sendall(("\n".join(payload_lines) + "\n").encode("utf-8"))
        results = [None] * len(requests)
        remaining = len(requests)
        while remaining:
            line = self.reader.readline()
            if not line:
                raise AppError(f"Electrum backend '{self.backend['name']}' closed the connection")
            message = json.loads(line)
            response_id = message.get("id")
            if response_id not in pending:
                continue
            index, method = pending.pop(response_id)
            if message.get("error"):
                error = message["error"]
                if isinstance(error, dict):
                    detail = f"({error.get('code', 'unknown')}): {error.get('message', error)}"
                else:
                    detail = str(error)
                raise AppError(f"Electrum call {method} failed {detail}")
            results[index] = message.get("result")
            remaining -= 1
        return results


def batched(items, batch_size):
    for start in range(0, len(items), batch_size):
        yield items[start : start + batch_size]


def electrum_call_many(client, requests, batch_size):
    if not requests:
        return []
    normalized_batch_size = max(1, int(batch_size))
    batch_call = getattr(client, "batch_call", None)
    results = []
    for chunk in batched(requests, normalized_batch_size):
        if callable(batch_call):
            results.extend(batch_call(chunk))
            continue
        for method, params in chunk:
            results.append(client.call(method, params))
    return results


def sync_target_from_address(address, chain, network, address_index):
    return {
        "chain": chain,
        "network": network,
        "branch_index": 0,
        "branch_label": "address",
        "address_index": address_index,
        "address": address,
        "unconfidential_address": None,
        "script_pubkey": address_to_scriptpubkey(address).hex(),
    }


def sync_target_from_derived(target):
    return {
        "chain": target.chain,
        "network": target.network,
        "branch_index": target.branch_index,
        "branch_label": target.branch_label,
        "address_index": target.address_index,
        "address": target.address,
        "unconfidential_address": target.unconfidential_address,
        "script_pubkey": target.script_pubkey,
        "derivation_path": target.derivation_path,
        "derivation_paths": list(target.derivation_paths),
        "key_origins": list(target.key_origins),
    }


def scriptpubkey_scripthash(script_pubkey_hex):
    return hashlib.sha256(bytes.fromhex(script_pubkey_hex)).digest()[::-1].hex()


def validate_backend_for_wallet(backend, chain, network, has_descriptor=False):
    kind = normalize_backend_kind(backend["kind"])
    backend_chain = backend_value(backend, "chain")
    if backend_chain:
        expected_chain = normalize_chain_value(backend_chain)
        if expected_chain != chain:
            raise AppError(
                f"Backend '{backend['name']}' is configured for {expected_chain}, but wallet sync requires {chain}"
            )
    backend_network = backend_value(backend, "network")
    if backend_network:
        expected_network = normalize_network_value(chain, backend_network)
        if expected_network != network:
            raise AppError(
                f"Backend '{backend['name']}' is configured for {expected_network}, but wallet sync requires {network}"
            )
    if chain == "liquid" and kind not in {"esplora", "electrum"}:
        raise AppError("Liquid live sync currently requires an Esplora-compatible or Electrum backend")
    if has_descriptor and kind == "bitcoinrpc":
        raise AppError("Descriptor-backed live sync is not implemented for bitcoinrpc yet; use Esplora or Electrum")
    if chain != "bitcoin" and kind == "bitcoinrpc":
        raise AppError(f"Backend kind '{kind}' does not support {chain} wallets")
    return kind


def scan_descriptor_targets(plan, target_used=None, target_used_batch=None, scan_batch_size=100):
    limits = branch_limits(plan)
    targets = []
    for branch in plan.branches:
        branch_gap_limit = limits.get(branch.branch_index, plan.gap_limit)
        if branch_gap_limit <= 1:
            targets.append(sync_target_from_derived(derive_descriptor_target(plan, branch.branch_index, 0)))
            continue
        consecutive_unused = 0
        address_index = 0
        while consecutive_unused < branch_gap_limit:
            if target_used_batch is not None:
                batch_targets = [
                    sync_target_from_derived(target)
                    for target in derive_descriptor_targets(
                        plan,
                        branch_index=branch.branch_index,
                        start=address_index,
                        end=address_index + scan_batch_size,
                    )
                ]
                if not batch_targets:
                    break
                used_batch = list(target_used_batch(batch_targets))
                if len(used_batch) != len(batch_targets):
                    raise AppError("Descriptor discovery returned an unexpected number of usage checks")
                for target, is_used in zip(batch_targets, used_batch):
                    targets.append(target)
                    if is_used:
                        consecutive_unused = 0
                    else:
                        consecutive_unused += 1
                    address_index += 1
                    if consecutive_unused >= branch_gap_limit:
                        break
                continue
            target = sync_target_from_derived(derive_descriptor_target(plan, branch.branch_index, address_index))
            targets.append(target)
            if target_used and target_used(target):
                consecutive_unused = 0
            else:
                consecutive_unused += 1
            address_index += 1
    return targets


def esplora_scripthash_has_history(base_url, script_pubkey_hex, timeout=30):
    resource = append_url_path(base_url, f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}")
    payload = http_get_json(resource, timeout=timeout)
    chain_stats = payload.get("chain_stats") or {}
    mempool_stats = payload.get("mempool_stats") or {}
    return int(chain_stats.get("tx_count") or 0) + int(mempool_stats.get("tx_count") or 0) > 0


def discover_descriptor_targets(backend, plan, kind):
    timeout = backend_timeout(backend)
    if kind == "esplora":
        return {
            "targets": scan_descriptor_targets(
                plan,
                lambda target: esplora_scripthash_has_history(
                    backend["url"],
                    target["script_pubkey"],
                    timeout=timeout,
                ),
            ),
            "history_cache": {},
        }
    if kind == "electrum":
        electrum_batch_size = backend_batch_size(backend)
        with ElectrumClient(backend) as client:
            history_cache = {}

            def target_used_batch(targets):
                scripthashes = [scriptpubkey_scripthash(target["script_pubkey"]) for target in targets]
                histories = electrum_call_many(
                    client,
                    [("blockchain.scripthash.get_history", [scripthash]) for scripthash in scripthashes],
                    batch_size=electrum_batch_size,
                )
                for scripthash, history in zip(scripthashes, histories):
                    history_cache[scripthash] = history or []
                return [bool(history) for history in histories]

            return {
                "targets": scan_descriptor_targets(
                    plan,
                    target_used_batch=target_used_batch,
                    scan_batch_size=electrum_batch_size,
                ),
                "history_cache": history_cache,
            }
    raise AppError(f"Descriptor-backed sync is not implemented for backend kind '{kind}'")


def resolve_wallet_sync_targets(backend, wallet):
    config = json.loads(wallet["config_json"] or "{}")
    descriptor_plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    history_cache = {}
    if descriptor_plan:
        chain = descriptor_plan.chain
        network = descriptor_plan.network
        if chain == "liquid" and not liquid_plan_can_unblind(descriptor_plan):
            raise AppError("Liquid descriptor wallets require private blinding keys for full sync and fee accounting")
        if chain == "liquid" and not config.get("backend"):
            raise AppError("Liquid wallets must name a backend explicitly; no public Liquid default is built in")
        kind = validate_backend_for_wallet(backend, chain, network, has_descriptor=True)
        discovery = discover_descriptor_targets(backend, descriptor_plan, kind)
        targets = discovery["targets"]
        history_cache = discovery.get("history_cache") or {}
    else:
        addresses = normalize_addresses(config.get("addresses"))
        if not addresses:
            return WalletSyncState(
                chain="",
                network="",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[],
                tracked_scripts={},
                history_cache={},
            )
        chain = normalize_chain_value(config.get("chain"))
        network = normalize_network_value(chain, config.get("network"))
        if chain == "liquid":
            raise AppError("Liquid live sync currently requires descriptor-backed wallets so outputs can be unblinded locally")
        validate_backend_for_wallet(backend, chain, network, has_descriptor=False)
        targets = [sync_target_from_address(address, chain, network, index) for index, address in enumerate(addresses)]
    tracked_scripts = {target["script_pubkey"]: target for target in targets}
    return WalletSyncState(
        chain=chain,
        network=network,
        descriptor_plan=descriptor_plan,
        policy_asset_id=wallet_policy_asset_id(config, chain, network),
        targets=targets,
        tracked_scripts=tracked_scripts,
        history_cache=history_cache,
    )


def fetch_esplora_history(base_url, resource_path, max_pages=None, timeout=30):
    transactions = []
    seen_txids = set()
    last_seen = None
    page_count = 0
    while True:
        if max_pages is not None and page_count >= max_pages:
            break
        chain_url = (
            append_url_path(base_url, f"{resource_path}/txs/chain/{last_seen}")
            if last_seen
            else append_url_path(base_url, f"{resource_path}/txs/chain")
        )
        page = http_get_json(chain_url, timeout=timeout)
        if not page:
            break
        for tx in page:
            txid = tx.get("txid")
            if txid and txid not in seen_txids:
                seen_txids.add(txid)
                transactions.append(tx)
        last_seen = page[-1]["txid"]
        page_count += 1
        if len(page) < 25:
            break
    mempool_url = append_url_path(base_url, f"{resource_path}/txs/mempool")
    for tx in http_get_json(mempool_url, timeout=timeout):
        txid = tx.get("txid")
        if txid and txid not in seen_txids:
            seen_txids.add(txid)
            transactions.append(tx)
    return transactions


def fetch_esplora_scripthash_transactions(
    base_url,
    script_pubkey_hex,
    max_pages=None,
    timeout=30,
):
    return fetch_esplora_history(
        base_url,
        f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}",
        max_pages=max_pages,
        timeout=timeout,
    )


def sats_to_btc(value):
    return dec(value) / SATS_PER_BTC


def record_from_bitcoin_esplora_tx(tx, tracked_scripts, backend_name):
    received_sats = sum(
        dec(vout.get("value", 0))
        for vout in tx.get("vout", [])
        if vout.get("scriptpubkey") in tracked_scripts
    )
    sent_sats = Decimal("0")
    for vin in tx.get("vin", []):
        prevout = vin.get("prevout") or {}
        if prevout.get("scriptpubkey") in tracked_scripts:
            sent_sats += dec(prevout.get("value", 0))
    if received_sats == 0 and sent_sats == 0:
        return None
    fee_sats = dec(tx.get("fee"), "0")
    if received_sats > sent_sats:
        direction = "inbound"
        amount = sats_to_btc(received_sats - sent_sats)
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out_sats = sent_sats - received_sats
        amount_sats = gross_out_sats - fee_sats
        if amount_sats < 0:
            amount_sats = Decimal("0")
        amount = sats_to_btc(amount_sats)
        fee = sats_to_btc(fee_sats)
        kind = "withdrawal" if amount > 0 else "fee"
    block_time = (tx.get("status") or {}).get("block_time")
    occurred_at = timestamp_to_iso(block_time)
    confirmed_at = timestamp_to_iso(block_time, default=None)
    return {
        "txid": tx.get("txid"),
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(tx, sort_keys=True),
    }


def liquid_asset_id_from_bytes(asset_bytes):
    if not isinstance(asset_bytes, (bytes, bytearray)) or len(asset_bytes) != 32:
        raise AppError("Unsupported Liquid asset encoding while decoding transaction")
    return bytes(reversed(bytes(asset_bytes))).hex()


def liquid_output_amount_asset_id(output, plan, target=None):
    try:
        if getattr(output, "is_blinded", False):
            if target is None:
                raise AppError("Unable to unblind Liquid output without wallet descriptor context")
            secret, _ = liquid_blinding_secret(plan, target["branch_index"], target["address_index"])
            value, asset_bytes, *_ = output.unblind(secret)
        else:
            value = output.value
            asset_bytes = output.asset
    except AppError:
        raise
    except Exception as exc:
        label = target["address"] if target and target.get("address") else target["script_pubkey"] if target else "fee output"
        raise AppError(f"Failed to decode Liquid output for {label}") from exc
    if not isinstance(value, int):
        raise AppError("Unsupported confidential Liquid value encoding while decoding transaction")
    return value, liquid_asset_id_from_bytes(asset_bytes)


def liquid_input_txid(vin):
    txid = getattr(vin, "txid", None)
    if isinstance(txid, str):
        return txid.lower()
    if isinstance(txid, (bytes, bytearray)):
        return bytes(txid).hex()
    raise AppError("Unsupported Liquid input txid encoding while decoding transaction")


def record_components_from_liquid_tx(
    txid,
    occurred_at,
    tx,
    descriptor_plan,
    tracked_scripts,
    backend_name,
    policy_asset_id,
    prev_tx_lookup,
    raw_json_context=None,
    confirmed_at=None,
):
    net_sats = defaultdict(int)
    fee_sats = defaultdict(int)
    for output in tx.vout:
        script_hex = output.script_pubkey.data.hex()
        if script_hex == "":
            value_sats, asset_id = liquid_output_amount_asset_id(output, descriptor_plan, target=None)
            fee_sats[asset_id] += value_sats
            continue
        target = tracked_scripts.get(script_hex)
        if not target:
            continue
        value_sats, asset_id = liquid_output_amount_asset_id(output, descriptor_plan, target=target)
        net_sats[asset_id] += value_sats
    for vin in tx.vin:
        prev_txid = liquid_input_txid(vin)
        prev_vout = getattr(vin, "vout", None)
        if prev_vout is None:
            continue
        prev_tx = prev_tx_lookup(prev_txid)
        if prev_vout >= len(prev_tx.vout):
            raise AppError(f"Liquid prevout index {prev_vout} is out of range for transaction {prev_txid}")
        prev_output = prev_tx.vout[prev_vout]
        script_hex = prev_output.script_pubkey.data.hex()
        target = tracked_scripts.get(script_hex)
        if not target:
            continue
        value_sats, asset_id = liquid_output_amount_asset_id(prev_output, descriptor_plan, target=target)
        net_sats[asset_id] -= value_sats
    records = []
    all_assets = sorted(set(net_sats) | set(fee_sats))
    for asset_id in all_assets:
        asset_code = liquid_asset_code(asset_id, policy_asset_id)
        net_value = dec(net_sats.get(asset_id, 0), default="0")
        fee_value = dec(fee_sats.get(asset_id, 0), default="0")
        if net_value == 0 and fee_value == 0:
            continue
        if net_value > 0:
            direction = "inbound"
            amount = sats_to_btc(net_value)
            fee = Decimal("0")
            kind = "deposit"
        else:
            direction = "outbound"
            gross_out_sats = abs(net_value)
            amount_sats = gross_out_sats - fee_value
            if amount_sats < 0:
                amount_sats = Decimal("0")
            amount = sats_to_btc(amount_sats)
            fee = sats_to_btc(fee_value)
            kind = "withdrawal" if amount > 0 else "fee"
        records.append(
            {
                "txid": txid,
                "occurred_at": occurred_at,
                "confirmed_at": (
                    confirmed_at
                    if confirmed_at is not None
                    else (None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at)
                ),
                "direction": direction,
                "asset": asset_code,
                "amount": amount,
                "fee": fee,
                "fiat_rate": None,
                "fiat_value": None,
                "kind": kind,
                "description": f"Synced from {backend_name}",
                "counterparty": None,
                "raw_json": json.dumps(
                    json_ready(
                        {
                            **(raw_json_context or {}),
                            "txid": txid,
                            "component": {
                                "asset_id": asset_id,
                                "asset": asset_code,
                                "net_sats": int(net_value),
                                "fee_sats": int(fee_value),
                            },
                        }
                    ),
                    sort_keys=True,
                ),
            }
        )
    return records


def esplora_records_for_wallet(backend, sync_state: WalletSyncState):
    max_pages = parse_int(backend_value(backend, "maxpages"), default=0) or None
    timeout = backend_timeout(backend)
    transactions_by_txid = {}
    for target in sync_state.targets:
        for tx in fetch_esplora_scripthash_transactions(
            backend["url"],
            target["script_pubkey"],
            max_pages=max_pages,
            timeout=timeout,
        ):
            transactions_by_txid[tx["txid"]] = tx
    records = []
    raw_tx_cache = {}

    def liquid_tx_lookup(txid):
        if txid not in raw_tx_cache:
            raw_hex = http_get_text(
                append_url_path(backend["url"], f"tx/{txid}/hex"),
                timeout=backend_timeout(backend),
            ).strip()
            raw_tx_cache[txid] = {
                "raw_hex": raw_hex,
                "decoded": decode_liquid_transaction(raw_hex),
            }
        return raw_tx_cache[txid]["decoded"]

    for tx in sorted(
        transactions_by_txid.values(),
        key=lambda item: (((item.get("status") or {}).get("block_time") or 0), item.get("txid", "")),
    ):
        if sync_state.chain == "liquid":
            raw_hex = http_get_text(
                append_url_path(backend["url"], f"tx/{tx['txid']}/hex"),
                timeout=backend_timeout(backend),
            ).strip()
            decoded_tx = decode_liquid_transaction(raw_hex)
            records.extend(
                record_components_from_liquid_tx(
                    tx["txid"],
                    timestamp_to_iso((tx.get("status") or {}).get("block_time")),
                    decoded_tx,
                    sync_state.descriptor_plan,
                    sync_state.tracked_scripts,
                    backend["name"],
                    sync_state.policy_asset_id,
                    liquid_tx_lookup,
                    {"tx": tx, "raw_hex": raw_hex},
                    confirmed_at=timestamp_to_iso((tx.get("status") or {}).get("block_time"), default=None),
                )
            )
        else:
            normalized = record_from_bitcoin_esplora_tx(tx, sync_state.tracked_scripts, backend["name"])
            if normalized:
                records.append(normalized)
    return records


def esplora_sync_adapter(backend, wallet, sync_state):
    del wallet
    return esplora_records_for_wallet(backend, sync_state), {}


def bitcoinrpc_auth_headers(backend):
    cookie_path = backend_value(backend, "cookiefile", "cookie_file")
    if cookie_path:
        token = Path(cookie_path).expanduser().read_text(encoding="utf-8").strip()
    else:
        username = backend_value(backend, "username", "rpcuser", "rpc_user")
        password = backend_value(backend, "password", "rpcpassword", "rpc_password")
        if not username or password is None:
            raise AppError(
                f"Bitcoin Core backend '{backend['name']}' requires USERNAME/PASSWORD or COOKIEFILE configuration"
            )
        token = f"{username}:{password}"
    encoded = base64.b64encode(token.encode("utf-8")).decode("ascii")
    return {"Authorization": f"Basic {encoded}"}


def bitcoinrpc_url(backend, wallet_name=None):
    if wallet_name:
        return append_url_path(backend["url"], f"wallet/{urlparse.quote(wallet_name, safe='')}")
    return backend["url"]


def bitcoinrpc_call(backend, method, params=None, wallet_name=None):
    payload = {
        "jsonrpc": "1.0",
        "id": f"{APP_NAME}-{method}",
        "method": method,
        "params": [] if params is None else params,
    }
    response = http_post_json(
        bitcoinrpc_url(backend, wallet_name=wallet_name),
        payload,
        headers=bitcoinrpc_auth_headers(backend),
        timeout=backend_timeout(backend),
    )
    if response.get("error"):
        error = response["error"]
        raise AppError(
            f"Bitcoin Core RPC {method} failed"
            f" ({error.get('code', 'unknown')}): {error.get('message', error)}"
        )
    return response.get("result")


def bitcoinrpc_wallet_name(backend, wallet):
    explicit_name = backend_value(backend, "wallet")
    if explicit_name:
        return explicit_name
    prefix = backend_value(backend, "walletprefix", "wallet_prefix") or APP_NAME
    return f"{sanitize_wallet_segment(prefix)}-{sanitize_wallet_segment(wallet['id'])}"


def bitcoinrpc_ensure_watchonly_wallet(backend, wallet):
    wallet_name = bitcoinrpc_wallet_name(backend, wallet)
    loaded_wallets = bitcoinrpc_call(backend, "listwallets")
    if wallet_name in loaded_wallets:
        return wallet_name
    try:
        bitcoinrpc_call(backend, "loadwallet", [wallet_name, True])
        return wallet_name
    except AppError as load_error:
        create_attempts = [
            [wallet_name, True, True, "", False, True, True],
            [wallet_name, True, True, "", False, True],
            [wallet_name, True, True],
        ]
        for params in create_attempts:
            try:
                bitcoinrpc_call(backend, "createwallet", params)
                return wallet_name
            except AppError:
                continue
        raise AppError(
            f"Unable to load or create Bitcoin Core wallet '{wallet_name}' for backend '{backend['name']}'. "
            f"Last load error: {load_error}"
        ) from load_error


def bitcoinrpc_import_addresses(backend, wallet_name, wallet, addresses):
    label = f"{APP_NAME}:{wallet['id']}"
    missing_addresses = []
    descriptors = []
    for address in addresses:
        info = bitcoinrpc_call(backend, "getaddressinfo", [address], wallet_name=wallet_name)
        if info.get("iswatchonly") or info.get("ismine"):
            continue
        descriptor = bitcoinrpc_call(backend, "getdescriptorinfo", [f"addr({address})"])
        descriptors.append({"desc": descriptor["descriptor"], "timestamp": 0, "label": label})
        missing_addresses.append(address)
    if not missing_addresses:
        return 0
    try:
        results = bitcoinrpc_call(backend, "importdescriptors", [descriptors], wallet_name=wallet_name)
        failures = [result for result in results if not result.get("success")]
        if failures:
            raise AppError(f"descriptor import failed: {failures[0]}")
    except AppError:
        requests = [
            {
                "scriptPubKey": {"address": address},
                "timestamp": 0,
                "watchonly": True,
                "label": label,
            }
            for address in missing_addresses
        ]
        options = {"rescan": True}
        results = bitcoinrpc_call(backend, "importmulti", [requests, options], wallet_name=wallet_name)
        failures = [result for result in results if not result.get("success")]
        if failures:
            raise AppError(f"address import failed: {failures[0]}")
    return len(missing_addresses)


def fetch_bitcoinrpc_wallet_transactions(backend, wallet_name, page_size=1000):
    transactions = []
    skip = 0
    while True:
        page = bitcoinrpc_call(backend, "listtransactions", ["*", page_size, skip, True], wallet_name=wallet_name)
        if not page:
            break
        transactions.extend(page)
        if len(page) < page_size:
            break
        skip += page_size
    return transactions


def record_from_bitcoinrpc_details(txid, details, backend_name):
    amount_total = Decimal("0")
    fee_total = Decimal("0")
    occurred_at = UNKNOWN_OCCURRED_AT
    confirmed_at = None
    for detail in details:
        category = str(detail.get("category") or "").lower()
        if category in {"orphan", "immature"}:
            continue
        amount_total += dec(detail.get("amount"), "0")
        fee_total += abs(dec(detail.get("fee"), "0"))
        if detail.get("blocktime") not in (None, "", 0, "0"):
            confirmed_at = timestamp_to_iso(detail.get("blocktime"), default=None)
        occurred_at = timestamp_to_iso(detail.get("blocktime") or detail.get("time"), default=occurred_at)
    if amount_total == 0 and fee_total == 0:
        return None
    if amount_total > 0:
        direction = "inbound"
        amount = amount_total
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out = abs(amount_total)
        amount = gross_out - fee_total
        if amount < 0:
            amount = Decimal("0")
        fee = fee_total
        kind = "withdrawal" if amount > 0 else "fee"
    return {
        "txid": txid,
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(details, sort_keys=True),
    }


def bitcoinrpc_records_for_wallet(backend, wallet, addresses):
    wallet_name = bitcoinrpc_ensure_watchonly_wallet(backend, wallet)
    imported_count = bitcoinrpc_import_addresses(backend, wallet_name, wallet, addresses)
    details = fetch_bitcoinrpc_wallet_transactions(backend, wallet_name)
    grouped = defaultdict(list)
    for detail in details:
        txid = detail.get("txid")
        if txid:
            grouped[txid].append(detail)
    records = []
    for txid, tx_details in sorted(
        grouped.items(),
        key=lambda item: (
            max(detail.get("blocktime") or detail.get("time") or 0 for detail in item[1]),
            item[0],
        ),
    ):
        normalized = record_from_bitcoinrpc_details(txid, tx_details, backend["name"])
        if normalized:
            records.append(normalized)
    return records, {"core_wallet": wallet_name, "imported_addresses": imported_count}


def bitcoinrpc_sync_adapter(backend, wallet, sync_state: WalletSyncState):
    return bitcoinrpc_records_for_wallet(
        backend,
        wallet,
        [target["address"] for target in sync_state.targets if target.get("address")],
    )


def read_varint(payload, offset):
    prefix = payload[offset]
    offset += 1
    if prefix < 0xFD:
        return prefix, offset
    if prefix == 0xFD:
        return int.from_bytes(payload[offset : offset + 2], "little"), offset + 2
    if prefix == 0xFE:
        return int.from_bytes(payload[offset : offset + 4], "little"), offset + 4
    return int.from_bytes(payload[offset : offset + 8], "little"), offset + 8


def decode_raw_transaction(raw_hex):
    payload = bytes.fromhex(raw_hex)
    offset = 0
    version = int.from_bytes(payload[offset : offset + 4], "little")
    offset += 4
    has_witness = len(payload) > offset + 1 and payload[offset] == 0 and payload[offset + 1] != 0
    if has_witness:
        offset += 2
    input_count, offset = read_varint(payload, offset)
    vin = []
    for _ in range(input_count):
        prev_txid = payload[offset : offset + 32][::-1].hex()
        offset += 32
        prev_vout = int.from_bytes(payload[offset : offset + 4], "little")
        offset += 4
        script_length, offset = read_varint(payload, offset)
        offset += script_length
        sequence = int.from_bytes(payload[offset : offset + 4], "little")
        offset += 4
        vin.append({"txid": prev_txid, "vout": prev_vout, "sequence": sequence})
    output_count, offset = read_varint(payload, offset)
    vout = []
    total_output_sats = Decimal("0")
    for index in range(output_count):
        value_sats = int.from_bytes(payload[offset : offset + 8], "little")
        offset += 8
        script_length, offset = read_varint(payload, offset)
        script = payload[offset : offset + script_length]
        offset += script_length
        value_decimal = dec(value_sats)
        total_output_sats += value_decimal
        vout.append(
            {
                "n": index,
                "value_sats": value_decimal,
                "value": sats_to_btc(value_decimal),
                "script_hex": script.hex(),
            }
        )
    if has_witness:
        for _ in range(input_count):
            witness_count, offset = read_varint(payload, offset)
            for _ in range(witness_count):
                item_length, offset = read_varint(payload, offset)
                offset += item_length
    locktime = int.from_bytes(payload[offset : offset + 4], "little")
    return {
        "version": version,
        "locktime": locktime,
        "vin": vin,
        "vout": vout,
        "total_output_sats": total_output_sats,
        "raw_hex": raw_hex,
    }


def block_header_timestamp(header_hex):
    header = bytes.fromhex(header_hex)
    return int.from_bytes(header[68:72], "little")


def electrum_output_at_index(tx, index):
    outputs = tx.get("vout") or []
    if index is None or index < 0 or index >= len(outputs):
        return None
    return outputs[index]


def record_from_electrum_tx(txid, tx, height, tracked_scripts, backend_name, tx_lookup):
    received_sats = Decimal("0")
    sent_sats = Decimal("0")
    total_input_sats = Decimal("0")
    total_output_sats = tx["total_output_sats"]
    for vout in tx.get("vout", []):
        if vout.get("script_hex") in tracked_scripts:
            received_sats += vout["value_sats"]
    for vin in tx.get("vin", []):
        prev_txid = vin.get("txid")
        prev_index = vin.get("vout")
        if prev_txid is None or prev_index is None:
            continue
        prev_tx = tx_lookup(prev_txid)
        prevout = electrum_output_at_index(prev_tx, prev_index)
        if not prevout:
            continue
        total_input_sats += prevout["value_sats"]
        if prevout.get("script_hex") in tracked_scripts:
            sent_sats += prevout["value_sats"]
    if received_sats == 0 and sent_sats == 0:
        return None
    fee_sats = total_input_sats - total_output_sats if total_input_sats > 0 else Decimal("0")
    if fee_sats < 0:
        fee_sats = Decimal("0")
    if received_sats > sent_sats:
        direction = "inbound"
        amount = sats_to_btc(received_sats - sent_sats)
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out_sats = sent_sats - received_sats
        amount_sats = gross_out_sats - fee_sats
        if amount_sats < 0:
            amount_sats = Decimal("0")
        amount = sats_to_btc(amount_sats)
        fee = sats_to_btc(fee_sats)
        kind = "withdrawal" if amount > 0 else "fee"
    occurred_at = timestamp_to_iso(height)
    confirmed_at = None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at
    return {
        "txid": txid,
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(json_ready(tx), sort_keys=True),
    }


def electrum_records_for_wallet(backend, sync_state: WalletSyncState):
    transactions = {}
    header_timestamps = {}
    records = []
    batch_size = backend_batch_size(backend)
    tracked_scripts = (
        set(sync_state.tracked_scripts)
        if sync_state.chain == "bitcoin"
        else sync_state.tracked_scripts
    )
    with ElectrumClient(backend) as client:
        histories = []
        history_cache = sync_state.history_cache or {}
        uncached_scripthashes = []
        for target in sync_state.targets:
            scripthash = scriptpubkey_scripthash(target["script_pubkey"])
            cached_history = history_cache.get(scripthash)
            if cached_history is not None:
                histories.extend(cached_history)
                continue
            uncached_scripthashes.append(scripthash)
        if uncached_scripthashes:
            uncached_histories = electrum_call_many(
                client,
                [("blockchain.scripthash.get_history", [scripthash]) for scripthash in uncached_scripthashes],
                batch_size=batch_size,
            )
            for scripthash, history in zip(uncached_scripthashes, uncached_histories):
                normalized_history = history or []
                history_cache[scripthash] = normalized_history
                histories.extend(normalized_history)

        def lookup(txid):
            if txid not in transactions:
                raw_tx = client.call("blockchain.transaction.get", [txid])
                if sync_state.chain == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
            return transactions[txid]

        def height_to_timestamp(height):
            if height in (None, 0) or int(height) <= 0:
                return None
            normalized_height = int(height)
            if normalized_height not in header_timestamps:
                header_hex = client.call("blockchain.block.header", [normalized_height])
                header_timestamps[normalized_height] = block_header_timestamp(header_hex)
            return header_timestamps[normalized_height]

        txids = {}
        for history in histories:
            txids[history["tx_hash"]] = history
        ordered_histories = sorted(txids.items(), key=lambda item: (item[1].get("height", 0), item[0]))
        ordered_txids = [txid for txid, _ in ordered_histories]
        if ordered_txids:
            raw_transactions = electrum_call_many(
                client,
                [("blockchain.transaction.get", [txid]) for txid in ordered_txids],
                batch_size=batch_size,
            )
            for txid, raw_tx in zip(ordered_txids, raw_transactions):
                if sync_state.chain == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
        seen_txids = set(transactions)
        prev_txids = []
        for txid in ordered_txids:
            current_tx = transactions[txid]["decoded"] if sync_state.chain == "liquid" else transactions[txid]
            vins = current_tx.vin if sync_state.chain == "liquid" else current_tx.get("vin", [])
            for vin in vins:
                prev_txid = liquid_input_txid(vin) if sync_state.chain == "liquid" else vin.get("txid")
                if not prev_txid or prev_txid in seen_txids:
                    continue
                seen_txids.add(prev_txid)
                prev_txids.append(prev_txid)
        if prev_txids:
            raw_prev_transactions = electrum_call_many(
                client,
                [("blockchain.transaction.get", [txid]) for txid in prev_txids],
                batch_size=batch_size,
            )
            for txid, raw_tx in zip(prev_txids, raw_prev_transactions):
                if sync_state.chain == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
        heights = sorted(
            {
                int(history.get("height"))
                for history in txids.values()
                if history.get("height") is not None and int(history.get("height")) > 0
            }
        )
        if heights:
            header_hexes = electrum_call_many(
                client,
                [("blockchain.block.header", [height]) for height in heights],
                batch_size=batch_size,
            )
            for height, header_hex in zip(heights, header_hexes):
                header_timestamps[height] = block_header_timestamp(header_hex)
        for txid, history in ordered_histories:
            occurred_at = timestamp_to_iso(height_to_timestamp(history.get("height")))
            if sync_state.chain == "liquid":
                current_tx = lookup(txid)
                records.extend(
                    record_components_from_liquid_tx(
                        txid,
                        occurred_at,
                        current_tx["decoded"],
                        sync_state.descriptor_plan,
                        tracked_scripts,
                        backend["name"],
                        sync_state.policy_asset_id,
                        lambda prev_txid: lookup(prev_txid)["decoded"],
                        {"history": history, "raw_hex": current_tx["raw_hex"]},
                        confirmed_at=None if occurred_at == UNKNOWN_OCCURRED_AT else occurred_at,
                    )
                )
                continue
            tx = lookup(txid)
            normalized = record_from_electrum_tx(
                txid,
                tx,
                height_to_timestamp(history.get("height")),
                tracked_scripts,
                backend["name"],
                lookup,
            )
            if normalized:
                records.append(normalized)
    return records


def electrum_sync_adapter(backend, wallet, sync_state):
    del wallet
    return electrum_records_for_wallet(backend, sync_state), {}


SYNC_BACKEND_ADAPTERS = MappingProxyType(
    {
        "esplora": esplora_sync_adapter,
        "electrum": electrum_sync_adapter,
        "bitcoinrpc": bitcoinrpc_sync_adapter,
    }
)


__all__ = [
    "SYNC_BACKEND_ADAPTERS",
    "bitcoinrpc_sync_adapter",
    "electrum_sync_adapter",
    "esplora_sync_adapter",
    "resolve_wallet_sync_targets",
    "sync_target_from_address",
    "sync_target_from_derived",
]

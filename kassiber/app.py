import argparse
import base64
import binascii
import csv
import hashlib
import json
import os
import socket
import sqlite3
import ssl
import sys
import tempfile
import uuid
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from importlib import import_module
from pathlib import Path
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from . import __version__
from .backends import (
    BACKEND_KINDS,
    DEFAULT_ENV_FILENAME,
    DEFAULT_BACKENDS,
    backend_batch_size,
    _backend_row_to_dict,
    _validate_backend_kind,
    backend_timeout,
    backend_value,
    clear_default_backend,
    create_db_backend,
    delete_db_backend,
    get_db_backend,
    list_backends,
    list_db_backends,
    load_dotenv_file,
    load_runtime_config,
    merge_db_backends,
    resolve_backend,
    resolve_effective_env_file,
    set_default_backend,
    update_db_backend,
)
from .db import (
    APP_NAME,
    DEFAULT_DATA_ROOT,
    SCHEMA,
    ensure_column,
    ensure_data_root,
    ensure_settings_file,
    ensure_schema_compat,
    get_setting,
    open_db,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
    resolve_exports_root,
    resolve_settings_path,
    set_setting,
)
from .envelope import (
    OUTPUT_FORMATS,
    SCHEMA_VERSION,
    _write_text,
    build_envelope,
    build_error_envelope,
    derive_kind,
    emit,
    format_table_value,
    json_ready,
    print_table,
)
from .errors import AppError
from .msat import (
    MSAT_PER_BTC,
    SATS_PER_BTC,
    btc_to_msat,
    dec,
    msat_to_btc,
)
from .time_utils import (
    UNKNOWN_OCCURRED_AT,
    _iso_z,
    _parse_iso_datetime,
    now_iso,
    parse_timestamp,
    timestamp_to_iso,
)
from .util import (
    normalize_chain_value,
    normalize_network_value,
    parse_bool,
    parse_int,
    str_or_none,
)
from .tax_policy import (
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    build_tax_policy,
    supported_tax_countries,
)
from .wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    branch_limits,
    decode_liquid_transaction,
    default_policy_asset_id,
    derive_descriptor_target,
    derive_descriptor_targets,
    liquid_asset_code,
    liquid_blinding_secret,
    liquid_plan_can_unblind,
    load_descriptor_plan,
    normalize_asset_code,
    normalize_chain,
    normalize_network,
)


ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
RP2_ACCOUNTING_METHODS = ("FIFO", "LIFO", "HIFO", "LOFO")
WALLET_KINDS = [
    "descriptor",
    "xpub",
    "address",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "custom",
]
INBOUND_DIRECTIONS = {"in", "inbound", "receive", "received", "deposit", "credit", "buy"}
OUTBOUND_DIRECTIONS = {"out", "outbound", "send", "sent", "withdrawal", "withdraw", "debit", "sell"}
B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
B58_INDEX = {char: index for index, char in enumerate(B58_ALPHABET)}
BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
BECH32_INDEX = {char: index for index, char in enumerate(BECH32_CHARSET)}

def normalize_code(value):
    code = str(value).strip().lower().replace(" ", "-")
    if not code:
        raise AppError("Code cannot be empty")
    return code


def normalize_wallet_kind(value):
    kind = str(value).strip().lower()
    if kind not in WALLET_KINDS:
        raise AppError(f"Unsupported wallet kind '{value}'. Supported: {', '.join(WALLET_KINDS)}")
    return kind


def normalize_addresses(values):
    if values is None:
        return []
    if isinstance(values, str):
        values = [part.strip() for part in values.split(",")]
    output = []
    seen = set()
    for value in values:
        address = str(value).strip()
        if not address or address in seen:
            continue
        seen.add(address)
        output.append(address)
    return output


def normalize_direction(direction, amount):
    if direction:
        value = str(direction).strip().lower()
        if value in INBOUND_DIRECTIONS:
            return "inbound"
        if value in OUTBOUND_DIRECTIONS:
            return "outbound"
        raise AppError(f"Unsupported direction '{direction}'")
    return "outbound" if dec(amount) < 0 else "inbound"


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


def normalize_backend_kind(kind):
    value = str(kind).strip().lower()
    aliases = {
        "bitcoin-core": "bitcoinrpc",
        "bitcoincore": "bitcoinrpc",
        "core": "bitcoinrpc",
        "liquid-esplora": "esplora",
    }
    return aliases.get(value, value)


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


def electrum_scripthash(address):
    script = address_to_scriptpubkey(address)
    return hashlib.sha256(script).digest()[::-1].hex()


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
                raise AppError(
                    f"Electrum call {method} failed {detail}"
                )
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


_RP2_MODULES = None


def get_rp2_modules():
    global _RP2_MODULES
    if _RP2_MODULES is not None:
        return _RP2_MODULES
    try:
        _RP2_MODULES = {
            "AVLTree": import_module("prezzemolo.avl_tree").AVLTree,
            "AbstractCountry": import_module("rp2.abstract_country").AbstractCountry,
            "AccountingEngine": import_module("rp2.accounting_engine").AccountingEngine,
            "Configuration": import_module("rp2.configuration").Configuration,
            "InputData": import_module("rp2.input_data").InputData,
            "InTransaction": import_module("rp2.in_transaction").InTransaction,
            "OutTransaction": import_module("rp2.out_transaction").OutTransaction,
            "TransactionSet": import_module("rp2.transaction_set").TransactionSet,
            "compute_tax": import_module("rp2.tax_engine").compute_tax,
            "RP2Decimal": import_module("rp2.rp2_decimal").RP2Decimal,
        }
    except ModuleNotFoundError as exc:
        raise AppError(
            "RP2 integration requires the 'rp2' package. Reinstall Kassiber in a Python >= 3.10 environment."
        ) from exc
    return _RP2_MODULES


def rp2_decimal(value):
    modules = get_rp2_modules()
    return modules["RP2Decimal"](str(value))


def make_rp2_country(profile):
    AbstractCountry = get_rp2_modules()["AbstractCountry"]
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    currency_code = policy.fiat_currency

    class KassiberCountry(AbstractCountry):
        def __init__(self):
            super().__init__(policy.tax_country, currency_code)

        def get_long_term_capital_gain_period(self):
            return policy.long_term_days

        def get_default_accounting_method(self):
            return policy.default_accounting_method

        def get_accounting_methods(self):
            return set(policy.accounting_methods)

        def get_report_generators(self):
            return set(policy.report_generators)

        def get_default_generation_language(self):
            return policy.generation_language

    return KassiberCountry()


def make_rp2_configuration(profile, wallet_labels, assets):
    Configuration = get_rp2_modules()["Configuration"]
    if not wallet_labels:
        raise AppError("RP2 configuration requires at least one wallet")
    if not assets:
        raise AppError("RP2 configuration requires at least one asset")
    content = "\n".join(
        [
            "[general]",
            f"assets = {', '.join(sorted(assets))}",
            f"exchanges = {', '.join(sorted(wallet_labels))}",
            f"holders = {profile['label']}",
            "",
            "[in_header]",
            "timestamp = 0",
            "asset = 1",
            "exchange = 2",
            "holder = 3",
            "transaction_type = 4",
            "spot_price = 5",
            "crypto_in = 6",
            "crypto_fee = 7",
            "fiat_in_no_fee = 8",
            "fiat_in_with_fee = 9",
            "fiat_fee = 10",
            "unique_id = 11",
            "notes = 12",
            "",
            "[out_header]",
            "timestamp = 0",
            "asset = 1",
            "exchange = 2",
            "holder = 3",
            "transaction_type = 4",
            "spot_price = 5",
            "crypto_out_no_fee = 6",
            "crypto_fee = 7",
            "crypto_out_with_fee = 8",
            "fiat_out_no_fee = 9",
            "fiat_fee = 10",
            "unique_id = 11",
            "notes = 12",
            "",
            "[intra_header]",
            "timestamp = 0",
            "asset = 1",
            "from_exchange = 2",
            "from_holder = 3",
            "to_exchange = 4",
            "to_holder = 5",
            "spot_price = 6",
            "crypto_sent = 7",
            "crypto_received = 8",
            "unique_id = 9",
            "notes = 10",
            "",
        ]
    )
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".ini", delete=False)
    try:
        handle.write(content)
        handle.flush()
    finally:
        handle.close()
    return Configuration(handle.name, make_rp2_country(profile)), handle.name


def build_rp2_accounting_engine(profile):
    modules = get_rp2_modules()
    method_name = str(profile["gains_algorithm"]).strip().lower()
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    if method_name not in set(policy.accounting_methods):
        raise AppError(f"Unsupported RP2 accounting method '{profile['gains_algorithm']}'")
    try:
        method_module = import_module(f"rp2.plugin.accounting_method.{method_name}")
    except ModuleNotFoundError as exc:
        raise AppError(f"RP2 accounting method '{profile['gains_algorithm']}' is not available") from exc
    years_to_methods = modules["AVLTree"]()
    years_to_methods.insert_node(1970, method_module.AccountingMethod())
    return modules["AccountingEngine"](years_2_methods=years_to_methods)


def rp2_spot_price(row, quantity):
    if row["fiat_rate"] is not None:
        rate = dec(row["fiat_rate"])
        if rate > 0:
            return rate
    if row["fiat_value"] is not None and quantity > 0:
        value = dec(row["fiat_value"])
        if value > 0:
            return value / quantity
    return None


def rp2_quarantine(profile, row, reason, detail):
    return {
        "transaction_id": row["id"],
        "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"],
        "reason": reason,
        "detail_json": json.dumps(detail, sort_keys=True),
    }


def wallet_is_altbestand(wallet):
    return parse_bool(wallet.get("altbestand"), default=False)


def resolve_workspace(conn, ref=None):
    ref = ref or get_setting(conn, "context_workspace")
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
        (ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Workspace '{ref}' not found")
    return row


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND (id = ? OR lower(label) = lower(?))
        LIMIT 1
        """,
        (workspace_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Profile '{ref}' not found in the selected workspace")
    return row


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def resolve_account(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Account '{ref}' not found")
    return row


def resolve_wallet(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND (w.id = ? OR lower(w.label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Wallet '{ref}' not found")
    return row


def resolve_transaction(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM transactions
        WHERE profile_id = ? AND (id = ? OR external_id = ?)
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Transaction '{ref}' not found")
    return row


def resolve_tag(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM tags
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Tag '{ref}' not found")
    return row


def invalidate_journals(conn, profile_id):
    conn.execute(
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0 WHERE id = ?",
        (profile_id,),
    )


def init_app(conn):
    set_setting(conn, "app_version", __version__)
    conn.commit()


def create_workspace(conn, label):
    workspace_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, label, now_iso()),
    )
    set_setting(conn, "context_workspace", workspace_id)
    conn.commit()
    return conn.execute("SELECT * FROM workspaces WHERE id = ?", (workspace_id,)).fetchone()


def list_workspaces(conn):
    current = get_setting(conn, "context_workspace")
    rows = conn.execute(
        "SELECT id, label, created_at FROM workspaces ORDER BY created_at ASC"
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def ensure_default_accounts(conn, workspace_id, profile_id):
    defaults = [
        ("treasury", "Treasury", "asset", "BTC"),
        ("fees", "Fees", "expense", "BTC"),
        ("external", "External", "equity", None),
    ]
    created_at = now_iso()
    for code, label, account_type, asset in defaults:
        exists = conn.execute(
            "SELECT 1 FROM accounts WHERE profile_id = ? AND code = ?",
            (profile_id, code),
        ).fetchone()
        if exists:
            continue
        conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (str(uuid.uuid4()), workspace_id, profile_id, code, label, account_type, asset, created_at),
        )


def create_profile(conn, workspace_ref, label, fiat_currency, gains_algorithm, tax_country, tax_long_term_days):
    workspace = resolve_workspace(conn, workspace_ref)
    if tax_long_term_days < 0:
        raise AppError("Tax long-term days cannot be negative")
    try:
        policy = build_tax_policy(
            {
                "fiat_currency": fiat_currency,
                "tax_country": tax_country,
                "tax_long_term_days": tax_long_term_days,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    profile_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country, tax_long_term_days, gains_algorithm, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            workspace["id"],
            label,
            policy.fiat_currency,
            policy.tax_country,
            policy.long_term_days,
            gains_algorithm.upper(),
            now_iso(),
        ),
    )
    ensure_default_accounts(conn, workspace["id"], profile_id)
    set_setting(conn, "context_workspace", workspace["id"])
    set_setting(conn, "context_profile", profile_id)
    conn.commit()
    return conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()


def list_profiles(conn, workspace_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    current = get_setting(conn, "context_profile")
    rows = conn.execute(
        """
        SELECT id, label, fiat_currency, tax_country, tax_long_term_days, gains_algorithm, created_at
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC
        """,
        (workspace["id"],),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "fiat_currency": row["fiat_currency"],
            "tax_country": row["tax_country"],
            "tax_long_term_days": row["tax_long_term_days"],
            "gains_algorithm": row["gains_algorithm"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def create_account(conn, workspace_ref, profile_ref, code, label, account_type, asset=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    code = normalize_code(code)
    account_type = account_type.lower()
    if account_type not in ACCOUNT_TYPES:
        raise AppError(f"Unsupported account type '{account_type}'. Supported: {', '.join(sorted(ACCOUNT_TYPES))}")
    account_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            account_id,
            workspace["id"],
            profile["id"],
            code,
            label,
            account_type,
            normalize_asset_code(asset) if asset else None,
            now_iso(),
        ),
    )
    conn.commit()
    return conn.execute("SELECT * FROM accounts WHERE id = ?", (account_id,)).fetchone()


def list_accounts(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT id, code, label, account_type, COALESCE(asset, '') AS asset, created_at
        FROM accounts
        WHERE profile_id = ?
        ORDER BY code ASC
        """,
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def read_text_argument(value, file_path, label):
    if value not in (None, ""):
        return str(value).strip()
    if not file_path:
        return None
    text = Path(file_path).expanduser().read_text(encoding="utf-8").strip()
    if not text:
        raise AppError(f"{label} file '{file_path}' is empty")
    return text


def wallet_live_chain_config(config):
    if not any(
        [
            config.get("descriptor"),
            config.get("change_descriptor"),
            config.get("addresses"),
            config.get("chain"),
            config.get("network"),
        ]
    ):
        return None, None
    chain = normalize_chain_value(config.get("chain"))
    network = normalize_network_value(chain, config.get("network"))
    return chain, network


def load_wallet_descriptor_plan_from_config(config):
    try:
        return load_descriptor_plan(config)
    except ValueError as exc:
        raise AppError(str(exc)) from exc


def wallet_policy_asset_id(config, chain, network):
    explicit = str_or_none(config.get("policy_asset"))
    if explicit:
        return normalize_asset_code(explicit)
    if chain == "liquid":
        return normalize_asset_code(default_policy_asset_id(network))
    return ""


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    descriptor_text = read_text_argument(
        getattr(args, "descriptor", None),
        getattr(args, "descriptor_file", None),
        "Descriptor",
    )
    if descriptor_text:
        config["descriptor"] = descriptor_text
    change_descriptor_text = read_text_argument(
        getattr(args, "change_descriptor", None),
        getattr(args, "change_descriptor_file", None),
        "Change descriptor",
    )
    if change_descriptor_text:
        config["change_descriptor"] = change_descriptor_text
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "chain", None):
        config["chain"] = normalize_chain_value(args.chain)
    if getattr(args, "network", None):
        chain = normalize_chain_value(config.get("chain"))
        config["network"] = normalize_network_value(chain, args.network)
    if getattr(args, "gap_limit", None) is not None:
        if args.gap_limit <= 0:
            raise AppError("Descriptor gap limit must be positive")
        config["gap_limit"] = args.gap_limit
    if getattr(args, "policy_asset", None):
        config["policy_asset"] = normalize_asset_code(args.policy_asset)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    if getattr(args, "altbestand", False):
        config["altbestand"] = True
    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network
    return config


def create_wallet(conn, workspace_ref, profile_ref, label, kind, account_ref=None, config=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if account_ref:
        account = resolve_account(conn, profile["id"], account_ref)
    else:
        account = resolve_account(conn, profile["id"], "treasury")
    normalized_kind = normalize_wallet_kind(kind)
    config = config or {}
    descriptor_plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    chain, network = wallet_live_chain_config(config)
    if normalized_kind == "address" and not config.get("addresses") and not config.get("source_file"):
        raise AppError("Address wallets require at least one --address or a file-based source")
    if normalized_kind == "descriptor" and descriptor_plan is None and not config.get("source_file"):
        raise AppError("Descriptor wallets require --descriptor/--descriptor-file or a file-based source")
    if chain == "liquid" and descriptor_plan is None and not config.get("source_file"):
        raise AppError("Liquid live sync currently requires a descriptor with private blinding keys")
    if descriptor_plan and descriptor_plan.chain == "liquid":
        if not liquid_plan_can_unblind(descriptor_plan):
            raise AppError("Liquid descriptor wallets require private blinding keys for full sync and fee accounting")
        if not config.get("backend") and not config.get("source_file"):
            raise AppError("Liquid descriptor wallets require an explicit --backend; no public Liquid default is built in")
        config["policy_asset"] = wallet_policy_asset_id(config, descriptor_plan.chain, descriptor_plan.network)
    elif chain == "liquid" and not config.get("backend") and not config.get("source_file"):
        raise AppError("Liquid wallets require an explicit --backend; no public Liquid default is built in")
    if chain and network:
        config["chain"] = chain
        config["network"] = network
    wallet_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            wallet_id,
            workspace["id"],
            profile["id"],
            account["id"],
            label,
            normalized_kind,
            json.dumps(config, sort_keys=True),
            now_iso(),
        ),
    )
    conn.commit()
    return conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()


def list_wallets(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            COALESCE(a.code, '') AS account_code,
            COALESCE(a.label, '') AS account_label,
            w.config_json,
            w.created_at
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
        ORDER BY w.label ASC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        config = json.loads(row["config_json"] or "{}")
        descriptor_state = ""
        chain, network = wallet_live_chain_config(config)
        if config.get("descriptor"):
            try:
                descriptor_plan = load_descriptor_plan(config)
                descriptor_state = f"{descriptor_plan.chain}:{descriptor_plan.network}"
                chain = descriptor_plan.chain
                network = descriptor_plan.network
            except ValueError:
                descriptor_state = "invalid"
        output.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": row["account_code"] or row["account_label"],
                "chain": chain or "",
                "network": network or "",
                "backend": config.get("backend", ""),
                "addresses": ",".join(normalize_addresses(config.get("addresses"))),
                "descriptor": descriptor_state,
                "gap_limit": config.get("gap_limit", DEFAULT_DESCRIPTOR_GAP_LIMIT if descriptor_state else ""),
                "altbestand": "yes" if parse_bool(config.get("altbestand"), default=False) else "",
                "source_format": config.get("source_format", ""),
                "source_file": config.get("source_file", ""),
                "created_at": row["created_at"],
            }
        )
    return output


def set_wallet_altbestand(conn, workspace_ref, profile_ref, wallet_ref, enabled):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"] or "{}")
    if enabled:
        config["altbestand"] = True
    else:
        config.pop("altbestand", None)
    conn.execute("UPDATE wallets SET config_json = ? WHERE id = ?", (json.dumps(config, sort_keys=True), wallet["id"]))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "wallet": wallet["label"],
        "altbestand": bool(enabled),
    }


WALLET_KIND_CATALOG = {
    "descriptor": {
        "summary": "Output-descriptor wallet with optional change branch; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "change_descriptor", "gap_limit", "backend", "chain", "network", "policy_asset"],
        "requires": ["descriptor"],
    },
    "xpub": {
        "summary": "Extended-public-key wallet derived to address set; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "gap_limit", "backend", "chain", "network"],
        "requires": ["descriptor"],
    },
    "address": {
        "summary": "Bare-address list wallet; useful for receive-only tracking or imports.",
        "config_fields": ["addresses", "backend", "chain", "network", "source_file", "source_format"],
        "requires": ["addresses|source_file"],
    },
    "coreln": {
        "summary": "Core Lightning CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "lnd": {
        "summary": "LND CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "nwc": {
        "summary": "Nostr Wallet Connect wallet fed by CSV exports.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "phoenix": {
        "summary": "Phoenix Wallet CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "river": {
        "summary": "River Financial CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "custom": {
        "summary": "Custom CSV/JSON source; use with --config/--config-file to describe field mapping.",
        "config_fields": ["source_file", "source_format", "config"],
        "requires": ["source_file"],
    },
}


def list_wallet_kinds():
    rows = []
    for kind in WALLET_KINDS:
        entry = WALLET_KIND_CATALOG.get(kind, {"summary": "", "config_fields": [], "requires": []})
        rows.append(
            {
                "kind": kind,
                "summary": entry["summary"],
                "requires": ", ".join(entry["requires"]),
                "config_fields": ", ".join(entry["config_fields"]),
            }
        )
    return rows


def _wallet_row_to_dict(row):
    config = json.loads(row["config_json"] or "{}")
    descriptor_state = ""
    chain, network = wallet_live_chain_config(config)
    if config.get("descriptor"):
        try:
            descriptor_plan = load_descriptor_plan(config)
            descriptor_state = f"{descriptor_plan.chain}:{descriptor_plan.network}"
            chain = descriptor_plan.chain
            network = descriptor_plan.network
        except ValueError:
            descriptor_state = "invalid"
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "account_id": row["account_id"],
        "account_code": row["account_code"] if "account_code" in row.keys() else None,
        "account_label": row["account_label"] if "account_label" in row.keys() else None,
        "label": row["label"],
        "kind": row["kind"],
        "chain": chain or "",
        "network": network or "",
        "backend": config.get("backend", ""),
        "addresses": normalize_addresses(config.get("addresses")),
        "descriptor": bool(config.get("descriptor")),
        "descriptor_state": descriptor_state,
        "change_descriptor": bool(config.get("change_descriptor")),
        "gap_limit": config.get("gap_limit"),
        "policy_asset": config.get("policy_asset"),
        "altbestand": parse_bool(config.get("altbestand"), default=False),
        "source_file": config.get("source_file", ""),
        "source_format": config.get("source_format", ""),
        "config": config,
        "created_at": row["created_at"],
    }


def get_wallet_details(conn, workspace_ref, profile_ref, wallet_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return _wallet_row_to_dict(wallet)


def update_wallet(conn, workspace_ref, profile_ref, wallet_ref, updates):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    new_label = updates.get("label")
    new_account = updates.get("account")
    new_altbestand = updates.get("altbestand")
    config_updates = updates.get("config") or {}
    clear_fields = updates.get("clear") or []

    if (
        new_label is None
        and new_account is None
        and new_altbestand is None
        and not config_updates
        and not clear_fields
    ):
        raise AppError(
            "wallets update requires at least one field to change",
            code="validation",
            hint="Pass --label, --account, --set-altbestand/--clear-altbestand, --config/--config-file, or --clear <field>",
        )

    label_value = new_label if new_label is not None else wallet["label"]
    account_id = wallet["account_id"]
    if new_account is not None:
        account = resolve_account(conn, profile["id"], new_account)
        account_id = account["id"]

    config = json.loads(wallet["config_json"] or "{}")
    for field in clear_fields:
        config.pop(field, None)
    for key, value in config_updates.items():
        if value is None:
            config.pop(key, None)
        else:
            config[key] = value
    if new_altbestand is True:
        config["altbestand"] = True
    elif new_altbestand is False:
        config.pop("altbestand", None)

    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network

    conn.execute(
        """
        UPDATE wallets
        SET label = ?, account_id = ?, config_json = ?
        WHERE id = ?
        """,
        (label_value, account_id, json.dumps(config, sort_keys=True), wallet["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    updated = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.id = ?
        """,
        (wallet["id"],),
    ).fetchone()
    return _wallet_row_to_dict(updated)


def delete_wallet(conn, workspace_ref, profile_ref, wallet_ref, cascade=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    tx_count = conn.execute(
        "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = ?",
        (wallet["id"],),
    ).fetchone()["n"]
    if tx_count and not cascade:
        raise AppError(
            f"Wallet '{wallet['label']}' has {tx_count} transaction(s); pass --cascade to delete them too",
            code="conflict",
            hint="Use --cascade to remove the wallet and all associated transactions/journal entries.",
            details={"transactions": tx_count},
        )
    conn.execute("DELETE FROM wallets WHERE id = ?", (wallet["id"],))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "id": wallet["id"],
        "label": wallet["label"],
        "deleted": True,
        "cascaded_transactions": tx_count if cascade else 0,
    }


def load_import_records(file_path, input_format):
    if input_format == "json":
        with open(file_path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        if isinstance(payload, dict):
            payload = payload.get("transactions", [])
        if not isinstance(payload, list):
            raise AppError("JSON import must be a list of transaction objects")
        return payload
    if input_format == "csv":
        with open(file_path, "r", encoding="utf-8", newline="") as handle:
            return list(csv.DictReader(handle))
    if input_format == "btcpay_json":
        return load_btcpay_export_records(file_path, "json")
    if input_format == "btcpay_csv":
        return load_btcpay_export_records(file_path, "csv")
    if input_format == "phoenix_csv":
        return load_phoenix_csv_records(file_path)
    raise AppError(f"Unsupported input format '{input_format}'")


def parse_btcpay_amount(amount_text, currency=None):
    if amount_text is None:
        raise AppError("BTCPay export is missing Amount")
    text = str(amount_text).strip()
    asset = str(currency or "BTC").strip().upper()
    suffixes = [asset, asset.lower(), asset.upper()]
    for suffix in suffixes:
        if suffix and text.endswith(suffix):
            text = text[: -len(suffix)].strip()
            break
    return dec(text)


def parse_btcpay_labels(value):
    if value is None or value == "":
        return []
    if isinstance(value, list):
        return [str(item).strip() for item in value if str(item).strip()]
    return [part.strip() for part in str(value).split(",") if part.strip()]


def normalize_btcpay_record(record):
    sanitized_record = {str(key): value for key, value in record.items() if key is not None}
    txid = sanitized_record.get("TransactionId") or sanitized_record.get("Transaction Id")
    timestamp = sanitized_record.get("Timestamp")
    currency = normalize_asset_code(sanitized_record.get("Currency") or "BTC")
    amount = parse_btcpay_amount(sanitized_record.get("Amount"), currency=currency)
    comment = sanitized_record.get("Comment")
    labels = parse_btcpay_labels(sanitized_record.get("Labels"))
    return {
        "txid": txid,
        "occurred_at": timestamp,
        "direction": "outbound" if amount < 0 else "inbound",
        "asset": currency,
        "amount": abs(amount),
        "fee": Decimal("0"),
        "fiat_rate": None,
        "fiat_value": None,
        "kind": "withdrawal" if amount < 0 else "deposit",
        "description": comment or "Imported from BTCPay",
        "counterparty": None,
        "_btcpay_comment": comment,
        "_btcpay_labels": labels,
        "raw_json": json.dumps(json_ready(sanitized_record), sort_keys=True),
    }


def load_btcpay_export_records(file_path, input_format):
    if input_format == "json":
        with open(file_path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        if not isinstance(payload, list):
            raise AppError("BTCPay JSON export must be a list of transaction objects")
        rows = payload
    elif input_format == "csv":
        with open(file_path, "r", encoding="utf-8", newline="") as handle:
            rows = list(csv.DictReader(handle))
    else:
        raise AppError(f"Unsupported BTCPay input format '{input_format}'")
    return [normalize_btcpay_record(row) for row in rows]


def is_btcpay_format(input_format):
    return input_format in {"btcpay_json", "btcpay_csv"}


_PHOENIX_REQUIRED_COLUMNS = (
    "date",
    "id",
    "type",
    "amount_msat",
)

_PHOENIX_OUTBOUND_TYPES = {
    "lightning_sent",
    "swap_out",
    "legacy_swap_out",
    "channel_close",
    "liquidity_purchase",
    "fee_bumping",
}

_PHOENIX_INBOUND_TYPES = {
    "lightning_received",
    "swap_in",
    "legacy_swap_in",
    "legacy_pay_to_open",
}


def parse_phoenix_fiat_amount(amount_text):
    """Parse a Phoenix amount_fiat cell like "22.9998 USD" into (Decimal, currency)."""
    if amount_text is None:
        return None, None
    text = str(amount_text).strip()
    if not text:
        return None, None
    parts = text.split()
    if len(parts) == 1:
        return dec(parts[0]), None
    value = dec(parts[0])
    currency = normalize_asset_code(parts[1])
    return value, currency


def normalize_phoenix_record(record):
    sanitized = {str(key): value for key, value in record.items() if key is not None}
    for column in _PHOENIX_REQUIRED_COLUMNS:
        if column not in sanitized:
            raise AppError(f"Phoenix CSV is missing required column '{column}'")
    phoenix_type = str(sanitized.get("type") or "").strip() or "unknown"
    amount_msat_raw = str(sanitized.get("amount_msat") or "0").strip() or "0"
    try:
        amount_msat_signed = int(amount_msat_raw)
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix amount_msat '{amount_msat_raw}'") from exc
    if amount_msat_signed < 0:
        direction = "outbound"
    elif amount_msat_signed > 0:
        direction = "inbound"
    elif phoenix_type in _PHOENIX_OUTBOUND_TYPES:
        direction = "outbound"
    elif phoenix_type in _PHOENIX_INBOUND_TYPES:
        direction = "inbound"
    else:
        direction = "outbound"
    amount_btc = msat_to_btc(abs(amount_msat_signed))
    mining_fee_sat_raw = str(sanitized.get("mining_fee_sat") or "0").strip() or "0"
    service_fee_msat_raw = str(sanitized.get("service_fee_msat") or "0").strip() or "0"
    try:
        mining_fee_msat = int(mining_fee_sat_raw) * 1000
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix mining_fee_sat '{mining_fee_sat_raw}'") from exc
    try:
        service_fee_msat = int(service_fee_msat_raw)
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix service_fee_msat '{service_fee_msat_raw}'") from exc
    fee_btc = msat_to_btc(mining_fee_msat + service_fee_msat)
    fiat_value_signed, _ = parse_phoenix_fiat_amount(sanitized.get("amount_fiat"))
    fiat_value = abs(fiat_value_signed) if fiat_value_signed is not None else None
    fiat_rate = None
    if fiat_value is not None and amount_btc > 0:
        fiat_rate = fiat_value / amount_btc
    description = str_or_none(sanitized.get("description"))
    counterparty = str_or_none(sanitized.get("destination"))
    txid = str_or_none(sanitized.get("tx_id")) or str_or_none(sanitized.get("payment_hash"))
    return {
        "txid": sanitized.get("id"),
        "occurred_at": sanitized.get("date"),
        "direction": direction,
        "asset": "BTC",
        "amount": amount_btc,
        "fee": fee_btc,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "kind": phoenix_type,
        "description": description,
        "counterparty": counterparty,
        "_phoenix_type": phoenix_type,
        "_phoenix_description": description,
        "_phoenix_onchain_txid": txid,
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }


def load_phoenix_csv_records(file_path):
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = rows[0].keys()
    missing = [column for column in _PHOENIX_REQUIRED_COLUMNS if column not in header]
    if missing:
        raise AppError(
            "Phoenix CSV is missing required columns: " + ", ".join(missing)
        )
    return [normalize_phoenix_record(row) for row in rows]


def is_phoenix_format(input_format):
    return input_format == "phoenix_csv"


def apply_phoenix_metadata(conn, profile, wallet, records):
    notes_set = 0
    tags_added = 0
    tags_created = 0
    for record in records:
        txid = record.get("txid")
        if not txid:
            continue
        tx = conn.execute(
            """
            SELECT id, note
            FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (profile["id"], wallet["id"], txid),
        ).fetchone()
        if not tx:
            continue
        description = str_or_none(record.get("_phoenix_description"))
        if description and not tx["note"]:
            conn.execute(
                "UPDATE transactions SET note = ? WHERE id = ?",
                (description, tx["id"]),
            )
            notes_set += 1
        phoenix_type = str_or_none(record.get("_phoenix_type"))
        if phoenix_type:
            tag, created = ensure_tag_row(
                conn, profile["workspace_id"], profile["id"], phoenix_type, phoenix_type
            )
            if created:
                tags_created += 1
            before = conn.total_changes
            conn.execute(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                (tx["id"], tag["id"]),
            )
            if conn.total_changes > before:
                tags_added += 1
    conn.commit()
    return {
        "phoenix_notes_set": notes_set,
        "phoenix_tags_added": tags_added,
        "phoenix_tags_created": tags_created,
    }


def make_transaction_fingerprint(wallet_id, external_id, occurred_at, direction, asset, amount, fee):
    payload = json.dumps(
        {
            "wallet_id": wallet_id,
            "external_id": external_id,
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount": str(amount),
            "fee": str(fee),
        },
        sort_keys=True,
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def normalize_import_record(record):
    raw_amount = dec(record.get("amount"))
    direction = normalize_direction(record.get("direction"), raw_amount)
    amount = abs(raw_amount)
    fee = abs(dec(record.get("fee"), "0"))
    fiat_rate = record.get("fiat_rate")
    fiat_value = record.get("fiat_value")
    rate = dec(fiat_rate) if fiat_rate not in (None, "") else None
    value = dec(fiat_value) if fiat_value not in (None, "") else None
    if value is None and rate is not None:
        value = amount * rate
    raw_json = record.get("raw_json")
    if raw_json is None:
        raw_json = json.dumps(json_ready(record), sort_keys=True)
    elif not isinstance(raw_json, str):
        raw_json = json.dumps(json_ready(raw_json), sort_keys=True)
    return {
        "external_id": str(record.get("txid") or record.get("id") or ""),
        "occurred_at": parse_timestamp(record.get("occurred_at") or record.get("timestamp") or record.get("date")),
        "direction": direction,
        "asset": normalize_asset_code(record.get("asset") or "BTC"),
        "amount": amount,
        "fee": fee,
        "fiat_rate": rate,
        "fiat_value": value,
        "kind": record.get("kind"),
        "description": record.get("description"),
        "counterparty": record.get("counterparty"),
        "raw_json": raw_json,
    }


def insert_wallet_records(conn, profile, wallet, records, source_label):
    imported = 0
    skipped = 0
    for record in records:
        normalized = normalize_import_record(record)
        fingerprint = make_transaction_fingerprint(
            wallet["id"],
            normalized["external_id"],
            normalized["occurred_at"],
            normalized["direction"],
            normalized["asset"],
            normalized["amount"],
            normalized["fee"],
        )
        exists = conn.execute(
            "SELECT 1 FROM transactions WHERE fingerprint = ?",
            (fingerprint,),
        ).fetchone()
        if exists:
            skipped += 1
            continue
        tx_id = str(uuid.uuid4())
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, counterparty, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                normalized["external_id"] or None,
                fingerprint,
                normalized["occurred_at"],
                normalized["direction"],
                normalized["asset"],
                btc_to_msat(normalized["amount"]),
                btc_to_msat(normalized["fee"]),
                profile["fiat_currency"],
                float(normalized["fiat_rate"]) if normalized["fiat_rate"] is not None else None,
                float(normalized["fiat_value"]) if normalized["fiat_value"] is not None else None,
                normalized["kind"],
                normalized["description"],
                normalized["counterparty"],
                normalized["raw_json"],
                now_iso(),
            ),
        )
        imported += 1
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "wallet": wallet["label"],
        "source": source_label,
        "imported": imported,
        "skipped": skipped,
    }


def import_into_wallet(conn, workspace_ref, profile_ref, wallet_ref, file_path, input_format):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    records = load_import_records(file_path, input_format)
    outcome = insert_wallet_records(conn, profile, wallet, records, f"file:{input_format}")
    if is_btcpay_format(input_format):
        outcome.update(apply_btcpay_metadata(conn, profile, wallet, records))
    if is_phoenix_format(input_format):
        outcome.update(apply_phoenix_metadata(conn, profile, wallet, records))
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


def ensure_tag_row(conn, workspace_id, profile_id, code, label):
    normalized_code = normalize_code(code)
    existing = conn.execute(
        "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
        (profile_id, normalized_code),
    ).fetchone()
    if existing:
        return existing, False
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace_id, profile_id, normalized_code, label, now_iso()),
    )
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone(), True


def apply_btcpay_metadata(conn, profile, wallet, records):
    notes_set = 0
    tags_added = 0
    tags_created = 0
    for record in records:
        txid = record.get("txid")
        if not txid:
            continue
        tx = conn.execute(
            """
            SELECT id, note
            FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (profile["id"], wallet["id"], txid),
        ).fetchone()
        if not tx:
            continue
        comment = str_or_none(record.get("_btcpay_comment"))
        if comment and not tx["note"]:
            conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (comment, tx["id"]))
            notes_set += 1
        for label in record.get("_btcpay_labels", []):
            tag, created = ensure_tag_row(conn, profile["workspace_id"], profile["id"], label, label)
            if created:
                tags_created += 1
            before = conn.total_changes
            conn.execute(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                (tx["id"], tag["id"]),
            )
            if conn.total_changes > before:
                tags_added += 1
    conn.commit()
    return {
        "btcpay_notes_set": notes_set,
        "btcpay_tags_added": tags_added,
        "btcpay_tags_created": tags_created,
    }


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
            return {
                "chain": "",
                "network": "",
                "descriptor_plan": None,
                "policy_asset_id": "",
                "targets": [],
                "tracked_scripts": {},
                "history_cache": {},
            }
        chain = normalize_chain_value(config.get("chain"))
        network = normalize_network_value(chain, config.get("network"))
        if chain == "liquid":
            raise AppError("Liquid live sync currently requires descriptor-backed wallets so outputs can be unblinded locally")
        validate_backend_for_wallet(backend, chain, network, has_descriptor=False)
        targets = [sync_target_from_address(address, chain, network, index) for index, address in enumerate(addresses)]
    tracked_scripts = {target["script_pubkey"]: target for target in targets}
    return {
        "chain": chain,
        "network": network,
        "descriptor_plan": descriptor_plan,
        "policy_asset_id": wallet_policy_asset_id(config, chain, network),
        "targets": targets,
        "tracked_scripts": tracked_scripts,
        "history_cache": history_cache,
    }


def fetch_esplora_history(base_url, resource_path, max_pages=None):
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
        page = http_get_json(chain_url)
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
    for tx in http_get_json(mempool_url):
        txid = tx.get("txid")
        if txid and txid not in seen_txids:
            seen_txids.add(txid)
            transactions.append(tx)
    return transactions


def fetch_esplora_scripthash_transactions(base_url, script_pubkey_hex, max_pages=None):
    return fetch_esplora_history(
        base_url,
        f"scripthash/{scriptpubkey_scripthash(script_pubkey_hex)}",
        max_pages=max_pages,
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
    block_time = ((tx.get("status") or {}).get("block_time"))
    occurred_at = timestamp_to_iso(block_time)
    return {
        "txid": tx.get("txid"),
        "occurred_at": occurred_at,
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
):
    net_sats = defaultdict(int)
    fee_sats = defaultdict(int)
    for index, output in enumerate(tx.vout):
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


def esplora_records_for_wallet(backend, sync_state):
    max_pages = parse_int(backend_value(backend, "maxpages"), default=0) or None
    transactions_by_txid = {}
    for target in sync_state["targets"]:
        for tx in fetch_esplora_scripthash_transactions(backend["url"], target["script_pubkey"], max_pages=max_pages):
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
        if sync_state["chain"] == "liquid":
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
                    sync_state["descriptor_plan"],
                    sync_state["tracked_scripts"],
                    backend["name"],
                    sync_state["policy_asset_id"],
                    liquid_tx_lookup,
                    {"tx": tx, "raw_hex": raw_hex},
                )
            )
        else:
            normalized = record_from_bitcoin_esplora_tx(tx, sync_state["tracked_scripts"], backend["name"])
            if normalized:
                records.append(normalized)
    return records


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
    for detail in details:
        category = str(detail.get("category") or "").lower()
        if category in {"orphan", "immature"}:
            continue
        amount_total += dec(detail.get("amount"), "0")
        fee_total += abs(dec(detail.get("fee"), "0"))
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
    return {
        "txid": txid,
        "occurred_at": occurred_at,
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


def electrum_records_for_wallet(backend, sync_state):
    transactions = {}
    header_timestamps = {}
    records = []
    batch_size = backend_batch_size(backend)
    tracked_scripts = (
        set(sync_state["tracked_scripts"])
        if sync_state["chain"] == "bitcoin"
        else sync_state["tracked_scripts"]
    )
    with ElectrumClient(backend) as client:
        histories = []
        history_cache = sync_state.get("history_cache") or {}
        uncached_scripthashes = []
        for target in sync_state["targets"]:
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
                if sync_state["chain"] == "liquid":
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
                if sync_state["chain"] == "liquid":
                    transactions[txid] = {
                        "raw_hex": raw_tx,
                        "decoded": decode_liquid_transaction(raw_tx),
                    }
                else:
                    transactions[txid] = decode_raw_transaction(raw_tx)
        seen_txids = set(transactions)
        prev_txids = []
        for txid in ordered_txids:
            current_tx = transactions[txid]["decoded"] if sync_state["chain"] == "liquid" else transactions[txid]
            vins = current_tx.vin if sync_state["chain"] == "liquid" else current_tx.get("vin", [])
            for vin in vins:
                prev_txid = liquid_input_txid(vin) if sync_state["chain"] == "liquid" else vin.get("txid")
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
                if sync_state["chain"] == "liquid":
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
            if sync_state["chain"] == "liquid":
                current_tx = lookup(txid)
                records.extend(
                    record_components_from_liquid_tx(
                        txid,
                        occurred_at,
                        current_tx["decoded"],
                        sync_state["descriptor_plan"],
                        tracked_scripts,
                        backend["name"],
                        sync_state["policy_asset_id"],
                        lambda prev_txid: lookup(prev_txid)["decoded"],
                        {"history": history, "raw_hex": current_tx["raw_hex"]},
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


def sync_wallet_from_backend(conn, runtime_config, workspace_ref, profile_ref, wallet):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    config = json.loads(wallet["config_json"] or "{}")
    backend = resolve_backend(runtime_config, config.get("backend"))
    sync_state = resolve_wallet_sync_targets(backend, wallet)
    if not sync_state["targets"]:
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": "no addresses or descriptors configured for backend sync",
        }
    kind = normalize_backend_kind(backend["kind"])
    adapter_meta = {}
    if kind == "esplora":
        normalized_records = esplora_records_for_wallet(backend, sync_state)
    elif kind == "electrum":
        normalized_records = electrum_records_for_wallet(backend, sync_state)
    elif kind == "bitcoinrpc":
        normalized_records, adapter_meta = bitcoinrpc_records_for_wallet(
            backend,
            wallet,
            [target["address"] for target in sync_state["targets"] if target.get("address")],
        )
    else:
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": f"backend kind '{backend['kind']}' is not implemented yet",
        }
    outcome = insert_wallet_records(conn, profile, wallet, normalized_records, f"backend:{backend['name']}")
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = backend["url"]
    outcome["chain"] = sync_state["chain"]
    outcome["network"] = sync_state["network"]
    outcome["sync_mode"] = "descriptor" if sync_state["descriptor_plan"] else "addresses"
    outcome["target_count"] = len(sync_state["targets"])
    if sync_state["descriptor_plan"]:
        outcome["gap_limit"] = sync_state["descriptor_plan"].gap_limit
    else:
        outcome["addresses"] = ",".join(target["address"] for target in sync_state["targets"] if target.get("address"))
    if sync_state["policy_asset_id"]:
        outcome["policy_asset"] = sync_state["policy_asset_id"]
    outcome.update(adapter_meta)
    return outcome


def sync_wallet(conn, runtime_config, workspace_ref, profile_ref, wallet_ref=None, sync_all=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if sync_all:
        wallets = conn.execute("SELECT * FROM wallets WHERE profile_id = ? ORDER BY label ASC", (profile["id"],)).fetchall()
    else:
        if not wallet_ref:
            raise AppError("Provide --wallet or use --all")
        wallets = [resolve_wallet(conn, profile["id"], wallet_ref)]
    results = []
    for wallet in wallets:
        config = json.loads(wallet["config_json"] or "{}")
        source_file = config.get("source_file")
        source_format = config.get("source_format")
        addresses = normalize_addresses(config.get("addresses"))
        has_descriptor = bool(str_or_none(config.get("descriptor")))
        if source_file and source_format:
            outcome = import_into_wallet(
                conn,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                source_file,
                source_format,
            )
            results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        if addresses:
            outcome = sync_wallet_from_backend(conn, runtime_config, profile["workspace_id"], profile["id"], wallet)
            if outcome.get("status") == "skipped":
                results.append(outcome)
            else:
                results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        if has_descriptor:
            outcome = sync_wallet_from_backend(conn, runtime_config, profile["workspace_id"], profile["id"], wallet)
            if outcome.get("status") == "skipped":
                results.append(outcome)
            else:
                results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        results.append(
            {
                "wallet": wallet["label"],
                "status": "skipped",
                "reason": "no file source, descriptor, or backend addresses configured",
            }
        )
    return results


def resolve_descriptor_branch_index(plan, branch):
    if branch in (None, "", "all"):
        return None
    normalized = str(branch).strip().lower()
    if normalized in {"0", "receive", "external"}:
        return 0
    if normalized in {"1", "change", "internal"}:
        return 1
    raise AppError("Descriptor branch must be one of: all, receive, change, 0, 1")


def derive_wallet_targets(conn, workspace_ref, profile_ref, wallet_ref, branch=None, start=0, count=None):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"] or "{}")
    plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    if plan is None:
        raise AppError(f"Wallet '{wallet['label']}' does not have a descriptor configured")
    if start < 0:
        raise AppError("Descriptor derivation start must be non-negative")
    count = count if count is not None else plan.gap_limit
    if count <= 0:
        raise AppError("Descriptor derivation count must be positive")
    branch_index = resolve_descriptor_branch_index(plan, branch)
    return [
        sync_target_from_derived(target)
        for target in derive_descriptor_targets(
            plan,
            branch_index=branch_index,
            start=start,
            end=start + count,
        )
    ]


def list_transactions(conn, workspace_ref, profile_ref, wallet_ref=None, limit=100):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    params = [profile["id"]]
    wallet_clause = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_clause = "AND t.wallet_id = ?"
        params.append(wallet["id"])
    params.append(limit)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            COALESCE(t.external_id, '') AS external_id,
            t.occurred_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            CASE WHEN t.excluded = 1 THEN 'yes' ELSE '' END AS excluded,
            COALESCE(GROUP_CONCAT(tags.code, ','), '') AS tags
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE t.profile_id = ? {wallet_clause}
        GROUP BY t.id
        ORDER BY t.occurred_at DESC, t.created_at DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    results = []
    for row in rows:
        record = dict(row)
        record["amount_msat"] = int(record["amount"])
        record["amount"] = float(msat_to_btc(record["amount"]))
        record["fee_msat"] = int(record["fee"])
        record["fee"] = float(msat_to_btc(record["fee"]))
        results.append(record)
    return results


def set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, note):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (note, tx["id"]))
    conn.commit()
    return {"transaction_id": tx["id"], "note": note}


def clear_transaction_note(conn, workspace_ref, profile_ref, tx_ref):
    return set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, None)


def set_transaction_excluded(conn, workspace_ref, profile_ref, tx_ref, excluded):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET excluded = ? WHERE id = ?", (1 if excluded else 0, tx["id"]))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "excluded": bool(excluded)}


def create_tag(conn, workspace_ref, profile_ref, code, label):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace["id"], profile["id"], normalize_code(code), label, now_iso()),
    )
    conn.commit()
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone()


def list_tags(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        "SELECT id, code, label, created_at FROM tags WHERE profile_id = ? ORDER BY code ASC",
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def add_tag_to_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    tag = resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
        (tx["id"], tag["id"]),
    )
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "added"}


def remove_tag_from_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    tag = resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
        (tx["id"], tag["id"]),
    )
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "removed"}


def _tags_for_transaction(conn, tx_id):
    rows = conn.execute(
        """
        SELECT t.code, t.label
        FROM transaction_tags tt
        JOIN tags t ON t.id = tt.tag_id
        WHERE tt.transaction_id = ?
        ORDER BY t.code ASC
        """,
        (tx_id,),
    ).fetchall()
    return [{"code": row["code"], "label": row["label"]} for row in rows]


def get_transaction_record(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    wallet = conn.execute(
        "SELECT id, label FROM wallets WHERE id = ?",
        (tx["wallet_id"],),
    ).fetchone()
    return {
        "transaction_id": tx["id"],
        "external_id": tx["external_id"] or "",
        "occurred_at": tx["occurred_at"],
        "direction": tx["direction"],
        "asset": tx["asset"],
        "amount": float(msat_to_btc(tx["amount"])),
        "amount_msat": int(tx["amount"]),
        "fee": float(msat_to_btc(tx["fee"])),
        "fee_msat": int(tx["fee"]),
        "counterparty": tx["counterparty"] or "",
        "wallet_id": wallet["id"] if wallet else "",
        "wallet_label": wallet["label"] if wallet else "",
        "note": tx["note"] or "",
        "excluded": bool(tx["excluded"]),
        "tags": _tags_for_transaction(conn, tx["id"]),
    }


def list_transaction_records(
    conn,
    workspace_ref,
    profile_ref,
    wallet=None,
    tag=None,
    has_note=None,
    excluded=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_EVENTS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_EVENTS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_EVENTS_LIMIT}",
            code="validation",
        )

    where = ["t.profile_id = ?"]
    params = [profile["id"]]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None

    if wallet:
        wallet_row = resolve_wallet(conn, profile["id"], wallet)
        where.append("t.wallet_id = ?")
        params.append(wallet_row["id"])
    if tag:
        tag_row = resolve_tag(conn, profile["id"], tag)
        where.append("EXISTS (SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = t.id AND tt.tag_id = ?)")
        params.append(tag_row["id"])
    if has_note is True:
        where.append("t.note IS NOT NULL AND t.note != ''")
    elif has_note is False:
        where.append("(t.note IS NULL OR t.note = '')")
    if excluded is True:
        where.append("t.excluded = 1")
    elif excluded is False:
        where.append("t.excluded = 0")
    if start_ts:
        where.append("t.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("t.occurred_at <= ?")
        params.append(end_ts)

    cursor_data = _decode_event_cursor(cursor)
    if cursor_data:
        where.append(
            "(t.occurred_at < ? OR "
            "(t.occurred_at = ? AND t.created_at < ?) OR "
            "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            t.id,
            t.occurred_at,
            t.created_at,
            t.external_id,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.counterparty,
            t.note,
            t.excluded,
            w.id AS wallet_id,
            w.label AS wallet_label
        FROM transactions t
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(where)}
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    records = []
    for row in page:
        records.append(
            {
                "transaction_id": row["id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "counterparty": row["counterparty"] or "",
                "wallet_id": row["wallet_id"] or "",
                "wallet_label": row["wallet_label"] or "",
                "note": row["note"] or "",
                "excluded": bool(row["excluded"]),
                "tags": _tags_for_transaction(conn, row["id"]),
            }
        )
    next_cursor = _encode_event_cursor(page[-1]) if has_more and page else None
    return {
        "records": records,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def normalize_bip329_record(record):
    if not isinstance(record, dict):
        raise AppError("BIP329 records must be JSON objects")
    record_type = str(record.get("type") or "").strip()
    ref = str(record.get("ref") or "").strip()
    if record_type not in {"tx", "addr", "pubkey", "input", "output", "xpub"}:
        raise AppError(f"Unsupported BIP329 record type '{record_type}'")
    if not ref:
        raise AppError("BIP329 records require a non-empty ref")
    spendable = record.get("spendable")
    if spendable is not None and not isinstance(spendable, bool):
        raise AppError("BIP329 spendable must be a boolean when present")
    if spendable is not None and record_type != "output":
        raise AppError("BIP329 spendable is only valid for output records")
    return {
        "type": record_type,
        "ref": ref,
        "label": str_or_none(record.get("label")),
        "origin": str_or_none(record.get("origin")),
        "spendable": spendable,
        "data": {
            key: value
            for key, value in record.items()
            if key not in {"type", "ref", "label", "origin", "spendable"}
        },
    }


def load_bip329_file(file_path):
    records = []
    with open(file_path, "r", encoding="utf-8") as handle:
        for line_number, raw_line in enumerate(handle, start=1):
            line = raw_line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError as exc:
                raise AppError(f"Invalid BIP329 JSON on line {line_number}") from exc
            records.append(normalize_bip329_record(payload))
    return records


def import_bip329_labels(conn, workspace_ref, profile_ref, file_path, wallet_ref=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    records = load_bip329_file(file_path)
    imported = 0
    updated = 0
    transaction_tags_added = 0
    transaction_tags_created = 0
    for record in records:
        existing = conn.execute(
            """
            SELECT id
            FROM bip329_labels
            WHERE profile_id = ?
              AND COALESCE(wallet_id, '') = ?
              AND record_type = ?
              AND ref = ?
              AND COALESCE(label, '') = ?
              AND COALESCE(origin, '') = ?
            LIMIT 1
            """,
            (
                profile["id"],
                wallet["id"] if wallet else "",
                record["type"],
                record["ref"],
                record["label"] or "",
                record["origin"] or "",
            ),
        ).fetchone()
        if existing:
            conn.execute(
                """
                UPDATE bip329_labels
                SET spendable = ?, data_json = ?
                WHERE id = ?
                """,
                (
                    None if record["spendable"] is None else (1 if record["spendable"] else 0),
                    json.dumps(record["data"], sort_keys=True),
                    existing["id"],
                ),
            )
            updated += 1
        else:
            conn.execute(
                """
                INSERT INTO bip329_labels(
                    id, workspace_id, profile_id, wallet_id, record_type, ref,
                    label, origin, spendable, data_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid.uuid4()),
                    workspace["id"],
                    profile["id"],
                    wallet["id"] if wallet else None,
                    record["type"],
                    record["ref"],
                    record["label"],
                    record["origin"],
                    None if record["spendable"] is None else (1 if record["spendable"] else 0),
                    json.dumps(record["data"], sort_keys=True),
                    now_iso(),
                ),
            )
            imported += 1
        if record["type"] == "tx" and record["label"]:
            query = """
                SELECT id
                FROM transactions
                WHERE profile_id = ? AND external_id = ?
            """
            params = [profile["id"], record["ref"]]
            if wallet:
                query += " AND wallet_id = ?"
                params.append(wallet["id"])
            tx_rows = conn.execute(query, params).fetchall()
            for tx in tx_rows:
                tag, created = ensure_tag_row(
                    conn,
                    profile["workspace_id"],
                    profile["id"],
                    record["label"],
                    record["label"],
                )
                if created:
                    transaction_tags_created += 1
                before = conn.total_changes
                conn.execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                    (tx["id"], tag["id"]),
                )
                if conn.total_changes > before:
                    transaction_tags_added += 1
    conn.commit()
    return {
        "file": os.path.abspath(file_path),
        "imported": imported,
        "updated": updated,
        "records": len(records),
        "transaction_tags_created": transaction_tags_created,
        "transaction_tags_added": transaction_tags_added,
    }


def list_bip329_labels(conn, workspace_ref, profile_ref, wallet_ref=None, limit=100):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    wallet_clause = "AND wallet_id = ?" if wallet else ""
    params = [profile["id"]]
    if wallet:
        params.append(wallet["id"])
    params.append(limit)
    rows = conn.execute(
        f"""
        SELECT
            record_type AS type,
            ref,
            COALESCE(label, '') AS label,
            COALESCE(origin, '') AS origin,
            CASE
                WHEN spendable IS NULL THEN ''
                WHEN spendable = 1 THEN 'true'
                ELSE 'false'
            END AS spendable,
            created_at
        FROM bip329_labels
        WHERE profile_id = ? {wallet_clause}
        ORDER BY created_at DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    return [dict(row) for row in rows]


def export_bip329_labels(conn, workspace_ref, profile_ref, file_path, wallet_ref=None):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    wallet_clause = "AND wallet_id = ?" if wallet else ""
    params = [profile["id"]]
    if wallet:
        params.append(wallet["id"])
    rows = conn.execute(
        f"""
        SELECT record_type, ref, label, origin, spendable, data_json
        FROM bip329_labels
        WHERE profile_id = ? {wallet_clause}
        ORDER BY created_at ASC
        """,
        params,
    ).fetchall()
    output_lines = []
    for row in rows:
        payload = {"type": row["record_type"], "ref": row["ref"]}
        if row["label"] is not None:
            payload["label"] = row["label"]
        if row["origin"] is not None:
            payload["origin"] = row["origin"]
        if row["spendable"] is not None:
            payload["spendable"] = bool(row["spendable"])
        payload.update(json.loads(row["data_json"] or "{}"))
        output_lines.append(json.dumps(payload, ensure_ascii=True))
    export_path = os.path.abspath(file_path)
    with open(export_path, "w", encoding="utf-8") as handle:
        if output_lines:
            handle.write("\n".join(output_lines) + "\n")
    return {"file": export_path, "exported": len(output_lines)}


def available_quantity(lots):
    total = Decimal("0")
    for lot in lots:
        total += lot["quantity"]
    return total


def consume_lots(lots, quantity, algorithm):
    remaining = dec(quantity)
    cost_basis = Decimal("0")
    while remaining > 0:
        if not lots:
            raise AppError("Not enough lots to consume")
        lot = lots[0] if algorithm == "FIFO" else lots[-1]
        take = min(remaining, lot["quantity"])
        cost_basis += take * lot["unit_cost"]
        lot["quantity"] -= take
        remaining -= take
        if lot["quantity"] <= Decimal("0"):
            if algorithm == "FIFO":
                lots.pop(0)
            else:
                lots.pop()
    return cost_basis


def latest_rates_for_profile(conn, profile_id):
    rows = conn.execute(
        """
        SELECT asset, fiat_rate, fiat_value, amount
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        ORDER BY occurred_at DESC, created_at DESC
        """,
        (profile_id,),
    ).fetchall()
    rates = {}
    for row in rows:
        asset = row["asset"]
        if asset in rates:
            continue
        if row["fiat_rate"] is not None:
            rates[asset] = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rates[asset] = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
    return rates


# -- rates cache -------------------------------------------------------------

SUPPORTED_RATE_PAIRS = ("BTC-USD", "BTC-EUR")
_COINGECKO_VS = {"USD": "usd", "EUR": "eur"}
_COINGECKO_COIN = {"BTC": "bitcoin"}


def _normalize_rate_pair(pair):
    if not pair:
        raise AppError("Pair is required", code="validation")
    raw = pair.strip().upper().replace("/", "-")
    if "-" not in raw:
        raise AppError(
            f"Invalid pair '{pair}'",
            code="validation",
            hint="Use <ASSET>-<FIAT>, e.g. BTC-USD",
        )
    asset, _, fiat = raw.partition("-")
    if not asset or not fiat:
        raise AppError(f"Invalid pair '{pair}'", code="validation")
    return f"{asset}-{fiat}"


def _require_supported_pair(pair):
    normalized = _normalize_rate_pair(pair)
    if normalized not in SUPPORTED_RATE_PAIRS:
        raise AppError(
            f"Pair '{normalized}' is not supported",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return normalized


def _rate_pair_parts(pair):
    asset, _, fiat = pair.partition("-")
    return asset, fiat


def _transaction_rate_pair(asset, fiat_currency):
    asset_code = str(asset or "").strip().upper()
    fiat_code = str(fiat_currency or "").strip().upper()
    if not asset_code or not fiat_code:
        return None
    asset_aliases = {
        "LBTC": "BTC",
    }
    asset_code = asset_aliases.get(asset_code, asset_code)
    pair = f"{asset_code}-{fiat_code}"
    if pair not in SUPPORTED_RATE_PAIRS:
        return None
    return pair


def upsert_rate(conn, pair, timestamp, rate, source, fetched_at=None):
    normalized = _normalize_rate_pair(pair)
    ts = _iso_z(_parse_iso_datetime(timestamp, "rate_timestamp"))
    fetched = fetched_at or _iso_z(datetime.now(timezone.utc))
    conn.execute(
        """
        INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(pair, timestamp, source) DO UPDATE SET
            rate = excluded.rate,
            fetched_at = excluded.fetched_at
        """,
        (normalized, ts, float(rate), source, fetched),
    )
    return {
        "pair": normalized,
        "timestamp": ts,
        "rate": float(rate),
        "source": source,
        "fetched_at": fetched,
    }


def get_latest_rate(conn, pair):
    normalized = _normalize_rate_pair(pair)
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at
        FROM rates_cache
        WHERE pair = ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized,),
    ).fetchone()
    if not row:
        raise AppError(
            f"No cached rate for pair '{normalized}'",
            code="not_found",
            hint="Run `kassiber rates sync` first",
        )
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
    }


def get_rate_range(conn, pair, start=None, end=None, limit=None):
    normalized = _normalize_rate_pair(pair)
    sql = "SELECT pair, timestamp, rate, source, fetched_at FROM rates_cache WHERE pair = ?"
    params = [normalized]
    if start:
        start_dt = _parse_iso_datetime(start, "start")
        sql += " AND timestamp >= ?"
        params.append(_iso_z(start_dt))
    if end:
        end_dt = _parse_iso_datetime(end, "end")
        sql += " AND timestamp <= ?"
        params.append(_iso_z(end_dt))
    sql += " ORDER BY timestamp ASC"
    if limit:
        sql += " LIMIT ?"
        params.append(int(limit))
    rows = conn.execute(sql, params).fetchall()
    return [
        {
            "pair": r["pair"],
            "timestamp": r["timestamp"],
            "rate": r["rate"],
            "source": r["source"],
            "fetched_at": r["fetched_at"],
        }
        for r in rows
    ]


def get_cached_rate_at_or_before(conn, pair, occurred_at):
    normalized = _require_supported_pair(pair)
    occurred_ts = _iso_z(_parse_iso_datetime(occurred_at, "occurred_at"))
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at
        FROM rates_cache
        WHERE pair = ? AND timestamp <= ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized, occurred_ts),
    ).fetchone()
    if not row:
        return None
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
    }


def list_cached_pairs(conn):
    rows = conn.execute(
        """
        SELECT pair,
               COUNT(*) AS sample_count,
               MIN(timestamp) AS first_timestamp,
               MAX(timestamp) AS last_timestamp
        FROM rates_cache
        GROUP BY pair
        ORDER BY pair ASC
        """
    ).fetchall()
    known = {p: None for p in SUPPORTED_RATE_PAIRS}
    for row in rows:
        known[row["pair"]] = {
            "sample_count": int(row["sample_count"]),
            "first_timestamp": row["first_timestamp"],
            "last_timestamp": row["last_timestamp"],
        }
    result = []
    for pair in SUPPORTED_RATE_PAIRS:
        detail = known.get(pair)
        result.append(
            {
                "pair": pair,
                "supported": True,
                "cached": detail is not None,
                "sample_count": detail["sample_count"] if detail else 0,
                "first_timestamp": detail["first_timestamp"] if detail else None,
                "last_timestamp": detail["last_timestamp"] if detail else None,
            }
        )
    # Report any non-canonical pairs cached from manual `rates set`.
    for pair, detail in known.items():
        if pair in SUPPORTED_RATE_PAIRS:
            continue
        if detail is None:
            continue
        result.append(
            {
                "pair": pair,
                "supported": False,
                "cached": True,
                "sample_count": detail["sample_count"],
                "first_timestamp": detail["first_timestamp"],
                "last_timestamp": detail["last_timestamp"],
            }
        )
    return result


def _coingecko_market_chart(coin_id, vs, days):
    url = (
        "https://api.coingecko.com/api/v3/coins/"
        f"{coin_id}/market_chart?vs_currency={vs}&days={int(days)}"
    )
    payload = http_get_json(url, timeout=30)
    prices = payload.get("prices") if isinstance(payload, dict) else None
    if not isinstance(prices, list):
        raise AppError(
            "CoinGecko response did not contain a prices array",
            code="upstream_error",
            retryable=True,
        )
    out = []
    for entry in prices:
        if not isinstance(entry, list) or len(entry) < 2:
            continue
        ms, value = entry[0], entry[1]
        try:
            ts = datetime.fromtimestamp(float(ms) / 1000.0, tz=timezone.utc)
            rate = float(value)
        except (TypeError, ValueError):
            continue
        out.append((_iso_z(ts.replace(microsecond=0)), rate))
    return out


def fetch_rates_coingecko(pair, days=30):
    normalized = _require_supported_pair(pair)
    asset, fiat = _rate_pair_parts(normalized)
    coin_id = _COINGECKO_COIN.get(asset)
    vs = _COINGECKO_VS.get(fiat)
    if not coin_id or not vs:
        raise AppError(
            f"Pair '{normalized}' has no CoinGecko mapping",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return _coingecko_market_chart(coin_id, vs, days)


def sync_rates(conn, pair=None, days=30, source="coingecko"):
    if source != "coingecko":
        raise AppError(
            f"Unknown rate source '{source}'",
            code="validation",
            hint="Supported sources: coingecko",
        )
    if pair:
        pairs = [_require_supported_pair(pair)]
    else:
        pairs = list(SUPPORTED_RATE_PAIRS)
    fetched_at = _iso_z(datetime.now(timezone.utc))
    summary = []
    for p in pairs:
        samples = fetch_rates_coingecko(p, days=days)
        inserted = 0
        for ts, rate in samples:
            upsert_rate(conn, p, ts, rate, source, fetched_at=fetched_at)
            inserted += 1
        conn.commit()
        summary.append(
            {
                "pair": p,
                "source": source,
                "samples": inserted,
                "days": int(days),
                "fetched_at": fetched_at,
            }
        )
    return summary


def set_manual_rate(conn, pair, timestamp, rate, source="manual"):
    normalized = _normalize_rate_pair(pair)
    try:
        value = float(rate)
    except (TypeError, ValueError) as exc:
        raise AppError(f"Invalid rate '{rate}'", code="validation") from exc
    if value <= 0:
        raise AppError("Rate must be positive", code="validation")
    row = upsert_rate(conn, normalized, timestamp, value, source)
    conn.commit()
    return row


def auto_price_transactions_from_rates_cache(conn, profile):
    tx_rows = conn.execute(
        """
        SELECT id, occurred_at, asset, amount, fiat_currency, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND fiat_rate IS NULL AND fiat_value IS NULL
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()
    auto_priced = 0
    for row in tx_rows:
        pair = _transaction_rate_pair(row["asset"], row["fiat_currency"] or profile["fiat_currency"])
        if pair is None:
            continue
        cached_rate = get_cached_rate_at_or_before(conn, pair, row["occurred_at"])
        if cached_rate is None:
            continue
        rate = dec(cached_rate["rate"])
        fiat_value = rate * msat_to_btc(row["amount"]) if row["amount"] > 0 else None
        conn.execute(
            "UPDATE transactions SET fiat_rate = ?, fiat_value = ? WHERE id = ?",
            (float(rate), float(fiat_value) if fiat_value is not None else None, row["id"]),
        )
        auto_priced += 1
    return auto_priced


def rp2_wallet_state(profile, wallet, asset, rows, configuration):
    modules = get_rp2_modules()
    TransactionSet = modules["TransactionSet"]
    InTransaction = modules["InTransaction"]
    OutTransaction = modules["OutTransaction"]
    InputData = modules["InputData"]
    compute_tax = modules["compute_tax"]
    in_transactions = TransactionSet(configuration, "IN", asset)
    out_transactions = TransactionSet(configuration, "OUT", asset)
    intra_transactions = TransactionSet(configuration, "INTRA", asset)
    holder = profile["label"]
    total_available = Decimal("0")
    priced_available = Decimal("0")
    quarantines = []
    row_index = 1
    row_by_id = {row["id"]: row for row in rows}
    for row in rows:
        amount = msat_to_btc(row["amount"])
        fee = msat_to_btc(row["fee"])
        description = row["note"] or row["description"] or row["kind"] or row["id"]
        if row["direction"] == "inbound":
            total_available += amount
            spot_price = rp2_spot_price(row, amount)
            if spot_price is None:
                quarantines.append(
                    rp2_quarantine(
                        profile,
                        row,
                        "missing_spot_price",
                        {
                            "wallet": wallet["label"],
                            "asset": asset,
                            "direction": row["direction"],
                            "required_for": "acquisition",
                        },
                    )
                )
                continue
            fiat_value = dec(row["fiat_value"]) if row["fiat_value"] is not None else amount * spot_price
            in_transactions.add_entry(
                InTransaction(
                    configuration=configuration,
                    timestamp=row["occurred_at"],
                    asset=asset,
                    exchange=wallet["label"],
                    holder=holder,
                    transaction_type="BUY",
                    spot_price=rp2_decimal(spot_price),
                    crypto_in=rp2_decimal(amount),
                    fiat_in_no_fee=rp2_decimal(fiat_value),
                    fiat_in_with_fee=rp2_decimal(fiat_value),
                    fiat_fee=rp2_decimal(0),
                    row=row_index,
                    unique_id=row["id"],
                    notes=description,
                )
            )
            priced_available += amount
            row_index += 1
            continue
        needed = amount + fee
        if needed <= 0:
            continue
        if total_available < needed:
            quarantines.append(
                rp2_quarantine(
                    profile,
                    row,
                    "insufficient_lots",
                    {
                        "wallet": wallet["label"],
                        "asset": asset,
                        "required": float(needed),
                        "available": float(total_available),
                    },
                )
            )
            continue
        if priced_available < needed:
            quarantines.append(
                rp2_quarantine(
                    profile,
                    row,
                    "missing_cost_basis",
                    {
                        "wallet": wallet["label"],
                        "asset": asset,
                        "required": float(needed),
                        "priced_available": float(priced_available),
                    },
                )
            )
            continue
        spot_price = rp2_spot_price(row, amount if amount > 0 else fee)
        if spot_price is None:
            quarantines.append(
                rp2_quarantine(
                    profile,
                    row,
                    "missing_spot_price",
                    {
                        "wallet": wallet["label"],
                        "asset": asset,
                        "direction": row["direction"],
                        "required_for": "disposal",
                    },
                )
            )
            continue
        fiat_out_no_fee = dec(row["fiat_value"]) if row["fiat_value"] is not None else amount * spot_price
        out_transactions.add_entry(
            OutTransaction(
                configuration=configuration,
                timestamp=row["occurred_at"],
                asset=asset,
                exchange=wallet["label"],
                holder=holder,
                transaction_type="SELL" if amount > 0 else "FEE",
                spot_price=rp2_decimal(spot_price),
                crypto_out_no_fee=rp2_decimal(amount),
                crypto_fee=rp2_decimal(fee),
                fiat_out_no_fee=rp2_decimal(fiat_out_no_fee) if amount > 0 else None,
                fiat_fee=rp2_decimal(fee * spot_price),
                row=row_index,
                unique_id=row["id"],
                notes=description,
            )
        )
        total_available -= needed
        priced_available -= needed
        row_index += 1
    if in_transactions.count == 0:
        return None, quarantines, row_by_id
    input_data = InputData(
        asset=asset,
        unfiltered_in_transaction_set=in_transactions,
        unfiltered_out_transaction_set=out_transactions,
        unfiltered_intra_transaction_set=intra_transactions,
    )
    try:
        computed_data = compute_tax(configuration, build_rp2_accounting_engine(profile), input_data)
    except Exception as exc:
        raise AppError(f"RP2 tax calculation failed for wallet '{wallet['label']}' asset '{asset}': {exc}") from exc
    return computed_data, quarantines, row_by_id


def append_rp2_journal_entries(entries, computed_data, wallet, profile, row_by_id):
    altbestand = wallet_is_altbestand(wallet)
    for transaction in computed_data.in_transaction_set:
        source_row = row_by_id.get(transaction.unique_id)
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": transaction.unique_id,
                "wallet_id": wallet["id"],
                "account_id": wallet["wallet_account_id"],
                "occurred_at": source_row["occurred_at"] if source_row else transaction.timestamp.isoformat(),
                "entry_type": "acquisition",
                "asset": transaction.asset,
                "quantity": dec(transaction.crypto_in),
                "fiat_value": dec(transaction.fiat_in_with_fee),
                "unit_cost": dec(transaction.fiat_in_with_fee) / dec(transaction.crypto_in),
                "cost_basis": None,
                "proceeds": None,
                "gain_loss": None,
                "description": transaction.notes or (source_row["description"] if source_row else "Inbound transaction"),
            }
        )
    realized_by_event = {}
    for gain_loss in computed_data.gain_loss_set:
        taxable_event = gain_loss.taxable_event
        event = realized_by_event.setdefault(
            taxable_event.internal_id,
            {
                "transaction_id": taxable_event.unique_id,
                "occurred_at": row_by_id[taxable_event.unique_id]["occurred_at"] if taxable_event.unique_id in row_by_id else taxable_event.timestamp.isoformat(),
                "entry_type": "fee" if taxable_event.transaction_type.value == "FEE" else "disposal",
                "asset": taxable_event.asset,
                "quantity": Decimal("0"),
                "fiat_value": Decimal("0"),
                "cost_basis": Decimal("0"),
                "proceeds": Decimal("0"),
                "gain_loss": Decimal("0"),
                "description": taxable_event.notes or (
                    row_by_id[taxable_event.unique_id]["description"] if taxable_event.unique_id in row_by_id else "Outbound transaction"
                ),
            },
        )
        event["quantity"] += dec(gain_loss.crypto_amount)
        event["cost_basis"] += dec(gain_loss.fiat_cost_basis)
        event["proceeds"] += dec(gain_loss.taxable_event_fiat_amount_with_fee_fraction)
        event["gain_loss"] += dec(gain_loss.fiat_gain)
    for event in realized_by_event.values():
        description = event["description"]
        proceeds = event["proceeds"]
        cost_basis = event["cost_basis"]
        gain_loss = event["gain_loss"]
        if altbestand:
            description = f"{description} [Altbestand tax-free]"
            if event["entry_type"] == "fee":
                proceeds = Decimal("0")
                cost_basis = Decimal("0")
            else:
                cost_basis = proceeds
            gain_loss = Decimal("0")
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": event["transaction_id"],
                "wallet_id": wallet["id"],
                "account_id": wallet["wallet_account_id"],
                "occurred_at": event["occurred_at"],
                "entry_type": event["entry_type"],
                "asset": event["asset"],
                "quantity": -event["quantity"],
                "fiat_value": proceeds,
                "unit_cost": Decimal("0"),
                "cost_basis": cost_basis,
                "proceeds": proceeds,
                "gain_loss": gain_loss,
                "description": description,
            }
        )


def accumulate_rp2_holdings(account_holdings, wallet_holdings, computed_data, wallet):
    for transaction in computed_data.in_transaction_set:
        sold_percent = dec(computed_data.get_in_lot_sold_percentage(transaction))
        remaining_ratio = Decimal("1") - sold_percent
        if remaining_ratio <= 0:
            continue
        quantity = dec(transaction.crypto_in) * remaining_ratio
        if quantity <= 0:
            continue
        cost_basis = dec(transaction.fiat_in_with_fee) * remaining_ratio
        account_key = (
            wallet["wallet_account_id"],
            wallet["account_code"],
            wallet["account_label"],
            transaction.asset,
        )
        wallet_key = (
            wallet["id"],
            wallet["label"],
            wallet["account_code"],
            transaction.asset,
        )
        account_holdings[account_key]["quantity"] += quantity
        account_holdings[account_key]["cost_basis"] += cost_basis
        wallet_holdings[wallet_key]["quantity"] += quantity
        wallet_holdings[wallet_key]["cost_basis"] += cost_basis


def build_ledger_state(conn, profile):
    rows = conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind,
            w.account_id AS wallet_account_id,
            w.config_json AS config_json,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    wallet_labels = {row["wallet_label"] for row in rows}
    assets = {row["asset"] for row in rows}
    if not rows:
        return {
            "entries": [],
            "quarantines": [],
            "account_holdings": defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}),
            "wallet_holdings": defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}),
            "latest_rates": latest_rates_for_profile(conn, profile["id"]),
        }
    configuration, configuration_path = make_rp2_configuration(profile, wallet_labels, assets)
    entries = []
    quarantines = []
    account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    rates = latest_rates_for_profile(conn, profile["id"])
    try:
        grouped_rows = defaultdict(list)
        wallet_refs = {}
        for row in rows:
            grouped_rows[(row["wallet_id"], row["asset"])].append(row)
            wallet_config = json.loads(row["config_json"] or "{}")
            wallet_refs[row["wallet_id"]] = {
                "id": row["wallet_id"],
                "label": row["wallet_label"],
                "wallet_account_id": row["wallet_account_id"],
                "account_code": row["account_code"],
                "account_label": row["account_label"],
                "altbestand": wallet_config.get("altbestand", False),
            }
        for (wallet_id, asset), wallet_rows in grouped_rows.items():
            computed_data, wallet_quarantines, row_by_id = rp2_wallet_state(
                profile,
                wallet_refs[wallet_id],
                asset,
                wallet_rows,
                configuration,
            )
            quarantines.extend(wallet_quarantines)
            if computed_data is None:
                continue
            append_rp2_journal_entries(entries, computed_data, wallet_refs[wallet_id], profile, row_by_id)
            accumulate_rp2_holdings(account_holdings, wallet_holdings, computed_data, wallet_refs[wallet_id])
    finally:
        try:
            os.unlink(configuration_path)
        except OSError:
            pass
    return {
        "entries": entries,
        "quarantines": quarantines,
        "account_holdings": account_holdings,
        "wallet_holdings": wallet_holdings,
        "latest_rates": rates,
    }


def process_journals(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    auto_priced = auto_price_transactions_from_rates_cache(conn, profile)
    state = build_ledger_state(conn, profile)
    conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile["id"],))
    conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile["id"],))
    created_at = now_iso()
    for entry in state["entries"]:
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                entry["id"],
                entry["workspace_id"],
                entry["profile_id"],
                entry["transaction_id"],
                entry["wallet_id"],
                entry["account_id"],
                entry["occurred_at"],
                entry["entry_type"],
                entry["asset"],
                btc_to_msat(entry["quantity"]),
                float(entry["fiat_value"]),
                float(entry["unit_cost"]),
                float(entry["cost_basis"]) if entry["cost_basis"] is not None else None,
                float(entry["proceeds"]) if entry["proceeds"] is not None else None,
                float(entry["gain_loss"]) if entry["gain_loss"] is not None else None,
                entry["description"],
                created_at,
            ),
        )
    for quarantine in state["quarantines"]:
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                quarantine["transaction_id"],
                quarantine["workspace_id"],
                quarantine["profile_id"],
                quarantine["reason"],
                quarantine["detail_json"],
                created_at,
            ),
        )
    tx_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    conn.execute(
        "UPDATE profiles SET last_processed_at = ?, last_processed_tx_count = ? WHERE id = ?",
        (created_at, tx_count, profile["id"]),
    )
    conn.commit()
    return {
        "profile": profile["label"],
        "entries_created": len(state["entries"]),
        "quarantined": len(state["quarantines"]),
        "auto_priced": auto_priced,
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }


DEFAULT_EVENTS_LIMIT = 100
MAX_EVENTS_LIMIT = 1000


def _encode_event_cursor(row):
    token = f"{row['occurred_at']}|{row['created_at']}|{row['id']}"
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_event_cursor(cursor):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        occurred_at, created_at, event_id = decoded.split("|", 2)
        return {"occurred_at": occurred_at, "created_at": created_at, "id": event_id}
    except (ValueError, UnicodeDecodeError, binascii.Error) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it.",
        ) from exc


def list_journal_events(
    conn,
    workspace_ref,
    profile_ref,
    wallet=None,
    account=None,
    asset=None,
    entry_type=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_EVENTS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_EVENTS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_EVENTS_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_EVENTS_LIMIT}.",
        )

    where = ["je.profile_id = ?"]
    params = [profile["id"]]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None

    if wallet:
        wallet_row = resolve_wallet(conn, profile["id"], wallet)
        where.append("je.wallet_id = ?")
        params.append(wallet_row["id"])
    if account:
        account_row = resolve_account(conn, profile["id"], account)
        where.append("je.account_id = ?")
        params.append(account_row["id"])
    if asset:
        where.append("upper(je.asset) = ?")
        params.append(asset.upper())
    if entry_type:
        where.append("lower(je.entry_type) = ?")
        params.append(entry_type.lower())
    if start_ts:
        where.append("je.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("je.occurred_at <= ?")
        params.append(end_ts)

    cursor_data = _decode_event_cursor(cursor)
    if cursor_data:
        where.append(
            "(je.occurred_at < ? OR "
            "(je.occurred_at = ? AND je.created_at < ?) OR "
            "(je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            je.id,
            je.occurred_at,
            je.created_at,
            je.transaction_id,
            je.wallet_id,
            w.label AS wallet,
            je.account_id,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            je.unit_cost,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    events = []
    for row in page:
        event = dict(row)
        event["quantity_msat"] = int(event["quantity"])
        event["quantity"] = float(msat_to_btc(event["quantity"]))
        events.append(event)
    next_cursor = _encode_event_cursor(page[-1]) if has_more and page else None

    return {
        "events": events,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def get_journal_event(conn, workspace_ref, profile_ref, event_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        """
        SELECT
            je.*,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            t.counterparty AS transaction_counterparty,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.id = ?
        """,
        (profile["id"], event_id),
    ).fetchone()
    if not row:
        raise AppError(
            f"Journal event '{event_id}' not found",
            code="not_found",
            hint="Run `kassiber journals events list` to find valid event ids.",
        )
    event = dict(row)
    event["quantity_msat"] = int(event["quantity"])
    event["quantity"] = float(msat_to_btc(event["quantity"]))
    return event


def list_journal_entries(conn, workspace_ref, profile_ref, limit=200):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            je.id,
            je.occurred_at,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at DESC, je.created_at DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    results = []
    for row in rows:
        entry = dict(row)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        results.append(entry)
    return results


def list_quarantines(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
            w.label AS wallet,
            t.asset,
            t.amount,
            t.fee,
            q.reason,
            q.detail_json
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY t.occurred_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        detail = json.loads(row["detail_json"] or "{}")
        output.append(
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "wallet": row["wallet"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "reason": row["reason"],
                "detail": detail,
            }
        )
    return output


def show_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    row = conn.execute(
        """
        SELECT q.transaction_id, q.reason, q.detail_json, q.created_at,
               w.label AS wallet, t.external_id, t.occurred_at, t.asset,
               t.amount, t.fee, t.fiat_rate, t.fiat_value, t.direction, t.excluded
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ? AND q.transaction_id = ?
        """,
        (profile["id"], tx["id"]),
    ).fetchone()
    if not row:
        raise AppError(
            f"Transaction '{tx_ref}' has no active quarantine",
            code="not_found",
            hint="Only transactions flagged during `journals process` appear here.",
        )
    return {
        "transaction_id": row["transaction_id"],
        "external_id": row["external_id"] or "",
        "wallet": row["wallet"],
        "occurred_at": row["occurred_at"],
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": float(msat_to_btc(row["amount"])),
        "amount_msat": int(row["amount"]),
        "fee": float(msat_to_btc(row["fee"])),
        "fee_msat": int(row["fee"]),
        "fiat_rate": row["fiat_rate"],
        "fiat_value": row["fiat_value"],
        "excluded": bool(row["excluded"]),
        "reason": row["reason"],
        "detail": json.loads(row["detail_json"] or "{}"),
        "quarantined_at": row["created_at"],
    }


def _ensure_quarantined(conn, profile_id, transaction_id):
    row = conn.execute(
        "SELECT reason FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile_id, transaction_id),
    ).fetchone()
    if not row:
        raise AppError(
            "Transaction is not quarantined",
            code="not_found",
            hint="Run `kassiber journals quarantined` to see active entries.",
        )
    return row["reason"]


def resolve_quarantine_price_override(
    conn, workspace_ref, profile_ref, tx_ref, fiat_rate=None, fiat_value=None
):
    if fiat_rate is None and fiat_value is None:
        raise AppError(
            "Provide at least one of --fiat-rate or --fiat-value",
            code="validation",
        )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    new_rate = dec(fiat_rate) if fiat_rate is not None else None
    new_value = dec(fiat_value) if fiat_value is not None else None
    amount = abs(msat_to_btc(tx["amount"]))
    if new_rate is None and new_value is not None and amount > 0:
        new_rate = new_value / amount
    if new_value is None and new_rate is not None and amount > 0:
        new_value = new_rate * amount
    if new_rate is not None and new_rate <= 0:
        raise AppError("--fiat-rate must be positive", code="validation")
    if new_value is not None and new_value < 0:
        raise AppError("--fiat-value must not be negative", code="validation")
    conn.execute(
        "UPDATE transactions SET fiat_rate = ?, fiat_value = ? WHERE id = ?",
        (
            float(new_rate) if new_rate is not None else None,
            float(new_value) if new_value is not None else None,
            tx["id"],
        ),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "price-override",
        "fiat_rate": float(new_rate) if new_rate is not None else None,
        "fiat_value": float(new_value) if new_value is not None else None,
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def resolve_quarantine_exclude(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "UPDATE transactions SET excluded = 1 WHERE id = ?",
        (tx["id"],),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "exclude",
        "excluded": True,
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def clear_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "clear",
        "note": "Run `kassiber journals process` to re-evaluate.",
    }


def require_processed_journals(conn, profile):
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    if not profile["last_processed_at"] or current_count != profile["last_processed_tx_count"]:
        raise AppError("Reports require fresh journals. Run `kassiber journals process` first.")


def report_balance_sheet(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    state = build_ledger_state(conn, profile)
    rows = []
    for (account_id, account_code, account_label, asset), value in sorted(
        state["account_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        rows.append(
            {
                "account": account_code or account_label,
                "asset": asset,
                "quantity": float(quantity),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_portfolio_summary(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    state = build_ledger_state(conn, profile)
    rows = []
    for (wallet_id, wallet_label, account_code, asset), value in sorted(
        state["wallet_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        avg_cost = cost_basis / quantity if quantity else Decimal("0")
        rows.append(
            {
                "wallet": wallet_label,
                "account": account_code,
                "asset": asset,
                "quantity": float(quantity),
                "avg_cost": float(avg_cost),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_capital_gains(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id,
            je.entry_type,
            je.asset,
            ABS(je.quantity) AS quantity,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        WHERE je.profile_id = ? AND je.entry_type IN ('disposal', 'fee')
        ORDER BY je.occurred_at ASC
        """,
        (profile["id"],),
    ).fetchall()
    results = []
    for row in rows:
        entry = dict(row)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        results.append(entry)
    return results


def report_journal_entries(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    return list_journal_entries(conn, profile["workspace_id"], profile["id"], limit=1000)


INTERVAL_CHOICES = ("hour", "day", "week", "month")


def _floor_to_interval(dt, interval):
    if interval == "hour":
        return dt.replace(minute=0, second=0, microsecond=0)
    if interval == "day":
        return dt.replace(hour=0, minute=0, second=0, microsecond=0)
    if interval == "week":
        floored = dt.replace(hour=0, minute=0, second=0, microsecond=0)
        return floored - timedelta(days=floored.weekday())
    if interval == "month":
        return dt.replace(day=1, hour=0, minute=0, second=0, microsecond=0)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def _next_interval(dt, interval):
    if interval == "hour":
        return dt + timedelta(hours=1)
    if interval == "day":
        return dt + timedelta(days=1)
    if interval == "week":
        return dt + timedelta(days=7)
    if interval == "month":
        if dt.month == 12:
            return dt.replace(year=dt.year + 1, month=1)
        return dt.replace(month=dt.month + 1)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def report_balance_history(
    conn,
    workspace_ref,
    profile_ref,
    interval="day",
    start=None,
    end=None,
    wallet_ref=None,
    account_ref=None,
    asset=None,
):
    if interval not in INTERVAL_CHOICES:
        raise AppError(
            f"Unsupported interval '{interval}'",
            code="validation",
            hint=f"Choose one of: {', '.join(INTERVAL_CHOICES)}",
        )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    start_dt = _parse_iso_datetime(start, "start")
    end_dt = _parse_iso_datetime(end, "end")
    if start_dt and end_dt and start_dt > end_dt:
        raise AppError("--start must not be after --end", code="validation")

    sql = """
        SELECT
            je.occurred_at,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis
        FROM journal_entries je
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
    """
    params = [profile["id"]]
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        sql += " AND je.wallet_id = ?"
        params.append(wallet["id"])
    if account_ref:
        sql += " AND (a.code = ? OR a.label = ? OR a.id = ?)"
        params.extend([account_ref, account_ref, account_ref])
    if asset:
        sql += " AND je.asset = ?"
        params.append(asset)
    sql += " ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC"
    rows = conn.execute(sql, params).fetchall()
    rate_rows = conn.execute(
        """
        SELECT occurred_at, asset, amount, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL)
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()

    if not rows and not (start_dt and end_dt):
        return []

    events = []
    for row in rows:
        row_dt = _parse_iso_datetime(row["occurred_at"], "occurred_at")
        events.append(
            (
                row_dt,
                row["asset"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
    rate_events = []
    for row in rate_rows:
        rate = None
        if row["fiat_rate"] is not None:
            rate = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rate = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
        if rate is None:
            continue
        rate_events.append((_parse_iso_datetime(row["occurred_at"], "occurred_at"), row["asset"], rate))

    first_event_dt = events[0][0] if events else None
    range_start = start_dt or first_event_dt or datetime.now(timezone.utc)
    range_end = end_dt or datetime.now(timezone.utc)
    if range_start > range_end:
        return []

    cumulative = defaultdict(lambda: Decimal("0"))
    cumulative_fiat = defaultdict(lambda: Decimal("0"))
    event_idx = 0
    rate_idx = 0
    current_rates = {}
    bucket_start = _floor_to_interval(range_start, interval)
    end_cap = _floor_to_interval(range_end, interval)

    results = []
    while bucket_start <= end_cap:
        bucket_end = _next_interval(bucket_start, interval)
        while event_idx < len(events) and events[event_idx][0] < bucket_end:
            _, ev_asset, ev_qty, ev_fiat, ev_cost_basis = events[event_idx]
            cumulative[ev_asset] += ev_qty
            if ev_qty >= 0:
                cumulative_fiat[ev_asset] += ev_fiat
            else:
                cumulative_fiat[ev_asset] -= ev_cost_basis
            event_idx += 1
        while rate_idx < len(rate_events) and rate_events[rate_idx][0] < bucket_end:
            _, rate_asset, rate = rate_events[rate_idx]
            current_rates[rate_asset] = rate
            rate_idx += 1
        emitted_assets = set(cumulative.keys()) if asset is None else {asset}
        for ev_asset in sorted(emitted_assets):
            qty = cumulative.get(ev_asset, Decimal("0"))
            if qty == 0 and asset is None:
                continue
            rate = current_rates.get(ev_asset, Decimal("0"))
            results.append(
                {
                    "period_start": _iso_z(bucket_start),
                    "period_end": _iso_z(bucket_end - timedelta(seconds=1)),
                    "asset": ev_asset,
                    "quantity": float(qty),
                    "cumulative_cost_basis": float(cumulative_fiat.get(ev_asset, Decimal("0"))),
                    "market_value": float(qty * rate),
                }
            )
        bucket_start = bucket_end
    return results


def show_status(conn, data_root):
    workspace_id = get_setting(conn, "context_workspace")
    profile_id = get_setting(conn, "context_profile")
    workspace = conn.execute("SELECT label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone() if workspace_id else None
    profile = conn.execute("SELECT label FROM profiles WHERE id = ?", (profile_id,)).fetchone() if profile_id else None
    effective_data_root = resolve_effective_data_root(data_root)
    state_root = resolve_effective_state_root(data_root)
    counts = {
        "workspaces": conn.execute("SELECT COUNT(*) AS count FROM workspaces").fetchone()["count"],
        "profiles": conn.execute("SELECT COUNT(*) AS count FROM profiles").fetchone()["count"],
        "accounts": conn.execute("SELECT COUNT(*) AS count FROM accounts").fetchone()["count"],
        "wallets": conn.execute("SELECT COUNT(*) AS count FROM wallets").fetchone()["count"],
        "transactions": conn.execute("SELECT COUNT(*) AS count FROM transactions").fetchone()["count"],
        "journal_entries": conn.execute("SELECT COUNT(*) AS count FROM journal_entries").fetchone()["count"],
        "quarantines": conn.execute("SELECT COUNT(*) AS count FROM journal_quarantines").fetchone()["count"],
    }
    return {
        "version": __version__,
        "schema_version": SCHEMA_VERSION,
        "auth": {"mode": "local", "authenticated": True},
        "state_root": str(state_root),
        "data_root": str(effective_data_root),
        "database": str(resolve_database_path(effective_data_root)),
        "config_root": str(resolve_config_root(data_root)),
        "settings_file": str(resolve_settings_path(data_root)),
        "exports_root": str(resolve_exports_root(data_root)),
        "current_workspace": workspace["label"] if workspace else "",
        "current_profile": profile["label"] if profile else "",
        **counts,
    }


def get_profile_details(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    current_profile_id = get_setting(conn, "context_profile")
    current_workspace_id = get_setting(conn, "context_workspace")
    return {
        "id": profile["id"],
        "workspace_id": profile["workspace_id"],
        "workspace_label": workspace["label"],
        "label": profile["label"],
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "tax_long_term_days": profile["tax_long_term_days"],
        "gains_algorithm": profile["gains_algorithm"],
        "last_processed_at": profile["last_processed_at"],
        "last_processed_tx_count": profile["last_processed_tx_count"],
        "created_at": profile["created_at"],
        "is_current": profile["id"] == current_profile_id and profile["workspace_id"] == current_workspace_id,
    }


def update_profile(conn, workspace_ref, profile_ref, updates):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)

    new_label = updates.get("label")
    new_fiat = updates.get("fiat_currency")
    new_country = updates.get("tax_country")
    new_long_term = updates.get("tax_long_term_days")
    new_algo = updates.get("gains_algorithm")

    merged_fiat = new_fiat if new_fiat is not None else profile["fiat_currency"]
    merged_country = new_country if new_country is not None else profile["tax_country"]
    merged_long_term = new_long_term if new_long_term is not None else profile["tax_long_term_days"]
    merged_algo = new_algo if new_algo is not None else profile["gains_algorithm"]
    merged_label = new_label if new_label is not None else profile["label"]

    if new_long_term is not None and new_long_term < 0:
        raise AppError(
            "Tax long-term days cannot be negative",
            code="validation",
            hint="Use a non-negative integer; pass 0 to treat every disposal as short-term.",
        )
    if new_algo is not None and new_algo.upper() not in RP2_ACCOUNTING_METHODS:
        raise AppError(
            f"Unsupported gains algorithm '{new_algo}'",
            code="validation",
            hint=f"Choose one of: {', '.join(RP2_ACCOUNTING_METHODS)}",
        )
    if new_country is not None and new_country not in supported_tax_countries():
        raise AppError(
            f"Unsupported tax country '{new_country}'",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(supported_tax_countries()))}",
        )
    try:
        build_tax_policy(
            {
                "fiat_currency": merged_fiat,
                "tax_country": merged_country,
                "tax_long_term_days": merged_long_term,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc), code="validation") from exc

    conn.execute(
        """
        UPDATE profiles
        SET label = ?, fiat_currency = ?, tax_country = ?, tax_long_term_days = ?, gains_algorithm = ?
        WHERE id = ?
        """,
        (
            merged_label,
            merged_fiat,
            merged_country,
            merged_long_term,
            merged_algo.upper(),
            profile["id"],
        ),
    )
    conn.commit()
    return get_profile_details(conn, workspace["label"], profile["id"])


def cmd_init(conn, args):
    init_app(conn)
    state_root = resolve_effective_state_root(args.data_root)
    effective_data_root = resolve_effective_data_root(args.data_root)
    emit(
        args,
        {
            "version": __version__,
            "state_root": str(state_root),
            "data_root": str(effective_data_root),
            "database": str(resolve_database_path(effective_data_root)),
            "config_root": str(resolve_config_root(args.data_root)),
            "settings_file": str(resolve_settings_path(args.data_root)),
            "exports_root": str(resolve_exports_root(args.data_root)),
            "env_file": str(args.env_file),
        },
    )


def cmd_status(conn, args):
    payload = show_status(conn, args.data_root)
    payload["default_backend"] = args.runtime_config["default_backend"]
    payload["env_file"] = args.runtime_config["env_file"]
    emit(args, payload)


def cmd_context_show(conn, args):
    workspace_id = get_setting(conn, "context_workspace")
    profile_id = get_setting(conn, "context_profile")
    workspace = conn.execute("SELECT id, label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone() if workspace_id else None
    profile = conn.execute("SELECT id, label FROM profiles WHERE id = ?", (profile_id,)).fetchone() if profile_id else None
    emit(
        args,
        {
            "workspace_id": workspace["id"] if workspace else "",
            "workspace_label": workspace["label"] if workspace else "",
            "profile_id": profile["id"] if profile else "",
            "profile_label": profile["label"] if profile else "",
        },
    )


def cmd_context_set(conn, args):
    if args.workspace:
        workspace = resolve_workspace(conn, args.workspace)
        set_setting(conn, "context_workspace", workspace["id"])
        if args.profile:
            profile = resolve_profile(conn, workspace["id"], args.profile)
            set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    elif args.profile:
        workspace = resolve_workspace(conn)
        profile = resolve_profile(conn, workspace["id"], args.profile)
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    else:
        raise AppError("Provide --workspace and/or --profile")
    cmd_context_show(conn, args)


def build_parser():
    parser = argparse.ArgumentParser(
        prog=APP_NAME,
        description="Open-source, local-first Bitcoin accounting CLI with multi-account and multi-wallet support.",
    )
    parser.add_argument("--data-root", default=DEFAULT_DATA_ROOT, help="Data directory for the local SQLite store")
    parser.add_argument(
        "--env-file",
        default=None,
        help=f"Path to a dotenv file that defines named sync backends (managed default: ~/.kassiber/config/{DEFAULT_ENV_FILENAME})",
    )
    parser.add_argument(
        "--format",
        choices=list(OUTPUT_FORMATS),
        default=None,
        help="Output format: table (default interactive), json (envelope), plain (text), csv (tabular)",
    )
    parser.add_argument(
        "--machine",
        action="store_true",
        help="Machine-readable mode: implies --format json, writes a structured envelope",
    )
    parser.add_argument(
        "--output",
        default=None,
        help="Write output to this file path instead of stdout (use '-' for stdout)",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Print a full traceback on error for diagnostics",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("init")
    sub.add_parser("status")

    backends = sub.add_parser("backends")
    backends_sub = backends.add_subparsers(dest="backends_command", required=True)
    backends_sub.add_parser("list")
    backends_sub.add_parser("kinds")

    backends_get = backends_sub.add_parser("get")
    backends_get.add_argument("name")

    backends_create = backends_sub.add_parser("create")
    backends_create.add_argument("name")
    backends_create.add_argument("--kind", required=True, choices=sorted(BACKEND_KINDS))
    backends_create.add_argument("--url", required=True)
    backends_create.add_argument("--chain", choices=["bitcoin", "liquid"])
    backends_create.add_argument("--network")
    backends_create.add_argument("--auth-header")
    backends_create.add_argument("--token")
    backends_create.add_argument("--batch-size", type=int)
    backends_create.add_argument("--timeout", type=int)
    backends_create.add_argument("--tor-proxy")
    backends_create.add_argument("--notes")

    backends_update = backends_sub.add_parser("update")
    backends_update.add_argument("name")
    backends_update.add_argument("--kind", choices=sorted(BACKEND_KINDS))
    backends_update.add_argument("--url")
    backends_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    backends_update.add_argument("--network")
    backends_update.add_argument("--auth-header")
    backends_update.add_argument("--token")
    backends_update.add_argument("--batch-size", type=int)
    backends_update.add_argument("--timeout", type=int)
    backends_update.add_argument("--tor-proxy")
    backends_update.add_argument("--notes")

    backends_delete = backends_sub.add_parser("delete")
    backends_delete.add_argument("name")

    backends_set_default = backends_sub.add_parser("set-default")
    backends_set_default.add_argument("name")

    backends_sub.add_parser("clear-default")

    context = sub.add_parser("context")
    context_sub = context.add_subparsers(dest="context_command", required=True)
    context_sub.add_parser("show")
    context_sub.add_parser("current")
    context_set = context_sub.add_parser("set")
    context_set.add_argument("--workspace")
    context_set.add_argument("--profile")

    workspaces = sub.add_parser("workspaces")
    ws_sub = workspaces.add_subparsers(dest="workspaces_command", required=True)
    ws_sub.add_parser("list")
    ws_create = ws_sub.add_parser("create")
    ws_create.add_argument("label")

    profiles = sub.add_parser("profiles")
    profiles_sub = profiles.add_subparsers(dest="profiles_command", required=True)
    profiles_list = profiles_sub.add_parser("list")
    profiles_list.add_argument("--workspace")
    profiles_create = profiles_sub.add_parser("create")
    profiles_create.add_argument("label")
    profiles_create.add_argument("--workspace")
    profiles_create.add_argument("--fiat-currency", default="USD")
    profiles_create.add_argument("--tax-country", choices=list(supported_tax_countries()), default=DEFAULT_TAX_COUNTRY)
    profiles_create.add_argument("--tax-long-term-days", type=int, default=DEFAULT_LONG_TERM_DAYS)
    profiles_create.add_argument("--gains-algorithm", choices=list(RP2_ACCOUNTING_METHODS), default="FIFO")

    profiles_get = profiles_sub.add_parser("get")
    profiles_get.add_argument("--workspace")
    profiles_get.add_argument("--profile")

    profiles_set = profiles_sub.add_parser("set")
    profiles_set.add_argument("--workspace")
    profiles_set.add_argument("--profile")
    profiles_set.add_argument("--label")
    profiles_set.add_argument("--fiat-currency")
    profiles_set.add_argument("--tax-country", choices=list(supported_tax_countries()))
    profiles_set.add_argument("--tax-long-term-days", type=int)
    profiles_set.add_argument("--gains-algorithm", choices=list(RP2_ACCOUNTING_METHODS))

    accounts = sub.add_parser("accounts")
    accounts_sub = accounts.add_subparsers(dest="accounts_command", required=True)
    accounts_list = accounts_sub.add_parser("list")
    accounts_list.add_argument("--workspace")
    accounts_list.add_argument("--profile")
    accounts_create = accounts_sub.add_parser("create")
    accounts_create.add_argument("--workspace")
    accounts_create.add_argument("--profile")
    accounts_create.add_argument("--code", required=True)
    accounts_create.add_argument("--label", required=True)
    accounts_create.add_argument("--type", required=True)
    accounts_create.add_argument("--asset")

    wallets = sub.add_parser("wallets")
    wallets_sub = wallets.add_subparsers(dest="wallets_command", required=True)
    wallets_list = wallets_sub.add_parser("list")
    wallets_list.add_argument("--workspace")
    wallets_list.add_argument("--profile")
    wallets_create = wallets_sub.add_parser("create")
    wallets_create.add_argument("--workspace")
    wallets_create.add_argument("--profile")
    wallets_create.add_argument("--label", required=True)
    wallets_create.add_argument("--kind", required=True)
    wallets_create.add_argument("--account")
    wallets_create.add_argument("--backend")
    wallets_create.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_create.add_argument("--network")
    wallets_create.add_argument("--address", action="append")
    wallets_create.add_argument("--descriptor")
    wallets_create.add_argument("--descriptor-file")
    wallets_create.add_argument("--change-descriptor")
    wallets_create.add_argument("--change-descriptor-file")
    wallets_create.add_argument("--gap-limit", type=int)
    wallets_create.add_argument("--policy-asset")
    wallets_create.add_argument("--altbestand", action="store_true")
    wallets_create.add_argument("--config")
    wallets_create.add_argument("--config-file")
    wallets_create.add_argument("--source-file")
    wallets_create.add_argument("--source-format", choices=["json", "csv", "btcpay_json", "btcpay_csv", "phoenix_csv"])
    wallets_altbestand = wallets_sub.add_parser("set-altbestand")
    wallets_altbestand.add_argument("--workspace")
    wallets_altbestand.add_argument("--profile")
    wallets_altbestand.add_argument("--wallet", required=True)
    wallets_neubestand = wallets_sub.add_parser("set-neubestand")
    wallets_neubestand.add_argument("--workspace")
    wallets_neubestand.add_argument("--profile")
    wallets_neubestand.add_argument("--wallet", required=True)

    wallets_sub.add_parser("kinds")

    wallets_get = wallets_sub.add_parser("get")
    wallets_get.add_argument("--workspace")
    wallets_get.add_argument("--profile")
    wallets_get.add_argument("--wallet", required=True)

    wallets_update = wallets_sub.add_parser("update")
    wallets_update.add_argument("--workspace")
    wallets_update.add_argument("--profile")
    wallets_update.add_argument("--wallet", required=True)
    wallets_update.add_argument("--label")
    wallets_update.add_argument("--account")
    wallets_update.add_argument("--backend")
    wallets_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_update.add_argument("--network")
    wallets_update.add_argument("--gap-limit", type=int)
    wallets_update.add_argument("--policy-asset")
    wallets_update.add_argument("--config")
    wallets_update.add_argument("--config-file")
    wallets_update.add_argument("--set-altbestand", action="store_true")
    wallets_update.add_argument("--clear-altbestand", action="store_true")
    wallets_update.add_argument("--clear", action="append", default=[], metavar="FIELD", help="Clear a config field (repeatable)")

    wallets_delete = wallets_sub.add_parser("delete")
    wallets_delete.add_argument("--workspace")
    wallets_delete.add_argument("--profile")
    wallets_delete.add_argument("--wallet", required=True)
    wallets_delete.add_argument("--cascade", action="store_true", help="Also delete transactions and journal entries belonging to this wallet")
    wallets_import_json = wallets_sub.add_parser("import-json")
    wallets_import_json.add_argument("--workspace")
    wallets_import_json.add_argument("--profile")
    wallets_import_json.add_argument("--wallet", required=True)
    wallets_import_json.add_argument("--file", required=True)
    wallets_import_csv = wallets_sub.add_parser("import-csv")
    wallets_import_csv.add_argument("--workspace")
    wallets_import_csv.add_argument("--profile")
    wallets_import_csv.add_argument("--wallet", required=True)
    wallets_import_csv.add_argument("--file", required=True)
    wallets_import_btcpay = wallets_sub.add_parser("import-btcpay")
    wallets_import_btcpay.add_argument("--workspace")
    wallets_import_btcpay.add_argument("--profile")
    wallets_import_btcpay.add_argument("--wallet", required=True)
    wallets_import_btcpay.add_argument("--file", required=True)
    wallets_import_btcpay.add_argument("--input-format", "--format", dest="input_format", choices=["json", "csv"], default="csv")
    wallets_import_phoenix = wallets_sub.add_parser("import-phoenix")
    wallets_import_phoenix.add_argument("--workspace")
    wallets_import_phoenix.add_argument("--profile")
    wallets_import_phoenix.add_argument("--wallet", required=True)
    wallets_import_phoenix.add_argument("--file", required=True)
    wallets_sync = wallets_sub.add_parser("sync")
    wallets_sync.add_argument("--workspace")
    wallets_sync.add_argument("--profile")
    wallets_sync.add_argument("--wallet")
    wallets_sync.add_argument("--all", action="store_true")
    wallets_derive = wallets_sub.add_parser("derive")
    wallets_derive.add_argument("--workspace")
    wallets_derive.add_argument("--profile")
    wallets_derive.add_argument("--wallet", required=True)
    wallets_derive.add_argument("--branch", default="all")
    wallets_derive.add_argument("--start", type=int, default=0)
    wallets_derive.add_argument("--count", type=int)

    transactions = sub.add_parser("transactions")
    tx_sub = transactions.add_subparsers(dest="transactions_command", required=True)
    tx_list = tx_sub.add_parser("list")
    tx_list.add_argument("--workspace")
    tx_list.add_argument("--profile")
    tx_list.add_argument("--wallet")
    tx_list.add_argument("--limit", type=int, default=100)

    metadata = sub.add_parser("metadata")
    meta_sub = metadata.add_subparsers(dest="metadata_command", required=True)
    notes = meta_sub.add_parser("notes")
    notes_sub = notes.add_subparsers(dest="notes_command", required=True)
    notes_set = notes_sub.add_parser("set")
    notes_set.add_argument("--workspace")
    notes_set.add_argument("--profile")
    notes_set.add_argument("--transaction", required=True)
    notes_set.add_argument("--note", required=True)
    notes_clear = notes_sub.add_parser("clear")
    notes_clear.add_argument("--workspace")
    notes_clear.add_argument("--profile")
    notes_clear.add_argument("--transaction", required=True)
    tags = meta_sub.add_parser("tags")
    tags_sub = tags.add_subparsers(dest="tags_command", required=True)
    tags_list = tags_sub.add_parser("list")
    tags_list.add_argument("--workspace")
    tags_list.add_argument("--profile")
    tags_create = tags_sub.add_parser("create")
    tags_create.add_argument("--workspace")
    tags_create.add_argument("--profile")
    tags_create.add_argument("--code", required=True)
    tags_create.add_argument("--label", required=True)
    tags_add = tags_sub.add_parser("add")
    tags_add.add_argument("--workspace")
    tags_add.add_argument("--profile")
    tags_add.add_argument("--transaction", required=True)
    tags_add.add_argument("--tag", required=True)
    tags_remove = tags_sub.add_parser("remove")
    tags_remove.add_argument("--workspace")
    tags_remove.add_argument("--profile")
    tags_remove.add_argument("--transaction", required=True)
    tags_remove.add_argument("--tag", required=True)
    bip329 = meta_sub.add_parser("bip329")
    bip329_sub = bip329.add_subparsers(dest="bip329_command", required=True)
    bip329_import = bip329_sub.add_parser("import")
    bip329_import.add_argument("--workspace")
    bip329_import.add_argument("--profile")
    bip329_import.add_argument("--wallet")
    bip329_import.add_argument("--file", required=True)
    bip329_list = bip329_sub.add_parser("list")
    bip329_list.add_argument("--workspace")
    bip329_list.add_argument("--profile")
    bip329_list.add_argument("--wallet")
    bip329_list.add_argument("--limit", type=int, default=100)
    bip329_export = bip329_sub.add_parser("export")
    bip329_export.add_argument("--workspace")
    bip329_export.add_argument("--profile")
    bip329_export.add_argument("--wallet")
    bip329_export.add_argument("--file", required=True)
    exclude = meta_sub.add_parser("exclude")
    exclude.add_argument("--workspace")
    exclude.add_argument("--profile")
    exclude.add_argument("--transaction", required=True)
    include = meta_sub.add_parser("include")
    include.add_argument("--workspace")
    include.add_argument("--profile")
    include.add_argument("--transaction", required=True)

    records = meta_sub.add_parser("records")
    records_sub = records.add_subparsers(dest="records_command", required=True)

    records_list = records_sub.add_parser("list")
    records_list.add_argument("--workspace")
    records_list.add_argument("--profile")
    records_list.add_argument("--wallet")
    records_list.add_argument("--tag")
    records_list.add_argument("--has-note", dest="has_note", action="store_true")
    records_list.add_argument("--no-note", dest="no_note", action="store_true")
    records_list.add_argument("--excluded", action="store_true")
    records_list.add_argument("--included", action="store_true")
    records_list.add_argument("--start")
    records_list.add_argument("--end")
    records_list.add_argument("--cursor")
    records_list.add_argument("--limit", type=int, default=DEFAULT_EVENTS_LIMIT)

    records_get = records_sub.add_parser("get")
    records_get.add_argument("--workspace")
    records_get.add_argument("--profile")
    records_get.add_argument("--transaction", required=True)

    records_note = records_sub.add_parser("note")
    records_note_sub = records_note.add_subparsers(dest="records_note_command", required=True)
    rn_set = records_note_sub.add_parser("set")
    rn_set.add_argument("--workspace")
    rn_set.add_argument("--profile")
    rn_set.add_argument("--transaction", required=True)
    rn_set.add_argument("--note", required=True)
    rn_clear = records_note_sub.add_parser("clear")
    rn_clear.add_argument("--workspace")
    rn_clear.add_argument("--profile")
    rn_clear.add_argument("--transaction", required=True)

    records_tag = records_sub.add_parser("tag")
    records_tag_sub = records_tag.add_subparsers(dest="records_tag_command", required=True)
    rt_add = records_tag_sub.add_parser("add")
    rt_add.add_argument("--workspace")
    rt_add.add_argument("--profile")
    rt_add.add_argument("--transaction", required=True)
    rt_add.add_argument("--tag", required=True)
    rt_remove = records_tag_sub.add_parser("remove")
    rt_remove.add_argument("--workspace")
    rt_remove.add_argument("--profile")
    rt_remove.add_argument("--transaction", required=True)
    rt_remove.add_argument("--tag", required=True)

    records_excluded = records_sub.add_parser("excluded")
    records_excluded_sub = records_excluded.add_subparsers(dest="records_excluded_command", required=True)
    re_set = records_excluded_sub.add_parser("set")
    re_set.add_argument("--workspace")
    re_set.add_argument("--profile")
    re_set.add_argument("--transaction", required=True)
    re_clear = records_excluded_sub.add_parser("clear")
    re_clear.add_argument("--workspace")
    re_clear.add_argument("--profile")
    re_clear.add_argument("--transaction", required=True)

    journals = sub.add_parser("journals")
    journals_sub = journals.add_subparsers(dest="journals_command", required=True)
    journals_process = journals_sub.add_parser("process")
    journals_process.add_argument("--workspace")
    journals_process.add_argument("--profile")
    journals_list = journals_sub.add_parser("list")
    journals_list.add_argument("--workspace")
    journals_list.add_argument("--profile")
    journals_list.add_argument("--limit", type=int, default=200)
    journals_quarantined = journals_sub.add_parser("quarantined")
    journals_quarantined.add_argument("--workspace")
    journals_quarantined.add_argument("--profile")

    journals_events = journals_sub.add_parser("events")
    events_sub = journals_events.add_subparsers(dest="events_command", required=True)
    events_list = events_sub.add_parser("list")
    events_list.add_argument("--workspace")
    events_list.add_argument("--profile")
    events_list.add_argument("--wallet")
    events_list.add_argument("--account")
    events_list.add_argument("--asset")
    events_list.add_argument("--entry-type", help="Filter by entry type (debit, credit, etc.)")
    events_list.add_argument("--start", help="RFC3339 lower bound (inclusive) on occurred_at")
    events_list.add_argument("--end", help="RFC3339 upper bound (inclusive) on occurred_at")
    events_list.add_argument("--cursor", help="Opaque pagination cursor from a previous response")
    events_list.add_argument("--limit", type=int, default=DEFAULT_EVENTS_LIMIT)
    events_get = events_sub.add_parser("get")
    events_get.add_argument("--workspace")
    events_get.add_argument("--profile")
    events_get.add_argument("--event-id", required=True)

    journals_quarantine = journals_sub.add_parser("quarantine")
    qsub = journals_quarantine.add_subparsers(dest="quarantine_command", required=True)

    q_show = qsub.add_parser("show")
    q_show.add_argument("--workspace")
    q_show.add_argument("--profile")
    q_show.add_argument("--transaction", required=True)

    q_clear = qsub.add_parser("clear")
    q_clear.add_argument("--workspace")
    q_clear.add_argument("--profile")
    q_clear.add_argument("--transaction", required=True)

    q_resolve = qsub.add_parser("resolve")
    qrsub = q_resolve.add_subparsers(dest="quarantine_resolve_command", required=True)

    q_price = qrsub.add_parser("price-override")
    q_price.add_argument("--workspace")
    q_price.add_argument("--profile")
    q_price.add_argument("--transaction", required=True)
    q_price.add_argument("--fiat-rate")
    q_price.add_argument("--fiat-value")

    q_exclude = qrsub.add_parser("exclude")
    q_exclude.add_argument("--workspace")
    q_exclude.add_argument("--profile")
    q_exclude.add_argument("--transaction", required=True)

    reports = sub.add_parser("reports")
    reports_sub = reports.add_subparsers(dest="reports_command", required=True)
    for report_name in ["balance-sheet", "portfolio-summary", "capital-gains", "journal-entries"]:
        report = reports_sub.add_parser(report_name)
        report.add_argument("--workspace")
        report.add_argument("--profile")

    balance_history = reports_sub.add_parser("balance-history")
    balance_history.add_argument("--workspace")
    balance_history.add_argument("--profile")
    balance_history.add_argument("--interval", choices=list(INTERVAL_CHOICES), default="day")
    balance_history.add_argument("--start")
    balance_history.add_argument("--end")
    balance_history.add_argument("--wallet")
    balance_history.add_argument("--account")
    balance_history.add_argument("--asset")

    rates = sub.add_parser("rates")
    rates_sub = rates.add_subparsers(dest="rates_command", required=True)

    rates_pairs = rates_sub.add_parser("pairs")
    rates_pairs.set_defaults(rates_command="pairs")
    _ = rates_pairs

    rates_sync = rates_sub.add_parser("sync")
    rates_sync.add_argument("--pair")
    rates_sync.add_argument("--days", type=int, default=30)
    rates_sync.add_argument("--source", default="coingecko")

    rates_latest = rates_sub.add_parser("latest")
    rates_latest.add_argument("pair")

    rates_range = rates_sub.add_parser("range")
    rates_range.add_argument("pair")
    rates_range.add_argument("--start")
    rates_range.add_argument("--end")
    rates_range.add_argument("--limit", type=int)

    rates_set = rates_sub.add_parser("set")
    rates_set.add_argument("pair")
    rates_set.add_argument("timestamp")
    rates_set.add_argument("rate")
    rates_set.add_argument("--source", default="manual")

    return parser


def dispatch(conn, args):
    if args.command == "init":
        return cmd_init(conn, args)
    if args.command == "status":
        return cmd_status(conn, args)
    if args.command == "backends":
        if args.backends_command == "list":
            return emit(args, list_backends(args.runtime_config))
        if args.backends_command == "kinds":
            return emit(args, [{"kind": k} for k in sorted(BACKEND_KINDS)])
        if args.backends_command == "get":
            # Prefer DB row; fall back to runtime_config view if env/built-in only
            try:
                return emit(args, get_db_backend(conn, args.name))
            except AppError as exc:
                if exc.code != "not_found":
                    raise
                name = args.name.strip().lower()
                backend = args.runtime_config["backends"].get(name)
                if not backend:
                    raise
                return emit(
                    args,
                    {
                        "name": name,
                        "kind": backend.get("kind", ""),
                        "chain": backend.get("chain", ""),
                        "network": backend.get("network", ""),
                        "url": backend.get("url", ""),
                        "batch_size": backend.get("batch_size"),
                        "auth_header": backend.get("auth_header", ""),
                        "token": backend.get("token", ""),
                        "timeout": backend.get("timeout"),
                        "tor_proxy": backend.get("tor_proxy", ""),
                        "notes": "",
                        "source": backend.get("source", ""),
                        "is_default": name == args.runtime_config["default_backend"],
                    },
                )
        if args.backends_command == "create":
            return emit(
                args,
                create_db_backend(
                    conn,
                    args.name,
                    args.kind,
                    args.url,
                    chain=args.chain,
                    network=args.network,
                    auth_header=args.auth_header,
                    token=args.token,
                    batch_size=args.batch_size,
                    timeout=args.timeout,
                    tor_proxy=args.tor_proxy,
                    notes=args.notes,
                ),
            )
        if args.backends_command == "update":
            updates = {
                "kind": args.kind,
                "url": args.url,
                "chain": args.chain,
                "network": args.network,
                "auth_header": args.auth_header,
                "token": args.token,
                "batch_size": args.batch_size,
                "timeout": args.timeout,
                "tor_proxy": args.tor_proxy,
                "notes": args.notes,
            }
            return emit(args, update_db_backend(conn, args.name, updates))
        if args.backends_command == "delete":
            return emit(args, delete_db_backend(conn, args.name))
        if args.backends_command == "set-default":
            return emit(args, set_default_backend(conn, args.runtime_config, args.name))
        if args.backends_command == "clear-default":
            return emit(args, clear_default_backend(conn, args.runtime_config))
    if args.command == "context":
        if args.context_command == "show":
            return cmd_context_show(conn, args)
        if args.context_command == "current":
            return cmd_context_show(conn, args)
        if args.context_command == "set":
            return cmd_context_set(conn, args)
    if args.command == "workspaces":
        if args.workspaces_command == "list":
            return emit(args, list_workspaces(conn))
        if args.workspaces_command == "create":
            return emit(args, dict(create_workspace(conn, args.label)))
    if args.command == "profiles":
        if args.profiles_command == "list":
            return emit(args, list_profiles(conn, args.workspace))
        if args.profiles_command == "create":
            return emit(
                args,
                dict(
                    create_profile(
                        conn,
                        args.workspace,
                        args.label,
                        args.fiat_currency,
                        args.gains_algorithm,
                        args.tax_country,
                        args.tax_long_term_days,
                    )
                ),
            )
        if args.profiles_command == "get":
            return emit(args, get_profile_details(conn, args.workspace, args.profile))
        if args.profiles_command == "set":
            updates = {
                "label": args.label,
                "fiat_currency": args.fiat_currency,
                "tax_country": args.tax_country,
                "tax_long_term_days": args.tax_long_term_days,
                "gains_algorithm": args.gains_algorithm,
            }
            if all(v is None for v in updates.values()):
                raise AppError(
                    "profiles set requires at least one field to update",
                    code="validation",
                    hint="Pass one or more of --label, --fiat-currency, --tax-country, --tax-long-term-days, --gains-algorithm",
                )
            return emit(args, update_profile(conn, args.workspace, args.profile, updates))
    if args.command == "accounts":
        if args.accounts_command == "list":
            return emit(args, list_accounts(conn, args.workspace, args.profile))
        if args.accounts_command == "create":
            return emit(
                args,
                dict(
                    create_account(
                        conn,
                        args.workspace,
                        args.profile,
                        args.code,
                        args.label,
                        args.type,
                        args.asset,
                    )
                ),
            )
    if args.command == "wallets":
        if args.wallets_command == "list":
            return emit(args, list_wallets(conn, args.workspace, args.profile))
        if args.wallets_command == "create":
            return emit(
                args,
                dict(
                    create_wallet(
                        conn,
                        args.workspace,
                        args.profile,
                        args.label,
                        args.kind,
                        args.account,
                        parse_wallet_config(args),
                    )
                ),
            )
        if args.wallets_command == "set-altbestand":
            return emit(args, set_wallet_altbestand(conn, args.workspace, args.profile, args.wallet, True))
        if args.wallets_command == "set-neubestand":
            return emit(args, set_wallet_altbestand(conn, args.workspace, args.profile, args.wallet, False))
        if args.wallets_command == "kinds":
            return emit(args, list_wallet_kinds())
        if args.wallets_command == "get":
            return emit(args, get_wallet_details(conn, args.workspace, args.profile, args.wallet))
        if args.wallets_command == "update":
            if args.set_altbestand and args.clear_altbestand:
                raise AppError(
                    "--set-altbestand and --clear-altbestand are mutually exclusive",
                    code="validation",
                )
            altbestand = None
            if args.set_altbestand:
                altbestand = True
            elif args.clear_altbestand:
                altbestand = False
            config_updates = {}
            if args.config:
                config_updates.update(json.loads(args.config))
            if args.config_file:
                with open(args.config_file, "r", encoding="utf-8") as handle:
                    config_updates.update(json.load(handle))
            if args.backend:
                config_updates["backend"] = args.backend.strip().lower()
            if args.chain:
                config_updates["chain"] = normalize_chain_value(args.chain)
            if args.network:
                chain_for_net = normalize_chain_value(config_updates.get("chain") or args.chain)
                config_updates["network"] = normalize_network_value(chain_for_net, args.network)
            if args.gap_limit is not None:
                if args.gap_limit <= 0:
                    raise AppError("Descriptor gap limit must be positive", code="validation")
                config_updates["gap_limit"] = args.gap_limit
            if args.policy_asset:
                config_updates["policy_asset"] = normalize_asset_code(args.policy_asset)
            updates = {
                "label": args.label,
                "account": args.account,
                "altbestand": altbestand,
                "config": config_updates,
                "clear": args.clear,
            }
            return emit(args, update_wallet(conn, args.workspace, args.profile, args.wallet, updates))
        if args.wallets_command == "delete":
            return emit(args, delete_wallet(conn, args.workspace, args.profile, args.wallet, cascade=args.cascade))
        if args.wallets_command == "import-json":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "json"))
        if args.wallets_command == "import-csv":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "csv"))
        if args.wallets_command == "import-btcpay":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    f"btcpay_{args.input_format}",
                ),
            )
        if args.wallets_command == "import-phoenix":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "phoenix_csv",
                ),
            )
        if args.wallets_command == "sync":
            return emit(args, sync_wallet(conn, args.runtime_config, args.workspace, args.profile, args.wallet, args.all))
        if args.wallets_command == "derive":
            return emit(
                args,
                derive_wallet_targets(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    branch=args.branch,
                    start=args.start,
                    count=args.count,
                ),
            )
    if args.command == "transactions":
        if args.transactions_command == "list":
            return emit(args, list_transactions(conn, args.workspace, args.profile, args.wallet, args.limit))
    if args.command == "metadata":
        if args.metadata_command == "notes":
            if args.notes_command == "set":
                return emit(args, set_transaction_note(conn, args.workspace, args.profile, args.transaction, args.note))
            if args.notes_command == "clear":
                return emit(args, clear_transaction_note(conn, args.workspace, args.profile, args.transaction))
        if args.metadata_command == "tags":
            if args.tags_command == "list":
                return emit(args, list_tags(conn, args.workspace, args.profile))
            if args.tags_command == "create":
                return emit(args, dict(create_tag(conn, args.workspace, args.profile, args.code, args.label)))
            if args.tags_command == "add":
                return emit(args, add_tag_to_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
            if args.tags_command == "remove":
                return emit(args, remove_tag_from_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
        if args.metadata_command == "bip329":
            if args.bip329_command == "import":
                return emit(args, import_bip329_labels(conn, args.workspace, args.profile, args.file, args.wallet))
            if args.bip329_command == "list":
                return emit(args, list_bip329_labels(conn, args.workspace, args.profile, args.wallet, args.limit))
            if args.bip329_command == "export":
                return emit(args, export_bip329_labels(conn, args.workspace, args.profile, args.file, args.wallet))
        if args.metadata_command == "exclude":
            return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, True))
        if args.metadata_command == "include":
            return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, False))
        if args.metadata_command == "records":
            if args.records_command == "list":
                if args.has_note and args.no_note:
                    raise AppError("--has-note and --no-note are mutually exclusive", code="validation")
                if args.excluded and args.included:
                    raise AppError("--excluded and --included are mutually exclusive", code="validation")
                has_note = True if args.has_note else (False if args.no_note else None)
                excluded = True if args.excluded else (False if args.included else None)
                return emit(
                    args,
                    list_transaction_records(
                        conn,
                        args.workspace,
                        args.profile,
                        wallet=args.wallet,
                        tag=args.tag,
                        has_note=has_note,
                        excluded=excluded,
                        start=args.start,
                        end=args.end,
                        cursor=args.cursor,
                        limit=args.limit,
                    ),
                )
            if args.records_command == "get":
                return emit(args, get_transaction_record(conn, args.workspace, args.profile, args.transaction))
            if args.records_command == "note":
                if args.records_note_command == "set":
                    return emit(args, set_transaction_note(conn, args.workspace, args.profile, args.transaction, args.note))
                if args.records_note_command == "clear":
                    return emit(args, clear_transaction_note(conn, args.workspace, args.profile, args.transaction))
            if args.records_command == "tag":
                if args.records_tag_command == "add":
                    return emit(args, add_tag_to_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
                if args.records_tag_command == "remove":
                    return emit(args, remove_tag_from_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
            if args.records_command == "excluded":
                if args.records_excluded_command == "set":
                    return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, True))
                if args.records_excluded_command == "clear":
                    return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, False))
    if args.command == "journals":
        if args.journals_command == "process":
            return emit(args, process_journals(conn, args.workspace, args.profile))
        if args.journals_command == "list":
            return emit(args, list_journal_entries(conn, args.workspace, args.profile, args.limit))
        if args.journals_command == "events":
            if args.events_command == "list":
                return emit(
                    args,
                    list_journal_events(
                        conn,
                        args.workspace,
                        args.profile,
                        wallet=args.wallet,
                        account=args.account,
                        asset=args.asset,
                        entry_type=args.entry_type,
                        start=args.start,
                        end=args.end,
                        cursor=args.cursor,
                        limit=args.limit,
                    ),
                )
            if args.events_command == "get":
                return emit(args, get_journal_event(conn, args.workspace, args.profile, args.event_id))
        if args.journals_command == "quarantined":
            return emit(args, list_quarantines(conn, args.workspace, args.profile))
        if args.journals_command == "quarantine":
            if args.quarantine_command == "show":
                return emit(args, show_quarantine(conn, args.workspace, args.profile, args.transaction))
            if args.quarantine_command == "clear":
                return emit(args, clear_quarantine(conn, args.workspace, args.profile, args.transaction))
            if args.quarantine_command == "resolve":
                if args.quarantine_resolve_command == "price-override":
                    return emit(
                        args,
                        resolve_quarantine_price_override(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            fiat_rate=args.fiat_rate,
                            fiat_value=args.fiat_value,
                        ),
                    )
                if args.quarantine_resolve_command == "exclude":
                    return emit(
                        args,
                        resolve_quarantine_exclude(
                            conn, args.workspace, args.profile, args.transaction
                        ),
                    )
    if args.command == "reports":
        if args.reports_command == "balance-sheet":
            return emit(args, report_balance_sheet(conn, args.workspace, args.profile))
        if args.reports_command == "portfolio-summary":
            return emit(args, report_portfolio_summary(conn, args.workspace, args.profile))
        if args.reports_command == "capital-gains":
            return emit(args, report_capital_gains(conn, args.workspace, args.profile))
        if args.reports_command == "journal-entries":
            return emit(args, report_journal_entries(conn, args.workspace, args.profile))
        if args.reports_command == "balance-history":
            return emit(
                args,
                report_balance_history(
                    conn,
                    args.workspace,
                    args.profile,
                    interval=args.interval,
                    start=args.start,
                    end=args.end,
                    wallet_ref=args.wallet,
                    account_ref=args.account,
                    asset=args.asset,
                ),
            )
    if args.command == "rates":
        if args.rates_command == "pairs":
            return emit(args, list_cached_pairs(conn))
        if args.rates_command == "sync":
            return emit(
                args,
                sync_rates(conn, pair=args.pair, days=args.days, source=args.source),
            )
        if args.rates_command == "latest":
            return emit(args, get_latest_rate(conn, args.pair))
        if args.rates_command == "range":
            return emit(
                args,
                get_rate_range(
                    conn,
                    args.pair,
                    start=args.start,
                    end=args.end,
                    limit=args.limit,
                ),
            )
        if args.rates_command == "set":
            return emit(
                args,
                set_manual_rate(conn, args.pair, args.timestamp, args.rate, source=args.source),
            )
    raise AppError("Unknown command")


def command_needs_db(args):
    return True


def _resolve_output_format(args):
    if args.machine:
        if args.format is not None and args.format != "json":
            raise AppError(
                f"--machine requires --format json, got --format {args.format}",
                code="invalid_flag_combination",
            )
        return "json"
    return args.format or "table"


def _emit_error(args, exc, debug_text=None):
    code = getattr(exc, "code", "app_error") or "app_error"
    message = str(exc)
    details = getattr(exc, "details", None)
    hint = getattr(exc, "hint", None)
    retryable = getattr(exc, "retryable", False)
    fmt = getattr(args, "format", None) or "table"
    if fmt == "json":
        envelope = build_error_envelope(
            code,
            message,
            details=details,
            hint=hint,
            retryable=retryable,
            debug=debug_text,
        )
        try:
            _write_text(args, json.dumps(envelope, indent=2, sort_keys=False))
        except Exception:
            print(json.dumps(envelope, indent=2, sort_keys=False), file=sys.stderr)
    else:
        print(f"error: {message}", file=sys.stderr)
        if hint:
            print(f"hint: {hint}", file=sys.stderr)


def main(argv=None):
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        raise
    try:
        args.format = _resolve_output_format(args)
    except AppError as exc:
        args.format = "table"
        _emit_error(args, exc)
        return 1
    args.data_root = str(resolve_effective_data_root(args.data_root))
    args.env_file = str(resolve_effective_env_file(args.env_file, args.data_root))
    ensure_data_root(args.data_root)
    ensure_data_root(Path(args.env_file).expanduser().parent)
    ensure_data_root(resolve_exports_root(args.data_root))
    ensure_settings_file(args.data_root, args.env_file)
    args.runtime_config = load_runtime_config(args.env_file)
    conn = open_db(args.data_root) if command_needs_db(args) else None
    if conn is not None:
        try:
            merge_db_backends(conn, args.runtime_config)
        except AppError as exc:
            debug_text = None
            if args.debug:
                import traceback
                debug_text = traceback.format_exc()
                sys.stderr.write(debug_text)
            _emit_error(args, exc, debug_text=debug_text)
            conn.close()
            return 1
    try:
        dispatch(conn, args)
        return 0
    except AppError as exc:
        debug_text = None
        if args.debug:
            import traceback
            debug_text = traceback.format_exc()
            sys.stderr.write(debug_text)
        _emit_error(args, exc, debug_text=debug_text)
        return 1
    except Exception as exc:
        import traceback
        debug_text = traceback.format_exc()
        if args.debug:
            sys.stderr.write(debug_text)
        wrapped = AppError(str(exc) or exc.__class__.__name__, code="internal_error")
        _emit_error(args, wrapped, debug_text=debug_text if args.debug else None)
        return 1
    finally:
        if conn is not None:
            conn.close()

import argparse
import base64
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
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from importlib import import_module
from pathlib import Path
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from . import __version__
from .tax_policy import (
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    build_tax_policy,
    supported_tax_countries,
)


APP_NAME = "kassiber"
LEGACY_APP_NAME = "satbooks"
DEFAULT_DATA_ROOT = os.path.expanduser(f"~/.local/share/{APP_NAME}")
LEGACY_DATA_ROOT = os.path.expanduser(f"~/.local/share/{LEGACY_APP_NAME}")
DEFAULT_DB_FILENAME = f"{APP_NAME}.sqlite3"
LEGACY_DB_FILENAME = f"{LEGACY_APP_NAME}.sqlite3"
DEFAULT_ENV_FILE = ".env"
SATS_PER_BTC = Decimal("100000000")
UNKNOWN_OCCURRED_AT = "1970-01-01T00:00:00Z"
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
DEFAULT_BACKENDS = {
    "mempool": {
        "name": "mempool",
        "kind": "esplora",
        "url": "https://mempool.space/api",
        "source": "built-in default",
    }
}
B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
B58_INDEX = {char: index for index, char in enumerate(B58_ALPHABET)}
BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
BECH32_INDEX = {char: index for index, char in enumerate(BECH32_CHARSET)}

SCHEMA = """
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    fiat_currency TEXT NOT NULL DEFAULT 'USD',
    tax_country TEXT NOT NULL DEFAULT 'generic',
    tax_long_term_days INTEGER NOT NULL DEFAULT 365,
    gains_algorithm TEXT NOT NULL DEFAULT 'FIFO',
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (workspace_id, label)
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    account_type TEXT NOT NULL,
    asset TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE IF NOT EXISTS wallets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, label)
);

CREATE TABLE IF NOT EXISTS transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount REAL NOT NULL,
    fee REAL NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE IF NOT EXISTS transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
);

CREATE TABLE IF NOT EXISTS journal_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    quantity REAL NOT NULL,
    fiat_value REAL NOT NULL DEFAULT 0,
    unit_cost REAL NOT NULL DEFAULT 0,
    cost_basis REAL,
    proceeds REAL,
    gain_loss REAL,
    description TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS journal_quarantines (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS bip329_labels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    record_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    label TEXT,
    origin TEXT,
    spendable INTEGER,
    data_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
"""


class AppError(Exception):
    pass


def load_dotenv_file(path):
    values = {}
    if not path.exists():
        return values
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if value.startswith(("'", '"')) and value.endswith(("'", '"')) and len(value) >= 2:
            value = value[1:-1]
        values[key] = value
    return values


def load_runtime_config(env_file):
    env_path = Path(env_file).expanduser()
    file_env = load_dotenv_file(env_path)
    merged_env = {**file_env, **os.environ}
    backends = {name: dict(config) for name, config in DEFAULT_BACKENDS.items()}
    for prefix in ("SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"):
        for key, value in merged_env.items():
            if not key.startswith(prefix):
                continue
            suffix = key[len(prefix):]
            if "_" not in suffix:
                continue
            backend_name, field_name = suffix.split("_", 1)
            backend_name = backend_name.lower()
            field_name = field_name.lower()
            if not backend_name or not field_name:
                continue
            backends.setdefault(
                backend_name,
                {
                    "name": backend_name,
                    "kind": "",
                    "url": "",
                    "source": f"{env_path}" if key in file_env else "environment",
                },
            )
            backends[backend_name][field_name] = value.strip()
            backends[backend_name]["source"] = f"{env_path}" if key in file_env else "environment"
    default_backend = (
        merged_env.get("KASSIBER_DEFAULT_BACKEND")
        or merged_env.get("SATBOOKS_DEFAULT_BACKEND")
        or "mempool"
    ).strip().lower() or "mempool"
    if default_backend not in backends:
        raise AppError(
            f"Default backend '{default_backend}' is not defined. Add KASSIBER_BACKEND_{default_backend.upper()}_KIND and _URL to {env_path}."
        )
    for name, backend in backends.items():
        if not backend.get("kind") or not backend.get("url"):
            raise AppError(f"Backend '{name}' is missing kind or url configuration")
    return {
        "env_file": str(env_path),
        "env_file_exists": env_path.exists(),
        "default_backend": default_backend,
        "backends": backends,
    }


def dec(value, default="0"):
    if value is None:
        return Decimal(default)
    if isinstance(value, str) and value == "":
        return Decimal(default)
    try:
        return Decimal(str(value))
    except InvalidOperation as exc:
        raise AppError(f"Invalid decimal value: {value}") from exc


def str_or_none(value):
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def backend_value(backend, *keys):
    for key in keys:
        value = str_or_none(backend.get(key))
        if value is not None:
            return value
    return None


def parse_bool(value, default=False):
    if value is None:
        return default
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    raise AppError(f"Invalid boolean value: {value}")


def parse_int(value, default):
    if value is None or value == "":
        return default
    try:
        return int(str(value).strip())
    except ValueError as exc:
        raise AppError(f"Invalid integer value: {value}") from exc


def backend_timeout(backend, default=30):
    return parse_int(backend_value(backend, "timeout"), default)


def now_iso():
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def parse_timestamp(value):
    if not value:
        raise AppError("Missing occurred_at/date value")
    raw = str(value).strip()
    if len(raw) == 10:
        raw = f"{raw}T00:00:00+00:00"
    elif raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    dt = datetime.fromisoformat(raw)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    else:
        dt = dt.astimezone(timezone.utc)
    return dt.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def timestamp_to_iso(value, default=UNKNOWN_OCCURRED_AT):
    if value in (None, "", 0, "0"):
        return default
    return datetime.fromtimestamp(int(value), tz=timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


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


def ensure_data_root(data_root):
    path = Path(data_root).expanduser()
    path.mkdir(parents=True, exist_ok=True)
    return path


def resolve_effective_data_root(data_root):
    requested = Path(data_root).expanduser()
    if requested == Path(DEFAULT_DATA_ROOT).expanduser():
        legacy = Path(LEGACY_DATA_ROOT).expanduser()
        if not requested.exists() and legacy.exists():
            return legacy
    return requested


def resolve_database_path(data_root):
    root = Path(data_root).expanduser()
    current = root / DEFAULT_DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


def resolve_backend(runtime_config, name=None):
    backend_name = (name or runtime_config["default_backend"]).strip().lower()
    backend = runtime_config["backends"].get(backend_name)
    if not backend:
        raise AppError(f"Backend '{backend_name}' is not configured in {runtime_config['env_file']}")
    return backend


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


def open_db(data_root):
    root = ensure_data_root(resolve_effective_data_root(data_root))
    conn = sqlite3.connect(resolve_database_path(root))
    conn.row_factory = sqlite3.Row
    conn.executescript(SCHEMA)
    ensure_schema_compat(conn)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


_RP2_MODULES = None


def ensure_column(conn, table_name, column_name, definition):
    columns = {row["name"] for row in conn.execute(f"PRAGMA table_info({table_name})").fetchall()}
    if column_name in columns:
        return
    conn.execute(f"ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}")
    conn.commit()


def ensure_schema_compat(conn):
    ensure_column(conn, "profiles", "tax_country", f"TEXT NOT NULL DEFAULT '{DEFAULT_TAX_COUNTRY}'")
    ensure_column(conn, "profiles", "tax_long_term_days", f"INTEGER NOT NULL DEFAULT {DEFAULT_LONG_TERM_DAYS}")


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


def json_ready(value):
    if isinstance(value, sqlite3.Row):
        return {k: json_ready(value[k]) for k in value.keys()}
    if isinstance(value, dict):
        return {k: json_ready(v) for k, v in value.items()}
    if isinstance(value, list):
        return [json_ready(v) for v in value]
    if isinstance(value, Decimal):
        return float(value)
    return value


def emit(args, payload):
    if args.format == "json":
        print(json.dumps(json_ready(payload), indent=2, sort_keys=False))
        return
    if isinstance(payload, list):
        print_table(payload)
    elif isinstance(payload, dict):
        rows = [{"field": key, "value": value} for key, value in payload.items()]
        print_table(rows)
    else:
        print(payload)


def print_table(rows):
    if not rows:
        print("(no rows)")
        return
    normalized = [{key: format_table_value(value) for key, value in row.items()} for row in rows]
    headers = list(normalized[0].keys())
    widths = {header: len(header) for header in headers}
    for row in normalized:
        for header in headers:
            widths[header] = max(widths[header], len(row.get(header, "")))
    header_line = "  ".join(header.ljust(widths[header]) for header in headers)
    separator = "  ".join("-" * widths[header] for header in headers)
    print(header_line)
    print(separator)
    for row in normalized:
        print("  ".join(row.get(header, "").ljust(widths[header]) for header in headers))


def format_table_value(value):
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.8f}".rstrip("0").rstrip(".")
    return str(value)


def set_setting(conn, key, value):
    conn.execute(
        """
        INSERT INTO settings(key, value)
        VALUES(?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        """,
        (key, value),
    )


def get_setting(conn, key):
    row = conn.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
    return row["value"] if row else None


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
        (account_id, workspace["id"], profile["id"], code, label, account_type, asset.upper() if asset else None, now_iso()),
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


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    if getattr(args, "altbestand", False):
        config["altbestand"] = True
    return config


def create_wallet(conn, workspace_ref, profile_ref, label, kind, account_ref=None, config=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if account_ref:
        account = resolve_account(conn, profile["id"], account_ref)
    else:
        account = resolve_account(conn, profile["id"], "treasury")
    normalized_kind = normalize_wallet_kind(kind)
    config = config or {}
    if normalized_kind == "address" and not config.get("addresses") and not config.get("source_file"):
        raise AppError("Address wallets require at least one --address or a file-based source")
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
        output.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": row["account_code"] or row["account_label"],
                "backend": config.get("backend", ""),
                "addresses": ",".join(normalize_addresses(config.get("addresses"))),
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
    currency = str(sanitized_record.get("Currency") or "BTC").strip().upper()
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
        "asset": str(record.get("asset") or "BTC").upper(),
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
                float(normalized["amount"]),
                float(normalized["fee"]),
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


def fetch_esplora_transactions(base_url, address, max_pages=None):
    encoded = urlparse.quote(address, safe="")
    transactions = []
    seen_txids = set()
    last_seen = None
    page_count = 0
    while True:
        if max_pages is not None and page_count >= max_pages:
            break
        chain_url = (
            f"{base_url.rstrip('/')}/address/{encoded}/txs/chain/{last_seen}"
            if last_seen
            else f"{base_url.rstrip('/')}/address/{encoded}/txs/chain"
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
    mempool_url = f"{base_url.rstrip('/')}/address/{encoded}/txs/mempool"
    for tx in http_get_json(mempool_url):
        txid = tx.get("txid")
        if txid and txid not in seen_txids:
            seen_txids.add(txid)
            transactions.append(tx)
    return transactions


def sats_to_btc(value):
    return dec(value) / SATS_PER_BTC


def record_from_esplora_tx(tx, tracked_addresses, backend_name):
    tracked = set(tracked_addresses)
    received_sats = sum(
        dec(vout.get("value", 0))
        for vout in tx.get("vout", [])
        if vout.get("scriptpubkey_address") in tracked
    )
    sent_sats = Decimal("0")
    for vin in tx.get("vin", []):
        prevout = vin.get("prevout") or {}
        if prevout.get("scriptpubkey_address") in tracked:
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


def esplora_records_for_wallet(backend, addresses):
    max_pages = parse_int(backend_value(backend, "maxpages"), default=0) or None
    transactions_by_txid = {}
    for address in addresses:
        for tx in fetch_esplora_transactions(backend["url"], address, max_pages=max_pages):
            transactions_by_txid[tx["txid"]] = tx
    records = []
    for tx in sorted(
        transactions_by_txid.values(),
        key=lambda item: (((item.get("status") or {}).get("block_time") or 0), item.get("txid", "")),
    ):
        normalized = record_from_esplora_tx(tx, addresses, backend["name"])
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


def electrum_records_for_wallet(backend, addresses):
    transactions = {}
    header_timestamps = {}
    records = []
    tracked_scripts = {address_to_scriptpubkey(address).hex() for address in addresses}
    with ElectrumClient(backend) as client:
        histories = []
        for address in addresses:
            script_hash = electrum_scripthash(address)
            histories.extend(client.call("blockchain.scripthash.get_history", [script_hash]))

        def lookup(txid):
            if txid not in transactions:
                raw_tx = client.call("blockchain.transaction.get", [txid])
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
        for txid, history in sorted(txids.items(), key=lambda item: (item[1].get("height", 0), item[0])):
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
    addresses = normalize_addresses(config.get("addresses"))
    if not addresses:
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": "no addresses configured for backend sync",
        }
    kind = normalize_backend_kind(backend["kind"])
    adapter_meta = {}
    if kind == "esplora":
        normalized_records = esplora_records_for_wallet(backend, addresses)
    elif kind == "electrum":
        normalized_records = electrum_records_for_wallet(backend, addresses)
    elif kind == "bitcoinrpc":
        normalized_records, adapter_meta = bitcoinrpc_records_for_wallet(backend, wallet, addresses)
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
    outcome["addresses"] = ",".join(addresses)
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
        results.append(
            {
                "wallet": wallet["label"],
                "status": "skipped",
                "reason": "no file source or backend addresses configured",
            }
        )
    return results


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
    return [dict(row) for row in rows]


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
            rates[asset] = dec(row["fiat_value"]) / dec(row["amount"])
    return rates


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
        amount = dec(row["amount"])
        fee = dec(row["fee"])
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
                float(entry["quantity"]),
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
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }


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
    return [dict(row) for row in rows]


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
                "amount": row["amount"],
                "fee": row["fee"],
                "reason": row["reason"],
                "detail": json.dumps(detail, sort_keys=True),
            }
        )
    return output


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
    return [dict(row) for row in rows]


def report_journal_entries(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    return list_journal_entries(conn, profile["workspace_id"], profile["id"], limit=1000)


def show_status(conn, data_root):
    workspace_id = get_setting(conn, "context_workspace")
    profile_id = get_setting(conn, "context_profile")
    workspace = conn.execute("SELECT label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone() if workspace_id else None
    profile = conn.execute("SELECT label FROM profiles WHERE id = ?", (profile_id,)).fetchone() if profile_id else None
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
        "data_root": str(resolve_effective_data_root(data_root)),
        "current_workspace": workspace["label"] if workspace else "",
        "current_profile": profile["label"] if profile else "",
        **counts,
    }


def list_backends(runtime_config):
    rows = []
    for name, backend in sorted(runtime_config["backends"].items()):
        rows.append(
            {
                "name": name,
                "kind": backend["kind"],
                "url": backend["url"],
                "default": "yes" if name == runtime_config["default_backend"] else "",
                "source": backend["source"],
            }
        )
    return rows


def cmd_init(conn, args):
    init_app(conn)
    emit(
        args,
        {
            "version": __version__,
            "data_root": str(resolve_effective_data_root(args.data_root)),
            "database": str(resolve_database_path(resolve_effective_data_root(args.data_root))),
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
    parser.add_argument("--env-file", default=DEFAULT_ENV_FILE, help="Path to a .env file that defines named sync backends")
    parser.add_argument("--format", choices=["table", "json"], default="table", help="Output format")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("init")
    sub.add_parser("status")

    backends = sub.add_parser("backends")
    backends_sub = backends.add_subparsers(dest="backends_command", required=True)
    backends_sub.add_parser("list")

    context = sub.add_parser("context")
    context_sub = context.add_subparsers(dest="context_command", required=True)
    context_sub.add_parser("show")
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
    wallets_create.add_argument("--address", action="append")
    wallets_create.add_argument("--altbestand", action="store_true")
    wallets_create.add_argument("--config")
    wallets_create.add_argument("--config-file")
    wallets_create.add_argument("--source-file")
    wallets_create.add_argument("--source-format", choices=["json", "csv", "btcpay_json", "btcpay_csv"])
    wallets_altbestand = wallets_sub.add_parser("set-altbestand")
    wallets_altbestand.add_argument("--workspace")
    wallets_altbestand.add_argument("--profile")
    wallets_altbestand.add_argument("--wallet", required=True)
    wallets_neubestand = wallets_sub.add_parser("set-neubestand")
    wallets_neubestand.add_argument("--workspace")
    wallets_neubestand.add_argument("--profile")
    wallets_neubestand.add_argument("--wallet", required=True)
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
    wallets_import_btcpay.add_argument("--format", choices=["json", "csv"], default="csv")
    wallets_sync = wallets_sub.add_parser("sync")
    wallets_sync.add_argument("--workspace")
    wallets_sync.add_argument("--profile")
    wallets_sync.add_argument("--wallet")
    wallets_sync.add_argument("--all", action="store_true")

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

    reports = sub.add_parser("reports")
    reports_sub = reports.add_subparsers(dest="reports_command", required=True)
    for report_name in ["balance-sheet", "portfolio-summary", "capital-gains", "journal-entries"]:
        report = reports_sub.add_parser(report_name)
        report.add_argument("--workspace")
        report.add_argument("--profile")

    return parser


def dispatch(conn, args):
    if args.command == "init":
        return cmd_init(conn, args)
    if args.command == "status":
        return cmd_status(conn, args)
    if args.command == "backends":
        if args.backends_command == "list":
            return emit(args, list_backends(args.runtime_config))
    if args.command == "context":
        if args.context_command == "show":
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
                    f"btcpay_{args.format}",
                ),
            )
        if args.wallets_command == "sync":
            return emit(args, sync_wallet(conn, args.runtime_config, args.workspace, args.profile, args.wallet, args.all))
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
    if args.command == "journals":
        if args.journals_command == "process":
            return emit(args, process_journals(conn, args.workspace, args.profile))
        if args.journals_command == "list":
            return emit(args, list_journal_entries(conn, args.workspace, args.profile, args.limit))
        if args.journals_command == "quarantined":
            return emit(args, list_quarantines(conn, args.workspace, args.profile))
    if args.command == "reports":
        if args.reports_command == "balance-sheet":
            return emit(args, report_balance_sheet(conn, args.workspace, args.profile))
        if args.reports_command == "portfolio-summary":
            return emit(args, report_portfolio_summary(conn, args.workspace, args.profile))
        if args.reports_command == "capital-gains":
            return emit(args, report_capital_gains(conn, args.workspace, args.profile))
        if args.reports_command == "journal-entries":
            return emit(args, report_journal_entries(conn, args.workspace, args.profile))
    raise AppError("Unknown command")


def command_needs_db(args):
    return args.command not in {"backends"}


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    args.runtime_config = load_runtime_config(args.env_file)
    conn = open_db(args.data_root) if command_needs_db(args) else None
    try:
        dispatch(conn, args)
        return 0
    except AppError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        if conn is not None:
            conn.close()

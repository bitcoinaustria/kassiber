from __future__ import annotations

"""Watch-only Silent Payments source support.

This module intentionally stops at receive/accounting. It validates BIP392-ish
watch-only material, keeps the scan descriptor classified as secret-bearing
configuration, and converts local/SP-capable scanner output into the same record
and UTXO shapes the existing sync pipeline already persists.
"""

import json
import re
import hashlib
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..msat import dec
from ..redaction import redact_secret_value
from ..time_utils import UNKNOWN_OCCURRED_AT, parse_iso_datetime_or_none, timestamp_to_iso
from ..util import normalize_chain_value, normalize_network_value, parse_bool, parse_int, str_or_none


WALLET_KIND = "silent-payment"
CONFIG_DESCRIPTOR = "sp_descriptor"
CONFIG_MATERIAL_FORMAT = "sp_material_format"
CONFIG_SCAN_MODE = "sp_scan_mode"
CONFIG_SCAN_START_HEIGHT = "sp_scan_start_height"
CONFIG_SCAN_START_DATE = "sp_scan_start_date"
CONFIG_FULL_HISTORY = "sp_full_history"
CONFIG_FULL_HISTORY_ACK = "sp_acknowledge_full_history_warning"
CONFIG_SERVER_WARNING_ACK = "sp_acknowledge_server_warning"

SCAN_MODE_LOCAL = "local_index"
SCAN_MODE_SERVER = "server_assisted"
SCAN_MODES = {SCAN_MODE_LOCAL, SCAN_MODE_SERVER}

BACKEND_CAPABILITY_FIELDS = ("silent_payments", "sp_capable", "bip352")
BACKEND_SCAN_FILE_FIELDS = ("silent_payment_scan_file", "sp_scan_file")
BACKEND_SCAN_PATH_FIELDS = ("silent_payment_scan_path", "sp_scan_path")

REDACTED_CONFIG_FIELDS = (CONFIG_DESCRIPTOR,)
SAFE_CONFIG_FIELDS = (
    CONFIG_MATERIAL_FORMAT,
    CONFIG_SCAN_MODE,
    CONFIG_SCAN_START_HEIGHT,
    CONFIG_SCAN_START_DATE,
    CONFIG_FULL_HISTORY,
    "sp_scan_start_kind",
)

_BECH32_CHARS = "023456789acdefghjklmnpqrstuvwxyz"
_SPSCAN_RE = re.compile(r"^(?P<hrp>t?spscan)1q[" + _BECH32_CHARS + r"]{8,}$")
_SPSPEND_RE = re.compile(r"^(?P<hrp>t?spspend)1q[" + _BECH32_CHARS + r"]{8,}$")
_PRIVATE_EXTENDED_RE = re.compile(r"^(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}$")
_PUBLIC_EXTENDED_RE = re.compile(r"^(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}$")
_COMPRESSED_PUBKEY_RE = re.compile(r"^(?:02|03)[0-9a-fA-F]{64}$")
_WIF_LIKE_RE = re.compile(r"^[KLc][1-9A-HJ-NP-Za-km-z]{20,}$")
_UNCOMPRESSED_WIF_LIKE_RE = re.compile(r"^[5][1-9A-HJ-NP-Za-km-z]{20,}$")
_TXID_RE = re.compile(r"^[0-9a-fA-F]{64}$")
_P2TR_SCRIPT_RE = re.compile(r"^5120[0-9a-fA-F]{64}$")
_MAINNET_PRIVATE_PREFIX_RE = re.compile(r"^(?:xprv|yprv|zprv|K|L)")
_TESTNET_PRIVATE_PREFIX_RE = re.compile(r"^(?:tprv|uprv|vprv|c)")
_MAINNET_PUBLIC_PREFIX_RE = re.compile(r"^(?:xpub|ypub|zpub)")
_TESTNET_PUBLIC_PREFIX_RE = re.compile(r"^(?:tpub|upub|vpub)")


@dataclass(frozen=True)
class SilentPaymentPlan:
    chain: str
    network: str
    scan_mode: str
    material_format: str
    start_height: int | None
    start_date: str | None
    full_history: bool
    descriptor_fingerprint: str
    gap_limit: int = 0
    kind: str = WALLET_KIND


def has_silent_payment_sync_material(config: Mapping[str, Any] | None) -> bool:
    return bool(isinstance(config, Mapping) and str_or_none(config.get(CONFIG_DESCRIPTOR)))


def normalize_scan_mode(value: Any) -> str:
    text = str(value or SCAN_MODE_LOCAL).strip().lower().replace("-", "_")
    aliases = {
        "local": SCAN_MODE_LOCAL,
        "local_scanner": SCAN_MODE_LOCAL,
        "local_index": SCAN_MODE_LOCAL,
        "server": SCAN_MODE_SERVER,
        "server_assisted": SCAN_MODE_SERVER,
        "server_assisted_scan": SCAN_MODE_SERVER,
    }
    normalized = aliases.get(text)
    if normalized not in SCAN_MODES:
        raise AppError(
            f"Unsupported Silent Payments scan mode '{value}'",
            code="validation",
            hint="Use local-index or server-assisted.",
            retryable=False,
        )
    return normalized


def _compact_descriptor(value: Any) -> str:
    return re.sub(r"\s+", "", str(value or ""))


def descriptor_fingerprint(material: Any) -> str:
    return hashlib.sha256(_compact_descriptor(material).encode("utf-8")).hexdigest()


def _descriptor_args(text: str) -> list[str]:
    if not text.lower().startswith("sp(") or not text.endswith(")"):
        raise AppError(
            "Silent Payments material must be a top-level BIP392 sp(...) descriptor",
            code="validation",
            hint="Use watch-only material such as sp(spscan1q...) or sp(<scan-private>,<spend-public>).",
            retryable=False,
        )
    inner = text[3:-1]
    if not inner:
        raise AppError("sp() requires watch-only key material", code="validation", retryable=False)
    args: list[str] = []
    depth = 0
    start = 0
    for idx, char in enumerate(inner):
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
            if depth < 0:
                raise AppError("Malformed sp() descriptor", code="validation", retryable=False)
        elif char == "," and depth == 0:
            args.append(inner[start:idx])
            start = idx + 1
    if depth != 0:
        raise AppError("Malformed sp() descriptor", code="validation", retryable=False)
    args.append(inner[start:])
    args = [arg.strip() for arg in args if arg.strip()]
    if not args:
        raise AppError("sp() requires watch-only key material", code="validation", retryable=False)
    return args


def _strip_origin(value: str) -> str:
    text = value.strip()
    if text.startswith("["):
        end = text.find("]")
        if end > 0:
            return text[end + 1 :]
    return text


def _network_expected_spscan_hrp(network: str) -> str:
    return "spscan" if network == "main" else "tspscan"


def _private_key_expr(value: str) -> bool:
    text = _strip_origin(value)
    lowered = text.lower()
    return bool(
        _SPSPEND_RE.match(lowered)
        or _PRIVATE_EXTENDED_RE.match(text)
        or _WIF_LIKE_RE.match(text)
        or _UNCOMPRESSED_WIF_LIKE_RE.match(text)
    )


def _public_spend_key_expr(value: str) -> bool:
    text = _strip_origin(value)
    return bool(
        _COMPRESSED_PUBKEY_RE.match(text)
        or _PUBLIC_EXTENDED_RE.match(text)
    )


def _network_for_key_expr(value: str, *, private: bool) -> str | None:
    text = _strip_origin(value)
    if private:
        if _MAINNET_PRIVATE_PREFIX_RE.match(text):
            return "main"
        if _TESTNET_PRIVATE_PREFIX_RE.match(text):
            return "test"
        return None
    if _MAINNET_PUBLIC_PREFIX_RE.match(text):
        return "main"
    if _TESTNET_PUBLIC_PREFIX_RE.match(text):
        return "test"
    return None


def _ensure_key_network(value: str, network: str, *, private: bool, label: str) -> None:
    key_network = _network_for_key_expr(value, private=private)
    if key_network is not None and key_network != network:
        raise AppError(
            f"Silent Payments {label} material is for {key_network}, but wallet network is {network}",
            code="validation",
            hint=f"Use {'mainnet' if network == 'main' else 'testnet'} {label} material for this wallet.",
            retryable=False,
        )


def _validate_spscan_token(value: str, network: str) -> str:
    token = _strip_origin(value).lower()
    if _SPSPEND_RE.match(token):
        raise AppError(
            "Silent Payments spending material is not accepted",
            code="validation",
            hint="Use watch-only spscan material; spspend/full-wallet descriptors are out of scope.",
            retryable=False,
        )
    match = _SPSCAN_RE.match(token)
    if not match:
        raise AppError(
            "sp(KEY) must use watch-only spscan material",
            code="validation",
            hint="Use sp(spscan1q...) for mainnet or sp(tspscan1q...) for test networks.",
            retryable=False,
        )
    expected = _network_expected_spscan_hrp(network)
    if match.group("hrp") != expected:
        raise AppError(
            f"Silent Payments material is for {match.group('hrp')}, but wallet network is {network}",
            code="validation",
            hint=f"Use {expected} material for this network.",
            retryable=False,
        )
    return token


def validate_watch_only_descriptor(material: Any, *, chain: Any = "bitcoin", network: Any = None) -> dict[str, Any]:
    normalized_chain = normalize_chain_value(chain or "bitcoin")
    normalized_network = normalize_network_value(normalized_chain, network)
    if normalized_chain != "bitcoin":
        raise AppError(
            "Silent Payments are only supported for Bitcoin networks",
            code="validation",
            hint="Do not use BIP352/BIP392 material for Liquid wallets.",
            retryable=False,
        )
    text = _compact_descriptor(material)
    args = _descriptor_args(text)
    if any("spspend" in arg.lower() for arg in args):
        raise AppError(
            "Silent Payments spending material is not accepted",
            code="validation",
            hint="Use watch-only spscan material or a private scan key with a public spend key.",
            retryable=False,
        )
    if len(args) == 1:
        _validate_spscan_token(args[0], normalized_network)
        material_format = "bip392-spscan"
    elif len(args) == 2:
        scan_key, spend_key = args
        if not _private_key_expr(scan_key):
            raise AppError(
                "sp(scan, spend) requires private scan material",
                code="validation",
                hint="The scan key may be private because it cannot spend, but the spend key must be public.",
                retryable=False,
            )
        if _UNCOMPRESSED_WIF_LIKE_RE.match(_strip_origin(scan_key)):
            raise AppError(
                "Uncompressed scan private keys are not accepted for BIP352",
                code="validation",
                retryable=False,
            )
        _ensure_key_network(scan_key, normalized_network, private=True, label="scan key")
        if _private_key_expr(spend_key):
            raise AppError(
                "Silent Payments spend-private material is not accepted",
                code="validation",
                hint="Use a public spend key/xpub in the second sp() argument.",
                retryable=False,
        )
        if not _public_spend_key_expr(spend_key):
            raise AppError(
                "sp(scan, spend) requires a public compressed spend key or public extended key",
                code="validation",
                retryable=False,
            )
        _ensure_key_network(spend_key, normalized_network, private=False, label="spend key")
        material_format = "bip392-two-key-watch-only"
    else:
        raise AppError(
            "sp() supports one spscan key or two scan/spend key expressions",
            code="validation",
            retryable=False,
        )
    return {
        CONFIG_DESCRIPTOR: text,
        CONFIG_MATERIAL_FORMAT: material_format,
        "chain": normalized_chain,
        "network": normalized_network,
    }


def validate_wallet_config(config: Mapping[str, Any]) -> dict[str, Any]:
    output = dict(config or {})
    chain = normalize_chain_value(output.get("chain") or "bitcoin")
    network = normalize_network_value(chain, output.get("network"))
    material = str_or_none(output.get(CONFIG_DESCRIPTOR))
    if material is None:
        raise AppError(
            "Silent Payments wallets require --sp-descriptor watch-only material",
            code="validation",
            retryable=False,
        )
    output.update(validate_watch_only_descriptor(material, chain=chain, network=network))
    output[CONFIG_SCAN_MODE] = normalize_scan_mode(output.get(CONFIG_SCAN_MODE))
    start_height = parse_int(output.get(CONFIG_SCAN_START_HEIGHT), None)
    if start_height is not None and start_height < 0:
        raise AppError("Silent Payments scan start height must be non-negative", code="validation", retryable=False)
    if start_height is not None:
        output[CONFIG_SCAN_START_HEIGHT] = start_height
    else:
        output.pop(CONFIG_SCAN_START_HEIGHT, None)
    start_date = str_or_none(output.get(CONFIG_SCAN_START_DATE))
    if start_date is not None and parse_iso_datetime_or_none(start_date) is None:
        raise AppError(
            "Silent Payments scan start date must be an RFC3339 timestamp",
            code="validation",
            retryable=False,
        )
    if start_date is not None:
        output[CONFIG_SCAN_START_DATE] = start_date
    else:
        output.pop(CONFIG_SCAN_START_DATE, None)
    full_history = parse_bool(output.get(CONFIG_FULL_HISTORY), default=False)
    output[CONFIG_FULL_HISTORY] = bool(full_history)
    if not start_height and start_height != 0 and start_date is None and not full_history:
        raise AppError(
            "Silent Payments wallets require a scan start height/date or explicit full-history mode",
            code="silent_payment_scan_start_required",
            hint=(
                "Pass --sp-scan-start-height, --sp-scan-start-date, or "
                "--sp-full-history with --sp-acknowledge-full-history-warning."
            ),
            retryable=False,
        )
    if full_history and not parse_bool(output.get(CONFIG_FULL_HISTORY_ACK), default=False):
        raise AppError(
            "Full-history Silent Payments scans require an explicit warning acknowledgement",
            code="silent_payment_full_history_warning_required",
            hint="Pass --sp-acknowledge-full-history-warning after reviewing runtime and privacy costs.",
            retryable=False,
        )
    if output[CONFIG_SCAN_MODE] == SCAN_MODE_SERVER and not parse_bool(
        output.get(CONFIG_SERVER_WARNING_ACK), default=False
    ):
        raise AppError(
            "Server-assisted Silent Payments scans require an explicit privacy and completeness warning acknowledgement",
            code="silent_payment_server_warning_required",
            hint=(
                "Pass --sp-acknowledge-server-warning after selecting a backend "
                "deliberately and accepting that omitted scan candidates can make "
                "reports incomplete."
            ),
            retryable=False,
        )
    if not str_or_none(output.get("backend")):
        raise AppError(
            "Silent Payments wallets require an explicit --backend",
            code="validation",
            hint="Select an SP-capable local scanner or server-assisted backend explicitly.",
            retryable=False,
        )
    output["chain"] = "bitcoin"
    output["network"] = network
    return output


def build_plan(config: Mapping[str, Any]) -> SilentPaymentPlan:
    validated = validate_wallet_config(config)
    return SilentPaymentPlan(
        chain=validated["chain"],
        network=validated["network"],
        scan_mode=validated[CONFIG_SCAN_MODE],
        material_format=validated[CONFIG_MATERIAL_FORMAT],
        start_height=validated.get(CONFIG_SCAN_START_HEIGHT),
        start_date=validated.get(CONFIG_SCAN_START_DATE),
        full_history=bool(validated.get(CONFIG_FULL_HISTORY)),
        descriptor_fingerprint=descriptor_fingerprint(validated[CONFIG_DESCRIPTOR]),
    )


def is_silent_payment_plan(value: Any) -> bool:
    return getattr(value, "kind", None) == WALLET_KIND


def sync_target(plan: SilentPaymentPlan) -> dict[str, Any]:
    return {
        "chain": plan.chain,
        "network": plan.network,
        "branch_index": None,
        "branch_label": "silent-payment",
        "address_index": None,
        "address": "",
        "unconfidential_address": None,
        "script_pubkey": "",
    }


def backend_supports_silent_payments(backend: Mapping[str, Any]) -> bool:
    capability_declared = False
    for key in BACKEND_CAPABILITY_FIELDS:
        value = _backend_value(backend, key)
        if str_or_none(value) is None:
            continue
        capability_declared = True
        if parse_bool(value, default=False):
            return True
    if capability_declared:
        return False
    return any(str_or_none(_backend_value(backend, key)) for key in BACKEND_SCAN_FILE_FIELDS)


def _backend_value(backend: Mapping[str, Any], key: str) -> Any:
    value = backend.get(key)
    if value in (None, ""):
        config = backend.get("config") if isinstance(backend.get("config"), dict) else None
        if config is not None:
            value = config.get(key)
    return value


def validate_backend_capability(backend: Mapping[str, Any], plan: SilentPaymentPlan, *, kind: str) -> None:
    backend_chain = str_or_none(_backend_value(backend, "chain"))
    if backend_chain and normalize_chain_value(backend_chain) != plan.chain:
        raise AppError(
            f"Backend '{backend.get('name')}' is not configured for Bitcoin Silent Payments",
            code="validation",
            retryable=False,
        )
    backend_network = str_or_none(_backend_value(backend, "network"))
    if backend_network and normalize_network_value(plan.chain, backend_network) != plan.network:
        raise AppError(
            f"Backend '{backend.get('name')}' network does not match the Silent Payments wallet",
            code="validation",
            retryable=False,
        )
    if kind not in {"esplora", "electrum", "bitcoinrpc", "custom"}:
        raise AppError(
            f"Backend kind '{kind}' cannot scan Silent Payments",
            code="silent_payment_backend_unsupported",
            hint="Use a deliberately configured SP-capable backend or local scanner.",
            retryable=False,
        )
    if not backend_supports_silent_payments(backend):
        raise AppError(
            f"Backend '{backend.get('name')}' is not marked Silent Payments capable",
            code="silent_payment_backend_unsupported",
            hint=(
                "Ordinary Esplora/Electrum scripthash sync cannot discover BIP352 outputs. "
                "Configure an SP-capable backend/local scanner explicitly."
            ),
            details={"backend": backend.get("name"), "backend_kind": kind, "sp_capable": False},
            retryable=False,
        )


def _normalize_txid(value: Any) -> str:
    text = str_or_none(value)
    if text is None or not _TXID_RE.match(text):
        raise AppError("Silent Payments scan result contains an invalid txid", code="validation", retryable=False)
    return text.lower()


def _normalize_vout(value: Any) -> int:
    parsed = parse_int(value, None)
    if parsed is None or parsed < 0:
        raise AppError("Silent Payments scan result contains an invalid vout", code="validation", retryable=False)
    return parsed


def _normalize_sats(value: Any, *, field: str) -> int:
    parsed = parse_int(value, None)
    if parsed is None:
        raise AppError(f"Silent Payments scan result is missing {field}", code="validation", retryable=False)
    if parsed < 0:
        raise AppError(f"Silent Payments scan result has a negative {field}", code="validation", retryable=False)
    return parsed


def _amount_sats(row: Mapping[str, Any], *, field: str = "amount_sats") -> int:
    for key in (field, "amount_sat", "value_sats", "value_sat"):
        if key in row:
            return _normalize_sats(row.get(key), field=field)
    if "amount" in row:
        value = dec(row.get("amount"), default="0")
        if value < 0:
            raise AppError(f"Silent Payments scan result has a negative {field}", code="validation", retryable=False)
        return int((value * Decimal("100000000")).to_integral_value())
    raise AppError(f"Silent Payments scan result is missing {field}", code="validation", retryable=False)


def _normalize_script_pubkey(value: Any) -> str:
    text = str_or_none(value)
    if text is None or not _P2TR_SCRIPT_RE.match(text):
        raise AppError(
            "Silent Payments detected outputs must be concrete Taproot scriptPubKeys",
            code="validation",
            retryable=False,
        )
    return text.lower()


def _block_time(tx: Mapping[str, Any], default: str | None = None) -> str | None:
    for key in ("block_time", "confirmed_at", "time", "timestamp", "blocktime"):
        value = tx.get(key)
        if value in (None, "", 0, "0"):
            continue
        try:
            return timestamp_to_iso(value, default=default)
        except (TypeError, ValueError, OSError, OverflowError):
            parsed = parse_iso_datetime_or_none(str(value))
            if parsed is not None:
                return parsed.replace(microsecond=0).isoformat().replace("+00:00", "Z")
    return default


def _block_height(tx: Mapping[str, Any]) -> int | None:
    return parse_int(tx.get("block_height", tx.get("height")), None)


def _confirmations(tx: Mapping[str, Any]) -> int | None:
    value = parse_int(tx.get("confirmations"), None)
    return value if value is None or value >= 0 else None


def _safe_raw(value: Any) -> Any:
    return redact_secret_value(value)


def _tx_outputs(tx: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    raw = tx.get("outputs", tx.get("vout", []))
    return [item for item in raw if isinstance(item, Mapping)] if isinstance(raw, list) else []


def _tx_inputs(tx: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    raw = tx.get("inputs", tx.get("vin", []))
    return [item for item in raw if isinstance(item, Mapping)] if isinstance(raw, list) else []


def _owned_outputs(tx: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    outputs = _tx_outputs(tx)
    return [
        output
        for output in outputs
        if output.get("owned") is True
        or output.get("matched") is True
        or output.get("silent_payment") is True
    ]


def _owned_inputs(tx: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    return [
        item
        for item in _tx_inputs(tx)
        if item.get("owned") is True
        or item.get("matched") is True
        or item.get("silent_payment") is True
    ]


def _record_from_tx(tx: Mapping[str, Any], *, backend_name: str) -> dict[str, Any] | None:
    txid = _normalize_txid(tx.get("txid"))
    received_sats = sum(_amount_sats(output) for output in _owned_outputs(tx))
    spent_sats = sum(_amount_sats(item) for item in _owned_inputs(tx))
    fee_present = "fee_sats" in tx or "fee_sat" in tx
    if spent_sats > 0 and not fee_present:
        raise AppError(
            "Silent Payments spend transactions require fee_sats",
            code="validation",
            hint="Scanner output must include explicit fee_sats so accounting rows remain idempotent.",
            retryable=False,
        )
    fee_sats = parse_int(tx.get("fee_sats", tx.get("fee_sat")), 0) or 0
    if received_sats == 0 and spent_sats == 0:
        return None
    if received_sats > spent_sats:
        direction = "inbound"
        amount_sats = received_sats - spent_sats
        fee_record_sats = 0
        kind = "deposit"
    else:
        direction = "outbound"
        amount_sats = max(spent_sats - received_sats - max(fee_sats, 0), 0)
        fee_record_sats = max(fee_sats, 0)
        kind = "withdrawal" if amount_sats > 0 else "fee"
    occurred_at = _block_time(tx, default=UNKNOWN_OCCURRED_AT)
    confirmed_at = _block_time(tx, default=None)
    return {
        "txid": txid,
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": "BTC",
        "amount": dec(amount_sats) / Decimal("100000000"),
        "fee": dec(fee_record_sats) / Decimal("100000000"),
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Silent Payment sync from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(json_ready(_safe_raw(tx)), sort_keys=True),
    }


def _utxo_from_output(
    output: Mapping[str, Any],
    tx: Mapping[str, Any],
    *,
    chain: str,
    network: str,
) -> dict[str, Any]:
    txid = _normalize_txid(output.get("txid") or tx.get("txid"))
    vout = _normalize_vout(output.get("vout", output.get("n")))
    block_height = parse_int(output.get("block_height", output.get("height")), None)
    if block_height is None:
        block_height = _block_height(tx)
    block_time = str_or_none(output.get("block_time")) or _block_time(tx, default=None)
    confirmations = parse_int(output.get("confirmations"), None)
    if confirmations is None:
        confirmations = _confirmations(tx)
    spent_by = str_or_none(output.get("spent_by", output.get("spentBy")))
    return {
        "txid": txid,
        "vout": vout,
        "asset": "BTC",
        "amount_sats": _amount_sats(output),
        "confirmation_status": "confirmed" if block_height and block_height > 0 else "mempool",
        "confirmations": confirmations,
        "block_height": block_height,
        "block_time": block_time,
        "chain": chain,
        "network": network,
        "address": "",
        "script_pubkey": _normalize_script_pubkey(output.get("script_pubkey", output.get("scriptPubKey"))),
        "address_label": "Silent Payment",
        "branch_label": "silent-payment",
        "branch_index": None,
        "address_index": None,
        "spent_by": spent_by,
        "spent": bool(spent_by) or parse_bool(output.get("spent"), default=False),
        "raw": {
            "source": "silent_payment_scan",
            **_safe_raw(output.get("raw") if isinstance(output.get("raw"), Mapping) else {}),
        },
    }


def _scan_transactions(payload: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    raw = payload.get("transactions", payload.get("txs", []))
    return [item for item in raw if isinstance(item, Mapping)] if isinstance(raw, list) else []


def _scan_utxo_rows(payload: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    raw = payload.get("utxos", payload.get("outputs", []))
    return [item for item in raw if isinstance(item, Mapping)] if isinstance(raw, list) else []


def _outpoint_from_mapping(row: Mapping[str, Any], fallback_txid: Any = None) -> str:
    txid = _normalize_txid(row.get("txid") or fallback_txid)
    vout = _normalize_vout(row.get("vout", row.get("n")))
    return f"{txid}:{vout}"


def _payload_binding(payload: Mapping[str, Any]) -> Mapping[str, Any]:
    wallet = payload.get("wallet")
    return wallet if isinstance(wallet, Mapping) else payload


def _validate_payload_binding(
    payload: Mapping[str, Any],
    *,
    plan: SilentPaymentPlan,
    wallet_id: str | None,
    wallet_label: str | None,
) -> None:
    binding = _payload_binding(payload)
    actual_fingerprint = str_or_none(
        binding.get("descriptor_fingerprint")
        or binding.get("sp_descriptor_fingerprint")
        or binding.get("wallet_fingerprint")
    )
    if actual_fingerprint is not None:
        if actual_fingerprint.lower() != plan.descriptor_fingerprint:
            raise AppError(
                "Silent Payments scanner output is for a different wallet",
                code="silent_payment_scan_wallet_mismatch",
                retryable=False,
            )
        return
    actual_wallet_id = str_or_none(binding.get("wallet_id") or binding.get("kassiber_wallet_id"))
    if actual_wallet_id is not None and wallet_id is not None:
        if actual_wallet_id != wallet_id:
            raise AppError(
                "Silent Payments scanner output wallet_id does not match this wallet",
                code="silent_payment_scan_wallet_mismatch",
                retryable=False,
            )
        return
    actual_wallet_label = str_or_none(binding.get("wallet_label") or binding.get("label"))
    if actual_wallet_label is not None and wallet_label is not None:
        if actual_wallet_label != wallet_label:
            raise AppError(
                "Silent Payments scanner output wallet_label does not match this wallet",
                code="silent_payment_scan_wallet_mismatch",
                retryable=False,
            )
        return
    raise AppError(
        "Silent Payments scanner output must be bound to the wallet",
        code="silent_payment_scan_wallet_mismatch",
        hint="Include descriptor_fingerprint, wallet_id, or wallet_label in the scanner JSON.",
        retryable=False,
    )


def _range_start_height(range_payload: Mapping[str, Any]) -> int | None:
    for key in ("from_height", "start_height", "from_block_height", "start_block_height"):
        if key in range_payload:
            return parse_int(range_payload.get(key), None)
    return None


def _range_start_date(range_payload: Mapping[str, Any]) -> str | None:
    for key in ("from_date", "start_date", "from_time", "start_time"):
        value = str_or_none(range_payload.get(key))
        if value is not None:
            return value
    return None


def _scan_range_covers_plan(range_payload: Mapping[str, Any], plan: SilentPaymentPlan) -> tuple[bool, str | None]:
    from_height = _range_start_height(range_payload)
    if plan.full_history:
        if from_height is None or from_height > 0:
            return False, "scan_range_incomplete"
    if plan.start_height is not None:
        if from_height is None or from_height > plan.start_height:
            return False, "scan_range_incomplete"
    if plan.start_date is not None:
        from_date = _range_start_date(range_payload)
        parsed_from = parse_iso_datetime_or_none(from_date)
        parsed_start = parse_iso_datetime_or_none(plan.start_date)
        if parsed_from is None or parsed_start is None or parsed_from > parsed_start:
            return False, "scan_range_incomplete"
    return True, None


def normalize_scan_payload(
    payload: Mapping[str, Any],
    *,
    backend_name: str,
    backend_kind: str,
    plan: SilentPaymentPlan,
    checkpoint: Mapping[str, Any] | None = None,
    wallet_id: str | None = None,
    wallet_label: str | None = None,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    if not isinstance(payload, Mapping):
        raise AppError("Silent Payments scanner returned an unexpected payload", code="validation", retryable=True)
    _validate_payload_binding(payload, plan=plan, wallet_id=wallet_id, wallet_label=wallet_label)
    txs = _scan_transactions(payload)
    records = [
        record
        for record in (
            _record_from_tx(tx, backend_name=backend_name)
            for tx in txs
        )
        if record is not None
    ]
    owned_output_outpoints = {
        _outpoint_from_mapping(output, tx.get("txid"))
        for tx in txs
        for output in _owned_outputs(tx)
    }
    spent_by_outpoint = {
        _outpoint_from_mapping(input_row): _normalize_txid(tx.get("txid"))
        for tx in txs
        for input_row in _owned_inputs(tx)
    }
    outputs_by_outpoint: dict[str, dict[str, Any]] = {}
    tx_by_id = {str(tx.get("txid") or "").lower(): tx for tx in txs}
    for row in _scan_utxo_rows(payload):
        tx = tx_by_id.get(str(row.get("txid") or "").lower(), {})
        outpoint = _outpoint_from_mapping(row, tx.get("txid"))
        if outpoint not in owned_output_outpoints:
            raise AppError(
                "Silent Payments UTXO rows must correspond to owned transaction outputs",
                code="validation",
                hint="Mark detected transaction outputs with silent_payment=true/owned=true/matched=true.",
                retryable=False,
            )
        utxo = _utxo_from_output(row, tx, chain=plan.chain, network=plan.network)
        outputs_by_outpoint[outpoint] = utxo
    for tx in txs:
        for output in _owned_outputs(tx):
            if output.get("script_pubkey") or output.get("scriptPubKey"):
                utxo = _utxo_from_output(output, tx, chain=plan.chain, network=plan.network)
                outputs_by_outpoint.setdefault(f"{utxo['txid']}:{utxo['vout']}", utxo)
    for outpoint, spending_txid in spent_by_outpoint.items():
        if outpoint in outputs_by_outpoint:
            outputs_by_outpoint[outpoint]["spent_by"] = spending_txid
            outputs_by_outpoint[outpoint]["spent"] = True
    complete = parse_bool(payload.get("complete", payload.get("scan_complete")), default=False)
    range_payload = payload.get("range") if isinstance(payload.get("range"), Mapping) else {}
    range_covers, range_reason = _scan_range_covers_plan(range_payload, plan)
    complete = complete and range_covers
    degraded_reason = str_or_none(payload.get("degraded_reason", payload.get("stop_reason"))) or range_reason
    degraded = parse_bool(payload.get("degraded"), default=False) or not complete
    next_checkpoint = dict(checkpoint or {})
    next_checkpoint.update(
        {
            "backend": {"name": backend_name, "kind": backend_kind},
            "silent_payment": {
                "scan_complete": complete,
                "degraded": degraded,
                "degraded_reason": degraded_reason or ("scan_incomplete" if not complete else None),
                "scan_mode": plan.scan_mode,
                "material_format": plan.material_format,
                "start_height": plan.start_height,
                "start_date": plan.start_date,
                "full_history": plan.full_history,
                "range": _safe_raw(range_payload),
            },
        }
    )
    meta = {
        "freshness_checkpoint": next_checkpoint,
        "silent_payment_scan_complete": complete,
        "silent_payment_degraded": degraded,
        "silent_payment_degraded_reason": degraded_reason or ("scan_incomplete" if not complete else ""),
        "silent_payment_transactions_seen": len(_scan_transactions(payload)),
        "silent_payment_outputs_seen": len(outputs_by_outpoint),
    }
    if degraded:
        meta["partial_success"] = True
        meta["blocking_reports"] = True
        meta["utxos_skipped_partial"] = True
    else:
        meta["utxos"] = list(outputs_by_outpoint.values())
    return records, meta

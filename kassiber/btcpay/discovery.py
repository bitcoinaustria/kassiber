"""Inspect a BTCPay connection: server, key permissions, stores, and wallets.

Inspection is a user-triggered read of the merchant's server. It answers the
questions the setup flow has to explain:

- Which BTCPay version is this, and is it fully synced?
- What can this API key read, per store, and does it grant more than
  Kassiber needs?
- Which payment methods does each store accept, and what does each one mean
  for bookkeeping (on-chain wallet, Lightning node, plugin, altcoin)?
- Which stores pay into the same on-chain wallet?

The last answer comes from the read-only address preview
(``/payment-methods/{id}/wallet/preview``): the first receive addresses of a
store wallet. Their hash becomes a ``wallet_fingerprint`` shared by every
store that uses the same wallet. The addresses themselves stay in
``InspectionResult.private`` for local ownership matching and are never part
of the public payload.
"""

from __future__ import annotations

import hashlib
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from typing import Any, Mapping
from urllib import parse as urlparse

from ..errors import AppError
from .client import (
    GreenfieldClient,
    greenfield_app_error,
    is_loopback_url,
    is_onion_url,
    scrub_api_key_payload,
)
from .payment_methods import (
    RAIL_ONCHAIN,
    canonical_payment_method_id,
    classify_payment_method,
)
from .permissions import (
    CAPABILITY_PERMISSIONS,
    KeyGrant,
    grant_summary,
    parse_key_grant,
    permission_label,
)

PREVIEW_ADDRESS_COUNT = 5
MAX_STORE_WORKERS = 4

# Address prefix -> (chain, network). Legacy base58 prefixes are ambiguous
# between test networks and are left undetermined.
_SEGWIT_HRP_NETWORKS = (
    ("bcrt1", "bitcoin", "regtest"),
    ("bc1", "bitcoin", "main"),
    ("tb1", "bitcoin", "test"),
    ("tlq1", "liquid", "liquidtestnet"),
    ("tex1", "liquid", "liquidtestnet"),
    ("lq1", "liquid", "liquidv1"),
    ("ex1", "liquid", "liquidv1"),
    ("el1", "liquid", "elementsregtest"),
    ("ert1", "liquid", "elementsregtest"),
)


@dataclass
class InspectionResult:
    public: dict[str, Any]
    private: dict[str, Any] = field(default_factory=dict)
    grant: KeyGrant = field(default_factory=lambda: KeyGrant(known=False))


def network_from_address(address: str) -> tuple[str | None, str | None]:
    value = str(address or "").strip().lower()
    for prefix, chain, network in _SEGWIT_HRP_NETWORKS:
        if value.startswith(prefix):
            return chain, network
    return None, None


def wallet_fingerprint(addresses: list[str]) -> str | None:
    cleaned = sorted({str(address).strip() for address in addresses if str(address or "").strip()})
    if not cleaned:
        return None
    digest = hashlib.sha256("\n".join(cleaned).encode("utf-8")).hexdigest()
    return f"btcpay-wallet:{digest[:24]}"


def _major_version(version: str | None) -> int | None:
    try:
        return int(str(version or "").split(".", 1)[0])
    except ValueError:
        return None


def _store_entry(raw: Mapping[str, Any]) -> dict[str, Any]:
    store_id = raw.get("id") or raw.get("storeId")
    if not store_id:
        raise AppError("BTCPay store record is missing 'id'", code="protocol_error")
    return {
        "id": str(store_id),
        "name": str(raw.get("name") or raw.get("label") or store_id),
        "default_currency": raw.get("defaultCurrency"),
        "archived": bool(raw.get("archived", False)),
    }


def _payment_method_rows(raw: Any) -> list[dict[str, Any]]:
    """Accept BTCPay 2.x list responses and 1.x dict responses."""

    rows: list[dict[str, Any]] = []
    if isinstance(raw, list):
        for item in raw:
            if isinstance(item, Mapping):
                rows.append(dict(item))
    elif isinstance(raw, Mapping):
        for key, value in raw.items():
            if isinstance(value, Mapping):
                rows.append({"paymentMethodId": key, **dict(value)})
            else:
                rows.append({"paymentMethodId": key, "enabled": bool(value)})
    return rows


def _payment_method_id(raw: Mapping[str, Any]) -> str | None:
    method_id = raw.get("paymentMethodId") or raw.get("paymentMethod") or raw.get("id")
    if not method_id:
        crypto_code = raw.get("cryptoCode") or raw.get("currency")
        payment_type = str(raw.get("paymentType") or "").lower()
        if crypto_code and ("chain" in payment_type or "onchain" in payment_type):
            method_id = f"{crypto_code}-CHAIN"
        elif crypto_code and "lightning" in payment_type:
            method_id = f"{crypto_code}-LN"
    return canonical_payment_method_id(method_id) if method_id else None


def normalize_payment_method(store_id: str, raw: Mapping[str, Any]) -> dict[str, Any] | None:
    method_id = _payment_method_id(raw)
    if not method_id:
        return None
    info = classify_payment_method(method_id)
    label = raw.get("name") or raw.get("label") or info["label"]
    return {
        "store_id": store_id,
        "payment_method_id": method_id,
        "label": str(label),
        "enabled": bool(raw.get("enabled", True)),
        "sync_supported": info["wallet_history_supported"],
        "rail": info["rail"],
        "currency": info["currency"],
        "chain": info["chain"],
        "bitcoin_asset": info["bitcoin_asset"],
        "settlement": info["settlement"],
        "settlement_hint": info["settlement_hint"],
        "ledger_group": info["ledger_group"],
    }


def _preview_paths(store_id: str, method: Mapping[str, Any], major_version: int | None) -> list[str]:
    store_q = urlparse.quote(store_id, safe="")
    method_q = urlparse.quote(method["payment_method_id"], safe="")
    paths = [f"/api/v1/stores/{store_q}/payment-methods/{method_q}/wallet/preview"]
    if major_version is not None and major_version < 2:
        code_q = urlparse.quote(method["currency"], safe="")
        paths.append(f"/api/v1/stores/{store_q}/payment-methods/onchain/{code_q}/preview")
    return paths


def _fetch_preview(client: GreenfieldClient, store_id: str, method: Mapping[str, Any], major_version: int | None, count: int):
    last_error = None
    for path in _preview_paths(store_id, method, major_version):
        payload, error = client.get_optional_json(path, {"offset": 0, "count": count})
        if error is None:
            addresses = []
            rows = payload.get("addresses") if isinstance(payload, Mapping) else None
            for row in rows or []:
                if isinstance(row, Mapping) and row.get("address"):
                    addresses.append(str(row["address"]))
            return addresses, None
        last_error = error
        if error.status != 404:
            break
    return [], last_error


def _inspect_store(
    client: GreenfieldClient,
    store: dict[str, Any],
    grant: KeyGrant,
    major_version: int | None,
    preview_count: int,
) -> tuple[dict[str, Any], list[dict[str, Any]], dict[str, list[str]], list[dict[str, Any]]]:
    store_id = store["id"]
    capabilities = grant.capabilities(store_id) if grant.known else {key: None for key in CAPABILITY_PERMISSIONS}
    warnings: list[dict[str, Any]] = []
    methods: list[dict[str, Any]] = []
    previews: dict[str, list[str]] = {}
    store = dict(store)
    store["capabilities"] = capabilities
    store["missing_permissions"] = sorted(
        {
            CAPABILITY_PERMISSIONS[capability]
            for capability, available in capabilities.items()
            if available is False and capability != "wallet_history"
        }
    )
    store["payment_methods_error"] = None
    if capabilities.get("store_catalog") is False:
        store["payment_methods_error"] = "missing_permission"
        warnings.append(
            {
                "code": "store_catalog_unavailable",
                "store_id": store_id,
                "message": (
                    f"The key cannot list payment methods for {store['name']}. "
                    f"Add '{permission_label(CAPABILITY_PERMISSIONS['store_catalog'])}' so Kassiber can "
                    "tell which wallets this store uses."
                ),
            }
        )
        return store, methods, previews, warnings
    store_q = urlparse.quote(store_id, safe="")
    raw_methods, error = client.get_optional_json(
        f"/api/v1/stores/{store_q}/payment-methods",
        {"onlyEnabled": "true"},
    )
    if error is not None:
        store["payment_methods_error"] = "missing_permission" if error.status == 403 else f"http_{error.status}"
        if error.status == 403 and grant.known is False:
            store["capabilities"] = {**capabilities, "store_catalog": False, "wallet_preview": False}
        return store, methods, previews, warnings
    for raw in _payment_method_rows(raw_methods):
        method = normalize_payment_method(store_id, raw)
        if method is None:
            continue
        method["wallet_history_available"] = (
            (capabilities.get("wallet_history") if method["sync_supported"] else False)
        )
        method["wallet_fingerprint"] = None
        method["network"] = None
        method["preview_error"] = None
        if (
            method["rail"] == RAIL_ONCHAIN
            and method["bitcoin_asset"]
            and method["enabled"]
            and capabilities.get("wallet_preview") is not False
        ):
            addresses, preview_error = _fetch_preview(client, store_id, method, major_version, preview_count)
            if addresses:
                previews[method["payment_method_id"]] = addresses
                method["wallet_fingerprint"] = wallet_fingerprint(addresses)
                chain, network = network_from_address(addresses[0])
                if chain == method["chain"]:
                    method["network"] = network
            elif preview_error is not None:
                method["preview_error"] = (
                    "missing_permission" if preview_error.status == 403 else f"http_{preview_error.status}"
                )
        methods.append(method)
    return store, methods, previews, warnings


def inspect_btcpay_connection(
    client: GreenfieldClient,
    *,
    preview_count: int = PREVIEW_ADDRESS_COUNT,
    include_archived: bool = False,
    max_workers: int = MAX_STORE_WORKERS,
) -> InspectionResult:
    warnings: list[dict[str, Any]] = []

    # 401 on server info means the key itself is rejected; fail fast.
    server_info, server_error = client.get_optional_json(
        "/api/v1/server/info", tolerate=(403, 404, 405)
    )
    server = {
        "version": None,
        "major_version": None,
        "fully_synced": None,
        "supported_payment_methods": [],
        "tor": is_onion_url(client.base_url),
        "loopback": is_loopback_url(client.base_url),
        "transport": "https" if client.base_url.startswith("https://") else "http",
    }
    if isinstance(server_info, Mapping):
        version = str(server_info.get("version") or "") or None
        server.update(
            {
                "version": version,
                "major_version": _major_version(version),
                "fully_synced": server_info.get("fullySynched"),
                "supported_payment_methods": [
                    canonical_payment_method_id(item)
                    for item in server_info.get("supportedPaymentMethods") or []
                    if item
                ],
            }
        )
        if server_info.get("fullySynched") is False:
            warnings.append(
                {
                    "code": "server_not_synced",
                    "message": "BTCPay Server is still syncing its node; recent payments may be missing.",
                }
            )
    elif server_error is not None and server_error.status not in (403, 404, 405):
        raise greenfield_app_error(server_error)
    if server["transport"] == "http" and not server["tor"] and not server["loopback"]:
        warnings.append(
            {
                "code": "unencrypted_transport",
                "message": (
                    "This connection uses http://. The API key and store data travel unencrypted; "
                    "use https:// or an onion address."
                ),
            }
        )

    key_payload, key_error = client.get_optional_json("/api/v1/api-keys/current", tolerate=(403, 404, 405))
    key_info = scrub_api_key_payload(key_payload) if key_error is None else {}
    grant = parse_key_grant(key_info.get("permissions") if key_error is None else None)
    api_key = {"label": key_info.get("label"), **grant_summary(grant)}
    risk = api_key["risk"]
    if risk == "server_admin":
        warnings.append(
            {
                "code": "key_server_admin",
                "message": (
                    "This key has server-wide admin rights. Kassiber only reads store data; "
                    "create a narrower key and revoke this one."
                ),
            }
        )
    elif risk == "can_modify_store":
        warnings.append(
            {
                "code": "key_can_modify_store",
                "message": (
                    "This key can modify the store, including the wallet customers pay into. "
                    "Kassiber only needs that for BTCPay's own wallet history."
                ),
            }
        )
    elif risk == "can_write":
        warnings.append(
            {
                "code": "key_can_write",
                "message": (
                    "This key can create or change data (for example invoices or payouts). "
                    "Kassiber only reads; a read-only key is safer."
                ),
            }
        )

    raw_stores, stores_error = client.get_optional_json("/api/v1/stores", tolerate=(403, 404))
    stores: list[dict[str, Any]] = []
    if stores_error is None and isinstance(raw_stores, list):
        for raw in raw_stores:
            if isinstance(raw, Mapping):
                entry = _store_entry(raw)
                if entry["archived"] and not include_archived:
                    continue
                stores.append(entry)
    elif stores_error is not None and stores_error.status == 404:
        raise AppError(
            "This URL does not look like a BTCPay Server (the store list returned HTTP 404)",
            code="not_found",
            hint="Check the server URL; enter the address you open BTCPay with, without /api/v1.",
        )
    if not stores and grant.known and grant.store_permissions:
        # The key cannot list stores but is scoped to known store ids.
        stores = [
            {"id": store_id, "name": store_id, "default_currency": None, "archived": False}
            for store_id in grant.scoped_store_ids
        ]
    if not stores:
        warnings.append(
            {
                "code": "no_store_access",
                "message": (
                    "The key cannot see any store. Grant it 'View your stores' for the stores Kassiber "
                    "should read."
                ),
            }
        )

    major_version = server["major_version"]
    results: list[tuple] = []
    if stores:
        workers = max(1, min(max_workers, len(stores)))
        if workers == 1:
            results = [_inspect_store(client, store, grant, major_version, preview_count) for store in stores]
        else:
            with ThreadPoolExecutor(max_workers=workers, thread_name_prefix="btcpay-inspect") as pool:
                futures = [
                    pool.submit(_inspect_store, client, store, grant, major_version, preview_count)
                    for store in stores
                ]
                results = [future.result() for future in futures]

    public_stores = []
    payment_methods = []
    private_previews: dict[str, dict[str, list[str]]] = {}
    for store, methods, previews, store_warnings in results:
        public_stores.append(store)
        payment_methods.extend(methods)
        if previews:
            private_previews[store["id"]] = previews
        warnings.extend(store_warnings)

    fingerprint_members: dict[str, list[dict[str, str]]] = {}
    for method in payment_methods:
        fingerprint = method.get("wallet_fingerprint")
        if fingerprint:
            fingerprint_members.setdefault(fingerprint, []).append(
                {"store_id": method["store_id"], "payment_method_id": method["payment_method_id"]}
            )
    shared_wallets = [
        {"wallet_fingerprint": fingerprint, "members": members}
        for fingerprint, members in sorted(fingerprint_members.items())
        if len(members) > 1
    ]
    if shared_wallets:
        store_names = {store["id"]: store["name"] for store in public_stores}
        for group in shared_wallets:
            names = ", ".join(sorted({store_names.get(member["store_id"], member["store_id"]) for member in group["members"]}))
            warnings.append(
                {
                    "code": "shared_store_wallet",
                    "wallet_fingerprint": group["wallet_fingerprint"],
                    "message": (
                        f"{names} pay into the same wallet. Kassiber imports that wallet once and "
                        "keeps each store's invoices as provenance."
                    ),
                }
            )

    public = {
        "server": server,
        "api_key": api_key,
        "stores": public_stores,
        "payment_methods": payment_methods,
        "shared_wallets": shared_wallets,
        "warnings": warnings,
    }
    return InspectionResult(public=public, private={"previews": private_previews}, grant=grant)


__all__ = [
    "InspectionResult",
    "PREVIEW_ADDRESS_COUNT",
    "inspect_btcpay_connection",
    "network_from_address",
    "normalize_payment_method",
    "wallet_fingerprint",
]

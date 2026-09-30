"""BTCPay setup planning shared by the CLI, desktop daemon, and AI tools.

``kassiber.btcpay.discovery`` reads the merchant's server. This module joins
that inspection with the local book and decides, per store payment method,
what Kassiber should do with it:

- ``wallet_source``: import BTCPay's own wallet history as a Kassiber wallet
  (requires the wallet-history key).
- ``existing_wallet``: the store pays into a wallet Kassiber already tracks
  (recognised from the read-only address preview, or chosen by the user);
  BTCPay enriches it with labels and invoice provenance.
- ``payment_ledger``: the store settles into something Kassiber cannot watch
  (a Lightning node, a plugin rail, a layer-two wallet). BTCPay's settled
  payments and completed payouts become the balance source.
- ``provenance_only``: keep invoices and payouts as provenance; balances come
  from another connection (a Lightning node Kassiber tracks, a Liquid wallet)
  or from another store that shares this wallet.
- ``skip``: ignore (non-bitcoin assets, disabled methods, network mismatch,
  or a store already served through another API key).

The recommendation is advisory. Wallet ownership suggestions never grant
ownership on their own; the user confirms every route, and the create path
re-validates the choices so two routes cannot import the same wallet twice.
"""

from __future__ import annotations

import json
import sqlite3
from typing import Any, Iterable, Mapping, Sequence

from ..btcpay.client import server_url_identity
from ..btcpay.discovery import InspectionResult, network_from_address
from ..btcpay.payment_methods import (
    RAIL_LIGHTNING,
    RAIL_LNURL,
    RAIL_ONCHAIN,
    canonical_payment_method_id,
    classify_payment_method,
    ledger_group,
    payment_method_label,
)
from ..errors import AppError
from . import btcpay_ledger as core_btcpay_ledger
from . import commercial as core_commercial
from . import wallets as core_wallets

ACTIONS = ("wallet_source", "existing_wallet", "payment_ledger", "provenance_only", "skip")
BALANCE_ACTIONS = frozenset({"wallet_source", "existing_wallet", "payment_ledger"})
FINGERPRINT_CONFIG_KEY = "btcpay_wallet_fingerprint"
OWNERSHIP_SCAN_TO_INDEX = 25
LIGHTNING_WALLET_KINDS = ("lnd", "coreln", "nwc", "phoenix")

_ENVIRONMENT_NETWORKS = {
    "main": {"bitcoin": {"main"}, "liquid": {"liquidv1"}},
    "test": {"bitcoin": {"test"}, "liquid": {"liquidtestnet"}},
    "signet": {"bitcoin": {"test", "signet"}},
    "regtest": {"bitcoin": {"regtest"}, "liquid": {"elementsregtest"}},
}

REASONS = {
    "already_configured": "Already set up in Kassiber.",
    "configured_via_other_key": "Already set up through another API key for this server.",
    "non_bitcoin_asset": "Kassiber tracks Bitcoin and Liquid Bitcoin only.",
    "disabled": "This payment method is disabled in BTCPay.",
    "lightning_settles_elsewhere": (
        "A Lightning node is already connected to this book, so Kassiber keeps these invoices as provenance "
        "and links them by payment hash. Choose the payment ledger if that node is not this store's."
    ),
    "lightning_payment_ledger": (
        "Kassiber cannot watch this store's Lightning node or wallet directly, so it books BTCPay's "
        "settled payments and completed payouts as the balance."
    ),
    "plugin_payment_ledger": (
        "Plugin payment method: Kassiber books BTCPay's settled payments and completed payouts as the balance."
    ),
    "plugin_rail": "Plugin payment method in an unknown currency; invoices are kept as provenance.",
    "not_ledger_rail": "Only Lightning and Bitcoin plugin payment methods can be booked from BTCPay's payments.",
    "payouts_permission_missing": (
        "The key cannot read payouts, so BTCPay's payments alone would overstate the balance. Keep the invoices "
        "only, or use a key that can read payouts."
    ),
    "configured_balance_source": (
        "Kassiber already takes this payment method's balance from a wallet set up here. Archive that wallet to "
        "set it up differently."
    ),
    "mapped_to_wallet": (
        "This payment method is mapped to a wallet Kassiber tracks. Skip the mapping first so the wallet is not "
        "counted twice."
    ),
    "network_mismatch": "This store's wallet is on a different network than this book.",
    "wallet_recognised": "Kassiber already tracks the wallet this store pays into.",
    "shared_wallet_already_imported": "Another store already imports this same wallet.",
    "shared_wallet": "Another store in this setup pays into the same wallet; it is imported once.",
    "import_btcpay_wallet_history": "Import BTCPay's wallet history for this store.",
    "read_only_key_upgrade": (
        "This key cannot read BTCPay's wallet history. Create a wallet-history key to import the store "
        "wallet from BTCPay, or map it to a wallet Kassiber already tracks."
    ),
    "wallet_history_permission_missing": "Needs a key with BTCPay wallet-history access.",
    "invoices_permission_missing": "The key cannot read invoices for this store.",
    "not_on_chain": "Only on-chain Bitcoin and Liquid can be imported as a wallet.",
}


def _config(row: Mapping[str, Any]) -> dict[str, Any]:
    try:
        decoded = json.loads(row["config_json"] or "{}")
    except (TypeError, ValueError):
        return {}
    return decoded if isinstance(decoded, dict) else {}


def same_server_backends(runtime_config: Mapping[str, Any], server_url: str | None) -> list[str]:
    """Saved BTCPay backends (API keys) that point at the same server."""

    if not server_url:
        return []
    identity = server_url_identity(server_url)
    names = []
    for name, backend in (runtime_config.get("backends") or {}).items():
        if not isinstance(backend, Mapping):
            continue
        if str(backend.get("kind") or "").strip().lower() != "btcpay":
            continue
        if server_url_identity(backend.get("url")) == identity:
            names.append(str(backend.get("name") or name).lower())
    return sorted(set(names))


def list_existing_routes(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    backend_names: Iterable[str],
) -> list[dict[str, Any]]:
    """Wallet sources, wallet mappings, and account routes for ``backend_names``."""

    wanted = {name.strip().lower() for name in backend_names if name}
    routes: list[dict[str, Any]] = []
    if not wanted:
        return routes
    rows = conn.execute(
        "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? ORDER BY label ASC",
        (profile_id,),
    ).fetchall()
    for row in rows:
        config = _config(row)
        if (
            str(config.get("sync_source") or "") == core_wallets.BTCPAY_SYNC_SOURCE
            and str(config.get("backend") or "").strip().lower() in wanted
            and config.get("store_id")
            and core_btcpay_ledger.is_payment_ledger_config(config)
        ):
            for payment_method_id in core_btcpay_ledger.ledger_payment_method_ids(config):
                routes.append(
                    {
                        "action": "payment_ledger",
                        "backend": str(config.get("backend")).strip().lower(),
                        "wallet": row["label"],
                        "wallet_id": row["id"],
                        "store_id": str(config.get("store_id")),
                        "payment_method_id": payment_method_id,
                        "wallet_fingerprint": None,
                        "deprecated": core_wallets.wallet_is_deprecated(config),
                    }
                )
        elif (
            str(config.get("sync_source") or "") == core_wallets.BTCPAY_SYNC_SOURCE
            and str(config.get("backend") or "").strip().lower() in wanted
            and config.get("store_id")
        ):
            routes.append(
                {
                    "action": "wallet_source",
                    "backend": str(config.get("backend")).strip().lower(),
                    "wallet": row["label"],
                    "wallet_id": row["id"],
                    "store_id": str(config.get("store_id")),
                    "payment_method_id": canonical_payment_method_id(
                        config.get("payment_method_id") or core_wallets.BTCPAY_DEFAULT_PAYMENT_METHOD_ID
                    ),
                    "wallet_fingerprint": config.get(FINGERPRINT_CONFIG_KEY),
                    "deprecated": core_wallets.wallet_is_deprecated(config),
                }
            )
        try:
            provenance_routes = core_wallets.wallet_btcpay_provenance_config(config)
        except AppError:
            provenance_routes = []
        for route in provenance_routes:
            if route["backend"] not in wanted:
                continue
            routes.append(
                {
                    "action": "existing_wallet",
                    "backend": route["backend"],
                    "wallet": row["label"],
                    "wallet_id": row["id"],
                    "store_id": route["store_id"],
                    "payment_method_id": route["payment_method_id"],
                    "wallet_fingerprint": None,
                    "deprecated": core_wallets.wallet_is_deprecated(config),
                }
            )
    for backend_name in sorted(wanted):
        for route in core_commercial.list_btcpay_account_routes(conn, profile_id, backend_name=backend_name):
            routes.append(
                {
                    "action": route["action"],
                    "backend": route["backend"],
                    "route_id": route["id"],
                    "wallet": None,
                    "wallet_id": None,
                    "store_id": route["store_id"],
                    "payment_method_id": route["payment_method_id"],
                    "label": route.get("label") or "",
                    "wallet_fingerprint": None,
                    "deprecated": False,
                }
            )
    return routes


def wallet_source_fingerprints(conn: sqlite3.Connection, profile_id: str) -> dict[str, list[dict[str, Any]]]:
    """BTCPay wallet sources grouped by the store-wallet fingerprint they import."""

    grouped: dict[str, list[dict[str, Any]]] = {}
    rows = conn.execute(
        "SELECT id, label, config_json FROM wallets WHERE profile_id = ?",
        (profile_id,),
    ).fetchall()
    for row in rows:
        config = _config(row)
        fingerprint = config.get(FINGERPRINT_CONFIG_KEY)
        if not fingerprint or str(config.get("sync_source") or "") != core_wallets.BTCPAY_SYNC_SOURCE:
            continue
        if core_wallets.wallet_is_deprecated(config):
            continue
        grouped.setdefault(str(fingerprint), []).append(
            {
                "wallet": row["label"],
                "wallet_id": row["id"],
                "backend": str(config.get("backend") or "").lower(),
                "store_id": str(config.get("store_id") or ""),
                "payment_method_id": canonical_payment_method_id(config.get("payment_method_id") or "BTC-CHAIN"),
            }
        )
    return grouped


def _book_network(conn: sqlite3.Connection, profile_id: str) -> dict[str, Any]:
    from .book_network import resolve_book_environment

    try:
        binding = resolve_book_environment(conn, profile_id)
    except sqlite3.OperationalError:
        return {"state": "unbound", "environment": None}
    return {"state": binding.get("state"), "environment": binding.get("environment")}


def network_compatible(book: Mapping[str, Any], chain: str | None, network: str | None) -> bool | None:
    if not chain or not network or book.get("state") != "bound":
        return None
    allowed = _ENVIRONMENT_NETWORKS.get(str(book.get("environment") or ""), {}).get(chain)
    if allowed is None:
        return False
    return network in allowed


def _ownership_matches(
    conn: sqlite3.Connection,
    profile_id: str,
    previews: Mapping[str, Mapping[str, Sequence[str]]],
) -> dict[tuple[str, str], dict[str, Any]]:
    addresses: list[str] = []
    for methods in previews.values():
        for values in methods.values():
            addresses.extend(values)
    if not addresses:
        return {}
    from . import ownership as core_ownership

    try:
        report = core_ownership.identify(
            conn,
            profile_id,
            addresses=sorted(set(addresses)),
            scan_to_index=OWNERSHIP_SCAN_TO_INDEX,
        )
    except AppError:
        return {}
    owned: dict[str, dict[str, Any]] = {}
    for result in report.get("results") or []:
        if result.get("status") != "owned" or result.get("ownership_ambiguous"):
            continue
        owned[str(result.get("input"))] = {
            "wallet": result.get("canonical_wallet"),
            "wallet_id": result.get("canonical_wallet_id"),
        }
    matches: dict[tuple[str, str], dict[str, Any]] = {}
    for store_id, methods in previews.items():
        for payment_method_id, values in methods.items():
            hits = [owned[address] for address in values if address in owned]
            wallet_ids = {hit["wallet_id"] for hit in hits}
            if len(wallet_ids) == 1 and hits:
                matches[(store_id, payment_method_id)] = {
                    **hits[0],
                    "addresses_matched": len(hits),
                    "addresses_checked": len(values),
                }
    return matches


def _lightning_wallets(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    placeholders = ",".join("?" for _ in LIGHTNING_WALLET_KINDS)
    rows = conn.execute(
        f"SELECT id, label, kind FROM wallets WHERE profile_id = ? AND kind IN ({placeholders}) ORDER BY label",
        (profile_id, *LIGHTNING_WALLET_KINDS),
    ).fetchall()
    return [{"wallet": row["label"], "wallet_id": row["id"], "kind": row["kind"]} for row in rows]


def _action_availability(method: Mapping[str, Any], capabilities: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    onchain = method.get("rail") == RAIL_ONCHAIN and bool(method.get("sync_supported"))
    invoices = capabilities.get("invoices")
    history = capabilities.get("wallet_history")
    network_ok = method.get("network_compatible") is not False

    def entry(available: bool, reason: str | None = None) -> dict[str, Any]:
        return {"available": available, "reason": reason, "reason_text": REASONS.get(reason) if reason else None}

    wallet_source = entry(True)
    if not onchain:
        wallet_source = entry(False, "not_on_chain")
    elif history is False:
        wallet_source = entry(False, "wallet_history_permission_missing")
    elif not network_ok:
        wallet_source = entry(False, "network_mismatch")
    existing_wallet = entry(onchain and network_ok, None if onchain and network_ok else ("network_mismatch" if onchain else "not_on_chain"))
    provenance = entry(invoices is not False, None if invoices is not False else "invoices_permission_missing")
    if not method.get("ledger_group"):
        ledger = entry(False, "not_ledger_rail")
    elif invoices is False:
        ledger = entry(False, "invoices_permission_missing")
    elif capabilities.get("payouts") is False:
        ledger = entry(False, "payouts_permission_missing")
    else:
        ledger = entry(True)
    return {
        "wallet_source": wallet_source,
        "existing_wallet": existing_wallet,
        "payment_ledger": ledger,
        "provenance_only": provenance,
        "skip": entry(True),
    }


def _balance_conflict(existing: str, requested: str, *, same_key: bool) -> str | None:
    """Why ``requested`` cannot join a store method already set up as ``existing``.

    A wallet source or payment ledger keeps booking until its wallet is
    archived, so a later choice on the same key cannot replace it; a mapped
    wallet already holds the balance, so importing the store too would count
    it twice. Choices through another key never add a second balance source.
    """

    if existing in {"wallet_source", "payment_ledger"}:
        if same_key and requested != existing:
            return "configured_balance_source"
        if not same_key and requested in BALANCE_ACTIONS:
            return "configured_via_other_key"
    if existing == "existing_wallet" and requested in {"wallet_source", "payment_ledger"}:
        return "mapped_to_wallet" if same_key else "configured_via_other_key"
    return None


def _lock_configured_actions(
    actions: dict[str, dict[str, Any]],
    own_routes: Sequence[Mapping[str, Any]],
    other_routes: Sequence[Mapping[str, Any]],
) -> None:
    routes = [(route, True) for route in own_routes] + [(route, False) for route in other_routes]
    for action, availability in actions.items():
        if not availability.get("available"):
            continue
        for route, same_key in routes:
            if route.get("deprecated"):
                continue
            reason = _balance_conflict(route["action"], action, same_key=same_key)
            if reason:
                availability.update({"available": False, "reason": reason, "reason_text": REASONS.get(reason)})
                break


def plan_btcpay_setup(
    conn: sqlite3.Connection,
    profile_id: str,
    inspection: InspectionResult,
    *,
    backend_name: str,
    runtime_config: Mapping[str, Any],
    server_url: str | None,
) -> dict[str, Any]:
    """Join a server inspection with the local book into a reviewable plan."""

    public = json.loads(json.dumps(inspection.public))
    backend_key = backend_name.strip().lower()
    sibling_backends = [name for name in same_server_backends(runtime_config, server_url) if name != backend_key]
    existing = list_existing_routes(conn, profile_id, backend_names=[backend_key, *sibling_backends])
    existing_by_key: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for route in existing:
        existing_by_key.setdefault((route["store_id"], route["payment_method_id"]), []).append(route)
    book = _book_network(conn, profile_id)
    previews = (inspection.private or {}).get("previews") or {}
    ownership = _ownership_matches(conn, profile_id, previews)
    imported_fingerprints = wallet_source_fingerprints(conn, profile_id)
    lightning_wallets = _lightning_wallets(conn, profile_id)
    capabilities_by_store = {store["id"]: store.get("capabilities") or {} for store in public.get("stores") or []}
    store_names = {store["id"]: store.get("name") or store["id"] for store in public.get("stores") or []}

    # Order so the first store of a shared-wallet group is the one that imports it.
    methods = public.get("payment_methods") or []
    order = sorted(
        range(len(methods)),
        key=lambda index: (store_names.get(methods[index]["store_id"], "").lower(), methods[index]["store_id"], methods[index]["payment_method_id"]),
    )
    planned_fingerprints: dict[str, dict[str, Any]] = {}
    for index in order:
        method = methods[index]
        key = (method["store_id"], method["payment_method_id"])
        capabilities = capabilities_by_store.get(method["store_id"], {})
        compatible = network_compatible(book, method.get("chain"), method.get("network"))
        method["network_compatible"] = compatible
        routes_here = existing_by_key.get(key, [])
        own_routes = [route for route in routes_here if route["backend"] == backend_key]
        other_routes = [route for route in routes_here if route["backend"] != backend_key]
        method["existing_routes"] = own_routes
        method["configured_via"] = sorted({route["backend"] for route in other_routes})
        method["owned_by"] = ownership.get(key)
        fingerprint = method.get("wallet_fingerprint")
        imported = [
            route
            for route in imported_fingerprints.get(fingerprint or "", [])
            if not (route["store_id"] == method["store_id"] and route["payment_method_id"] == method["payment_method_id"])
        ]
        method["same_wallet_imported_by"] = imported
        shared_with = [
            member
            for group in public.get("shared_wallets") or []
            if group["wallet_fingerprint"] == fingerprint
            for member in group["members"]
            if (member["store_id"], member["payment_method_id"]) != key
        ]
        method["shared_with"] = shared_with
        method["actions"] = _action_availability(method, capabilities)
        _lock_configured_actions(method["actions"], own_routes, other_routes)
        if method.get("rail") in {RAIL_LIGHTNING, RAIL_LNURL}:
            method["lightning_wallets"] = lightning_wallets

        action, wallet, reason = "skip", None, None
        if own_routes:
            preferred = sorted(own_routes, key=lambda route: ACTIONS.index(route["action"]) if route["action"] in ACTIONS else 9)[0]
            action, wallet, reason = preferred["action"], preferred.get("wallet"), "already_configured"
        elif any(route["action"] in BALANCE_ACTIONS for route in other_routes):
            # Another key already books this store's balance. Invoice-only
            # routes on another key do not block upgrading to a balance source.
            action, reason = "skip", "configured_via_other_key"
        elif not method.get("bitcoin_asset"):
            action, reason = "skip", "non_bitcoin_asset"
        elif not method.get("enabled", True):
            action, reason = "skip", "disabled"
        elif method.get("rail") in {RAIL_LIGHTNING, RAIL_LNURL}:
            if lightning_wallets:
                action, reason = "provenance_only", "lightning_settles_elsewhere"
            else:
                action, reason = "payment_ledger", "lightning_payment_ledger"
        elif method.get("rail") != RAIL_ONCHAIN or not method.get("sync_supported"):
            if method.get("ledger_group"):
                action, reason = "payment_ledger", "plugin_payment_ledger"
            else:
                action, reason = "provenance_only", "plugin_rail"
        elif compatible is False:
            action, reason = "skip", "network_mismatch"
        elif method["owned_by"]:
            action, wallet, reason = "existing_wallet", method["owned_by"]["wallet"], "wallet_recognised"
        elif imported:
            action, reason = "provenance_only", "shared_wallet_already_imported"
        elif fingerprint and fingerprint in planned_fingerprints:
            action, reason = "provenance_only", "shared_wallet"
        elif capabilities.get("wallet_history") is False:
            action, reason = "provenance_only", "read_only_key_upgrade"
        else:
            action, reason = "wallet_source", "import_btcpay_wallet_history"
        if action == "payment_ledger" and capabilities.get("payouts") is False:
            action, reason = "provenance_only", "payouts_permission_missing"
        if action in {"provenance_only", "payment_ledger"} and capabilities.get("invoices") is False:
            action, reason = "skip", "invoices_permission_missing"
        if fingerprint and action in {"wallet_source", "existing_wallet"}:
            planned_fingerprints.setdefault(fingerprint, {"store_id": method["store_id"], "payment_method_id": method["payment_method_id"]})
        method["recommendation"] = {
            "action": action,
            "wallet": wallet,
            "reason": reason,
            "reason_text": REASONS.get(reason) if reason else None,
        }

    own_existing = [route for route in existing if route["backend"] == backend_key and not route.get("deprecated")]
    configured_stores = sorted({route["store_id"] for route in own_existing})
    sync_states = core_commercial.btcpay_store_sync_states(
        conn,
        profile_id,
        [(backend_key, store_id) for store_id in configured_stores],
    )
    stale = []
    for store in public.get("stores") or []:
        state = sync_states.get((backend_key, store["id"]))
        store["sync_state"] = state
        if state and state["stale"]:
            stale.append(store["id"])
    if stale:
        public.setdefault("warnings", []).append(
            {
                "code": "stale_store_data",
                "severity": "info",
                "message": (
                    "BTCPay does not notify Kassiber of new invoices or payouts. Data for "
                    f"{len(stale)} configured store(s) is over a day old or its last refresh failed; "
                    "sync to bring it up to date."
                ),
                "store_ids": stale,
            }
        )
    public["backend"] = backend_key
    public["sibling_backends"] = sibling_backends
    public["existing_routes"] = [route for route in existing if route["backend"] == backend_key]
    public["book_network"] = book
    public["lightning_wallets"] = lightning_wallets
    public["detected_network"] = detected_network(public)[1]
    public["summary"] = _plan_summary(public)
    return public


def _plan_summary(plan: Mapping[str, Any]) -> dict[str, Any]:
    counts = {action: 0 for action in ACTIONS}
    for method in plan.get("payment_methods") or []:
        action = (method.get("recommendation") or {}).get("action")
        if action in counts:
            counts[action] += 1
    return {
        "stores": len(plan.get("stores") or []),
        "payment_methods": len(plan.get("payment_methods") or []),
        "recommended": counts,
        "key_risk": (plan.get("api_key") or {}).get("risk"),
    }


def validate_setup_routes(
    conn: sqlite3.Connection,
    profile_id: str,
    routes: Sequence[Mapping[str, Any]],
    *,
    backend_name: str,
    runtime_config: Mapping[str, Any],
    server_url: str | None,
) -> None:
    """Refuse route sets that would import one store wallet twice."""

    backend_key = backend_name.strip().lower()
    sibling_backends = [name for name in same_server_backends(runtime_config, server_url) if name != backend_key]
    existing_by_key: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for existing in list_existing_routes(conn, profile_id, backend_names=[backend_key, *sibling_backends]):
        if not existing.get("deprecated"):
            existing_by_key.setdefault((existing["store_id"], existing["payment_method_id"]), []).append(existing)
    for route in routes:
        for existing in existing_by_key.get((route["store_id"], route["payment_method_id"]), []):
            reason = _balance_conflict(existing["action"], route["action"], same_key=existing["backend"] == backend_key)
            if reason:
                raise AppError(
                    f"Store {route['store_id']} {route['payment_method_id']} is already set up as {existing['action']}"
                    + (f" ('{existing['wallet']}')" if existing.get("wallet") else ""),
                    code="conflict",
                    hint=REASONS[reason],
                    details={
                        "reason": reason,
                        "existing_action": existing["action"],
                        "existing_wallet": existing.get("wallet"),
                        "existing_backend": existing["backend"],
                    },
                    retryable=False,
                )
    if sibling_backends:
        sibling_routes = list_existing_routes(conn, profile_id, backend_names=sibling_backends)
        sibling_sources = {
            (route["store_id"], route["payment_method_id"]): route
            for route in sibling_routes
            if route["action"] == "wallet_source" and not route.get("deprecated")
        }
        sibling_ledgers = {
            (route["store_id"], ledger_group(route["payment_method_id"])): route
            for route in sibling_routes
            if route["action"] == "payment_ledger" and not route.get("deprecated")
        }
        for route in routes:
            if route.get("action") == "payment_ledger":
                clash = sibling_ledgers.get((route["store_id"], ledger_group(route["payment_method_id"])))
                if clash:
                    raise AppError(
                        f"Store {route['store_id']} {route['payment_method_id']} payments are already booked by '{clash['wallet']}' through another API key",
                        code="conflict",
                        hint="Keep one payment ledger per store rail, or keep this key's routes as provenance only.",
                        details={"existing_wallet": clash["wallet"], "existing_backend": clash["backend"]},
                        retryable=False,
                    )
                continue
            if route.get("action") != "wallet_source":
                continue
            clash = sibling_sources.get((route["store_id"], route["payment_method_id"]))
            if clash:
                raise AppError(
                    f"Store {route['store_id']} {route['payment_method_id']} is already imported as '{clash['wallet']}' through another API key",
                    code="conflict",
                    hint="Keep one wallet source per store wallet. Map this store to the existing wallet or skip it.",
                    details={"existing_wallet": clash["wallet"], "existing_backend": clash["backend"]},
                    retryable=False,
                )
    imported = wallet_source_fingerprints(conn, profile_id)
    seen: dict[str, Mapping[str, Any]] = {}
    for route in routes:
        if route.get("action") != "wallet_source":
            continue
        fingerprint = route.get("wallet_fingerprint")
        if not fingerprint:
            continue
        previous = seen.get(fingerprint)
        if previous is not None:
            raise AppError(
                "Two selected stores pay into the same wallet; importing both would double-count it",
                code="conflict",
                hint="Import the wallet once and keep the other store as invoice provenance.",
                details={
                    "stores": sorted({previous["store_id"], route["store_id"]}),
                    "wallet_fingerprint": fingerprint,
                },
                retryable=False,
            )
        seen[fingerprint] = route
        for existing in imported.get(fingerprint, []):
            if existing["store_id"] == route["store_id"] and existing["payment_method_id"] == route["payment_method_id"]:
                continue
            raise AppError(
                f"This store's wallet is already imported as '{existing['wallet']}'",
                code="conflict",
                hint="Keep this store as invoice provenance, or map it to that wallet.",
                details={"existing_wallet": existing["wallet"], "wallet_fingerprint": fingerprint},
                retryable=False,
            )


def wallet_source_config(
    *,
    backend_name: str,
    store_id: str,
    payment_method_id: str,
    wallet_fingerprint: str | None = None,
    network: str | None = None,
) -> dict[str, Any]:
    """Config for a new BTCPay wallet source, scoped to the payment method's chain."""

    info = classify_payment_method(payment_method_id)
    config: dict[str, Any] = {
        "backend": backend_name,
        "store_id": store_id,
        "payment_method_id": info["payment_method_id"],
        "sync_source": core_wallets.BTCPAY_SYNC_SOURCE,
    }
    if info.get("chain"):
        config["chain"] = info["chain"]
    if network:
        config["network"] = network
    if wallet_fingerprint:
        config[FINGERPRINT_CONFIG_KEY] = wallet_fingerprint
    return config


def detected_network(inspection_public: Mapping[str, Any]) -> tuple[str | None, str | None]:
    """The Bitcoin network the inspected server runs on, if it is unambiguous."""

    networks = {
        (method.get("chain"), method.get("network"))
        for method in inspection_public.get("payment_methods") or []
        if method.get("network") and method.get("chain") == "bitcoin"
    }
    if len(networks) == 1:
        return next(iter(networks))
    return None, None


__all__ = [
    "ACTIONS",
    "BALANCE_ACTIONS",
    "FINGERPRINT_CONFIG_KEY",
    "REASONS",
    "detected_network",
    "list_existing_routes",
    "network_compatible",
    "network_from_address",
    "plan_btcpay_setup",
    "same_server_backends",
    "validate_setup_routes",
    "wallet_source_config",
    "wallet_source_fingerprints",
]


# ---------------------------------------------------------------------------
# Applying a reviewed route set (shared by the desktop daemon and the CLI)
# ---------------------------------------------------------------------------

_ACTION_ALIASES = {
    "create": "wallet_source",
    "create_wallet": "wallet_source",
    "wallet": "wallet_source",
    "map": "existing_wallet",
    "map_existing": "existing_wallet",
    "settlement_wallet": "existing_wallet",
    "provenance": "provenance_only",
    "invoice_provenance": "provenance_only",
    "ledger": "payment_ledger",
    "payments": "payment_ledger",
}
ROUTE_NETWORKS = ("main", "test", "signet", "regtest", "liquidv1", "liquidtestnet", "elementsregtest")


def _route_text(raw: Mapping[str, Any], *keys: str) -> str | None:
    for key in keys:
        value = raw.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def normalize_setup_routes(raw_routes: Any, *, allow_only_skips: bool = False) -> list[dict[str, Any]]:
    """Validate route objects from the desktop or CLI into canonical routes."""

    from ..btcpay.payment_methods import require_wallet_history_payment_method

    if not isinstance(raw_routes, list) or not raw_routes:
        raise AppError(
            "BTCPay account setup requires at least one route",
            code="validation",
            hint="Discover stores, then choose what Kassiber should do with each payment method.",
            retryable=False,
        )
    routes: list[dict[str, Any]] = []
    seen = set()
    for raw in raw_routes:
        if not isinstance(raw, Mapping):
            raise AppError("BTCPay account setup routes must be objects", code="validation", retryable=False)
        action = (_route_text(raw, "action", "mode") or "skip").lower()
        action = _ACTION_ALIASES.get(action, action)
        if action not in ACTIONS:
            raise AppError(f"Unsupported BTCPay account route action '{action}'", code="validation", retryable=False)
        store_id = _route_text(raw, "store_id")
        if store_id is None:
            raise AppError("BTCPay store ID is required", code="validation", retryable=False)
        store_id = core_wallets.normalize_btcpay_store_id(store_id)
        payment_method_id = core_wallets.normalize_btcpay_payment_method_id(
            _route_text(raw, "payment_method_id") or core_wallets.BTCPAY_DEFAULT_PAYMENT_METHOD_ID
        )
        if action in {"wallet_source", "existing_wallet"}:
            require_wallet_history_payment_method(payment_method_id)
        if action == "payment_ledger" and not ledger_group(payment_method_id):
            raise AppError(
                f"BTCPay payment method '{payment_method_id}' cannot be booked as a payment ledger",
                code="validation",
                hint="Import on-chain methods as wallet history or map them to a tracked wallet; payment ledgers are for Lightning and Bitcoin plugin rails.",
                retryable=False,
            )
        wallet_ref = _route_text(raw, "wallet", "target_wallet")
        if action == "existing_wallet" and wallet_ref is None:
            raise AppError("BTCPay account settlement routes require a wallet", code="validation", retryable=False)
        network = _route_text(raw, "network")
        if network is not None and network not in ROUTE_NETWORKS:
            raise AppError(f"Unsupported BTCPay route network '{network}'", code="validation", retryable=False)
        key = (store_id, payment_method_id, action, wallet_ref or "")
        if key in seen:
            continue
        seen.add(key)
        routes.append(
            {
                "store_id": store_id,
                "store_name": _route_text(raw, "store_name"),
                "payment_method_id": payment_method_id,
                "label": _route_text(raw, "label"),
                "action": action,
                "wallet": wallet_ref,
                "wallet_fingerprint": _route_text(raw, "wallet_fingerprint"),
                "network": network,
            }
        )
    if not allow_only_skips and not any(route["action"] != "skip" for route in routes):
        raise AppError(
            "BTCPay account setup has no selected routes",
            code="validation",
            hint="Choose at least one wallet source, settlement mapping, payment ledger, or provenance-only route.",
            retryable=False,
        )
    return routes


def wallet_source_label(base_label: str, route: Mapping[str, Any]) -> str:
    if route.get("label"):
        return str(route["label"]).strip()
    store_part = (route.get("store_name") or route["store_id"]).strip() or route["store_id"]
    return f"{base_label} - {store_part} - {route['payment_method_id']}"


def _find_wallet_source(conn, profile_id, *, backend_name, store_id, payment_method_id):
    rows = conn.execute(
        "SELECT * FROM wallets WHERE profile_id = ? AND kind = 'custom' ORDER BY label ASC",
        (profile_id,),
    ).fetchall()
    for row in rows:
        config = _config(row)
        if (
            str(config.get("sync_source") or "") == core_wallets.BTCPAY_SYNC_SOURCE
            and not core_btcpay_ledger.is_payment_ledger_config(config)
            and str(config.get("backend") or "").lower() == backend_name.lower()
            and str(config.get("store_id") or "") == store_id
            and canonical_payment_method_id(config.get("payment_method_id") or "BTC-CHAIN") == payment_method_id
        ):
            return row
    return None


def _find_payment_ledger(conn, profile_id, *, backend_name, store_id, group):
    rows = conn.execute(
        "SELECT * FROM wallets WHERE profile_id = ? AND kind = 'custom' ORDER BY label ASC",
        (profile_id,),
    ).fetchall()
    for row in rows:
        config = _config(row)
        if (
            core_btcpay_ledger.is_payment_ledger_config(config)
            and str(config.get("backend") or "").lower() == backend_name.lower()
            and str(config.get("store_id") or "") == store_id
            and group in core_btcpay_ledger.ledger_groups(core_btcpay_ledger.ledger_payment_method_ids(config))
        ):
            return row
    return None


def payment_ledger_label(base_label: str, routes: Sequence[Mapping[str, Any]], group: str) -> str:
    for route in routes:
        if route.get("label"):
            return str(route["label"]).strip()
    first = routes[0]
    store_part = (first.get("store_name") or first["store_id"]).strip() or first["store_id"]
    rail = "Lightning" if group == "lightning" else payment_method_label(first["payment_method_id"])
    return f"{base_label} - {store_part} - {rail}"


def payment_ledger_config(
    *,
    backend_name: str,
    store_id: str,
    payment_method_ids: Sequence[str],
    network: str | None = None,
) -> dict[str, Any]:
    methods = sorted(set(payment_method_ids))
    config: dict[str, Any] = {
        "backend": backend_name,
        "store_id": store_id,
        "payment_method_id": methods[0],
        "sync_source": core_wallets.BTCPAY_SYNC_SOURCE,
        core_btcpay_ledger.SOURCE_MODE_CONFIG_KEY: core_btcpay_ledger.LEDGER_SOURCE_MODE,
        core_btcpay_ledger.LEDGER_PAYMENT_METHODS_CONFIG_KEY: methods,
        "chain": "bitcoin",
    }
    if network:
        config["network"] = network
    return config


def _create_or_extend_payment_ledger(conn, profile, *, label, backend_name, store_id, group, routes):
    methods = [route["payment_method_id"] for route in routes]
    network = next((route.get("network") for route in routes if route.get("network")), None)
    existing = _find_payment_ledger(conn, str(profile["id"]), backend_name=backend_name, store_id=store_id, group=group)
    if existing is not None:
        config = _config(existing)
        current = core_btcpay_ledger.ledger_payment_method_ids(config)
        merged = sorted(set(current) | set(methods))
        if merged == sorted(current):
            return core_wallets.wallet_row_to_dict(existing), True
        wallet = core_wallets.update_wallet(
            conn,
            profile["workspace_id"],
            profile["id"],
            existing["id"],
            {"config": {core_btcpay_ledger.LEDGER_PAYMENT_METHODS_CONFIG_KEY: merged}},
            commit=False,
        )
        return wallet, True
    if conn.execute("SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?", (profile["id"], label)).fetchone():
        raise AppError(
            f"Wallet '{label}' already exists in profile '{profile['label']}'",
            code="conflict",
            hint="Choose a different connection label or skip this already-used route.",
            details={"existing_labels": [label]},
            retryable=False,
        )
    wallet = core_wallets.create_wallet(
        conn,
        profile["workspace_id"],
        profile["id"],
        label,
        "custom",
        config=payment_ledger_config(
            backend_name=backend_name,
            store_id=store_id,
            payment_method_ids=methods,
            network=network,
        ),
        commit=False,
    )
    return wallet, False


def _create_or_reuse_wallet_source(conn, profile, *, label, backend_name, route):
    existing = _find_wallet_source(
        conn,
        str(profile["id"]),
        backend_name=backend_name,
        store_id=route["store_id"],
        payment_method_id=route["payment_method_id"],
    )
    if existing is not None:
        return core_wallets.wallet_row_to_dict(existing), True
    if conn.execute("SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?", (profile["id"], label)).fetchone():
        raise AppError(
            f"Wallet '{label}' already exists in profile '{profile['label']}'",
            code="conflict",
            hint="Choose a different connection label or skip this already-used route.",
            details={"existing_labels": [label]},
            retryable=False,
        )
    wallet = core_wallets.create_wallet(
        conn,
        profile["workspace_id"],
        profile["id"],
        label,
        "custom",
        config=wallet_source_config(
            backend_name=backend_name,
            store_id=route["store_id"],
            payment_method_id=route["payment_method_id"],
            wallet_fingerprint=route.get("wallet_fingerprint"),
            network=route.get("network"),
        ),
        commit=False,
    )
    return wallet, False


def _set_wallet_routes(conn, profile, wallet_id, routes):
    update = (
        {"clear": [core_wallets.BTCPAY_PROVENANCE_CONFIG_KEY]}
        if not routes
        else {"config": {core_wallets.BTCPAY_PROVENANCE_CONFIG_KEY: routes}}
    )
    return core_wallets.update_wallet(
        conn, profile["workspace_id"], profile["id"], wallet_id, update, commit=False
    )


def apply_setup_routes(
    conn: sqlite3.Connection,
    workspace: Mapping[str, Any],
    profile: Mapping[str, Any],
    *,
    backend_name: str,
    routes: Sequence[Mapping[str, Any]],
    label: str,
) -> dict[str, Any]:
    """Create wallet sources, wallet mappings, and provenance routes atomically.

    The caller validates the plan first (``validate_setup_routes``) and owns
    the network refresh afterwards; this function performs no network I/O and
    commits once, so a failure leaves the book unchanged.
    """

    backend_key = backend_name.strip().lower()
    wallet_sources: list[dict[str, Any]] = []
    mappings: list[dict[str, Any]] = []
    account_routes: list[dict[str, Any]] = []
    payment_ledgers: list[dict[str, Any]] = []
    ledger_routes: dict[tuple[str, str], list[Mapping[str, Any]]] = {}
    skipped: list[dict[str, Any]] = []
    reused = 0
    provenance_stores: set[str] = set()
    try:
        for route in routes:
            action = route["action"]
            store_id = route["store_id"]
            payment_method_id = route["payment_method_id"]
            if action != "provenance_only":
                core_commercial.delete_btcpay_account_route(
                    conn,
                    profile["id"],
                    backend_name=backend_key,
                    store_id=store_id,
                    payment_method_id=payment_method_id,
                )
            if action == "skip":
                skipped.append(dict(route))
                if route.get("wallet"):
                    wallet = core_wallets.get_wallet_details(conn, profile["workspace_id"], profile["id"], route["wallet"])
                    existing = list(wallet.get("config", {}).get(core_wallets.BTCPAY_PROVENANCE_CONFIG_KEY) or [])
                    remaining = [
                        item
                        for item in existing
                        if not (
                            str(item.get("backend") or "").strip().lower() == backend_key
                            and item.get("store_id") == store_id
                            and canonical_payment_method_id(item.get("payment_method_id") or "BTC-CHAIN") == payment_method_id
                        )
                    ]
                    if len(remaining) != len(existing):
                        _set_wallet_routes(conn, profile, wallet["id"], remaining)
                continue
            if action == "wallet_source":
                wallet, was_reused = _create_or_reuse_wallet_source(
                    conn,
                    profile,
                    label=wallet_source_label(label, route),
                    backend_name=backend_key,
                    route=route,
                )
                wallet_sources.append(wallet)
                reused += int(was_reused)
                provenance_stores.add(store_id)
                continue
            if action == "existing_wallet":
                wallet = core_wallets.get_wallet_details(conn, profile["workspace_id"], profile["id"], route["wallet"])
                if str(wallet.get("config", {}).get("sync_source") or "") == core_wallets.BTCPAY_SYNC_SOURCE:
                    raise AppError(
                        f"Wallet '{wallet['label']}' is itself a BTCPay wallet source",
                        code="validation",
                        hint="Map BTCPay stores onto the wallet that holds the funds (a descriptor, xpub, or Liquid wallet).",
                        retryable=False,
                    )
                existing = list(wallet.get("config", {}).get(core_wallets.BTCPAY_PROVENANCE_CONFIG_KEY) or [])
                next_route = {"backend": backend_key, "store_id": store_id, "payment_method_id": payment_method_id}
                if next_route not in existing:
                    existing.append(next_route)
                mappings.append({"wallet": _set_wallet_routes(conn, profile, wallet["id"], existing), "route": next_route})
                provenance_stores.add(store_id)
                continue
            if action == "payment_ledger":
                ledger_routes.setdefault((store_id, ledger_group(payment_method_id) or payment_method_id), []).append(route)
                provenance_stores.add(store_id)
                continue
            account_routes.append(
                core_commercial.upsert_btcpay_account_route(
                    conn,
                    workspace,
                    profile,
                    backend_name=backend_key,
                    store_id=store_id,
                    payment_method_id=payment_method_id,
                    action="provenance_only",
                    label=route.get("store_name") or route.get("label"),
                )
            )
            provenance_stores.add(store_id)
        for (store_id, group), grouped in sorted(ledger_routes.items()):
            wallet, was_reused = _create_or_extend_payment_ledger(
                conn,
                profile,
                label=payment_ledger_label(label, grouped, group),
                backend_name=backend_key,
                store_id=store_id,
                group=group,
                routes=grouped,
            )
            payment_ledgers.append(wallet)
            reused += int(was_reused)
        conn.commit()
    except BaseException:
        conn.rollback()
        raise
    wallets = wallet_sources + payment_ledgers + [mapping["wallet"] for mapping in mappings]
    return {
        "wallet": wallets[0] if wallets else None,
        "wallets": wallets,
        "wallet_sources": wallet_sources,
        "payment_ledgers": payment_ledgers,
        "reused_wallets": reused,
        "mappings": mappings,
        "account_routes": account_routes,
        "skipped": skipped,
        "provenance_store_ids": sorted(provenance_stores),
    }


def routes_from_plan(
    plan: Mapping[str, Any],
    *,
    overrides: Mapping[tuple[str, str], Mapping[str, Any]] | None = None,
    use_recommendations: bool = True,
) -> list[dict[str, Any]]:
    """Turn a reviewed plan into create routes (recommendations plus overrides)."""

    overrides = overrides or {}
    routes = []
    for method in plan.get("payment_methods") or []:
        key = (method["store_id"], method["payment_method_id"])
        override = overrides.get(key)
        recommendation = method.get("recommendation") or {}
        if override is not None:
            action = override.get("action")
            wallet = override.get("wallet") or (recommendation.get("wallet") if action == "existing_wallet" else None)
        elif use_recommendations and recommendation.get("action") not in (None, "skip"):
            action = recommendation["action"]
            wallet = recommendation.get("wallet")
        else:
            continue
        availability = (method.get("actions") or {}).get(action) or {}
        if availability and availability.get("available") is False:
            raise AppError(
                f"{method['payment_method_id']} for store {method['store_id']} cannot use '{action}'",
                code="validation",
                hint=availability.get("reason_text") or REASONS.get(availability.get("reason") or "", ""),
                details={"reason": availability.get("reason")},
                retryable=False,
            )
        store_name = next(
            (store.get("name") for store in plan.get("stores") or [] if store.get("id") == method["store_id"]),
            None,
        )
        routes.append(
            {
                "store_id": method["store_id"],
                "store_name": store_name,
                "payment_method_id": method["payment_method_id"],
                "action": action,
                "wallet": wallet,
                "wallet_fingerprint": method.get("wallet_fingerprint"),
                # Lightning and plugin rails expose no addresses; ledgers take
                # the server's on-chain network.
                "network": method.get("network")
                or (plan.get("detected_network") if action == "payment_ledger" else None),
            }
        )
    unknown = set(overrides) - {(m["store_id"], m["payment_method_id"]) for m in plan.get("payment_methods") or []}
    if unknown:
        store_id, payment_method_id = sorted(unknown)[0]
        raise AppError(
            f"Store {store_id} has no enabled payment method {payment_method_id} visible to this key",
            code="not_found",
            hint="Run `kassiber btcpay inspect --backend NAME` to list the stores and payment methods.",
            retryable=False,
        )
    return routes

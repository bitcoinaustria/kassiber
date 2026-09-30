"""BTCPay Greenfield permission model for Kassiber's read-only use.

Greenfield permissions are strings such as ``btcpay.store.canviewinvoices``
that may be scoped to one store with a ``:<storeId>`` suffix. Broader
permissions include narrower ones; the inclusion table below mirrors
``GET /misc/permissions`` on BTCPay Server 2.3.

Kassiber needs very little:

- ``btcpay.store.canviewstoresettings`` is read-only and already includes
  invoices, payment requests, payouts, pull payments, and reports. It also
  unlocks the store catalog, enabled payment methods, and the wallet address
  preview Kassiber uses to recognise which wallet a store pays into.
- BTCPay's own wallet history endpoints are gated behind
  ``btcpay.store.canmodifystoresettings``. That permission can also change the
  store, including where it sends customer payments, so Kassiber treats it as
  an explicit opt-in and never recommends it by default.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping, Sequence
from urllib import parse as urlparse

from ..errors import AppError

VIEW_STORE_SETTINGS = "btcpay.store.canviewstoresettings"
MODIFY_STORE_SETTINGS = "btcpay.store.canmodifystoresettings"
VIEW_INVOICES = "btcpay.store.canviewinvoices"
VIEW_PAYMENT_REQUESTS = "btcpay.store.canviewpaymentrequests"
VIEW_PAYOUTS = "btcpay.store.canviewpayouts"
VIEW_PULL_PAYMENTS = "btcpay.store.canviewpullpayments"
VIEW_REPORTS = "btcpay.store.canviewreports"
UNRESTRICTED = "unrestricted"

# Direct inclusions from BTCPay Server 2.3 (`GET /misc/permissions`). The
# closure is computed below so only first-level edges need to be listed.
_DIRECT_INCLUDES: dict[str, tuple[str, ...]] = {
    UNRESTRICTED: (
        "btcpay.server.canmodifyserversettings",
        "btcpay.server.canviewusers",
        "btcpay.impersonation.canimpersonate",
        MODIFY_STORE_SETTINGS,
        "btcpay.user.candeleteuser",
        "btcpay.user.canmanagenotificationsforuser",
        "btcpay.user.canmodifyprofile",
    ),
    "btcpay.server.canmodifyserversettings": (
        "btcpay.server.canmanageusers",
        "btcpay.server.canuseinternallightningnode",
    ),
    "btcpay.server.canmanageusers": ("btcpay.server.cancreateuser",),
    "btcpay.server.canuseinternallightningnode": (
        "btcpay.server.cancreatelightninginvoiceinternalnode",
        "btcpay.server.canviewlightninginvoiceinternalnode",
    ),
    MODIFY_STORE_SETTINGS: (
        VIEW_STORE_SETTINGS,
        "btcpay.store.canmodifyinvoices",
        "btcpay.store.canmodifypaymentrequests",
        "btcpay.store.canmanagepullpayments",
        "btcpay.store.canmanagepayouts",
        "btcpay.store.canuselightningnode",
        "btcpay.store.canmodifyofferings",
        "btcpay.store.webhooks.canmodifywebhooks",
    ),
    VIEW_STORE_SETTINGS: (
        VIEW_INVOICES,
        VIEW_PAYMENT_REQUESTS,
        VIEW_PAYOUTS,
        VIEW_PULL_PAYMENTS,
        VIEW_REPORTS,
    ),
    "btcpay.store.canmodifyinvoices": (
        VIEW_INVOICES,
        "btcpay.store.cancreateinvoice",
        "btcpay.store.cancreatelightninginvoice",
    ),
    "btcpay.store.canmodifypaymentrequests": (VIEW_PAYMENT_REQUESTS,),
    "btcpay.store.canmanagepullpayments": (
        "btcpay.store.cancreatepullpayments",
        "btcpay.store.canarchivepullpayments",
    ),
    "btcpay.store.cancreatepullpayments": ("btcpay.store.cancreatenonapprovedpullpayments",),
    "btcpay.store.cancreatenonapprovedpullpayments": (VIEW_PULL_PAYMENTS,),
    "btcpay.store.canmanagepayouts": (VIEW_PAYOUTS,),
    "btcpay.store.canuselightningnode": ("btcpay.store.cancreatelightninginvoice",),
    "btcpay.store.cancreatelightninginvoice": ("btcpay.store.canviewlightninginvoice",),
    "btcpay.store.canmodifyofferings": (
        "btcpay.store.canviewofferings",
        "btcpay.store.canmanagesubscribers",
        "btcpay.store.cancreditsubscribers",
    ),
    "btcpay.user.canmodifyprofile": ("btcpay.user.canviewprofile",),
    "btcpay.user.canmanagenotificationsforuser": ("btcpay.user.canviewnotificationsforuser",),
}

# Permissions that only read data. Everything else can change the server,
# the store, or move money and is reported as broader than Kassiber needs.
READ_ONLY_PERMISSIONS = frozenset(
    {
        VIEW_STORE_SETTINGS,
        VIEW_INVOICES,
        VIEW_PAYMENT_REQUESTS,
        VIEW_PAYOUTS,
        VIEW_PULL_PAYMENTS,
        VIEW_REPORTS,
        "btcpay.store.canviewofferings",
        "btcpay.store.canviewlightninginvoice",
        "btcpay.user.canviewprofile",
        "btcpay.user.canviewnotificationsforuser",
        "btcpay.server.canviewusers",
        "btcpay.server.canviewlightninginvoiceinternalnode",
    }
)

PERMISSION_LABELS = {
    VIEW_STORE_SETTINGS: "View your stores",
    MODIFY_STORE_SETTINGS: "Modify your stores",
    VIEW_INVOICES: "View invoices",
    VIEW_PAYMENT_REQUESTS: "View payment requests",
    VIEW_PAYOUTS: "View payouts",
    VIEW_PULL_PAYMENTS: "View pull payments",
    VIEW_REPORTS: "View reports",
    UNRESTRICTED: "Unrestricted access",
}

# Kassiber capability -> the narrowest permission that grants it.
CAPABILITY_PERMISSIONS: dict[str, str] = {
    "store_catalog": VIEW_STORE_SETTINGS,
    "wallet_preview": VIEW_STORE_SETTINGS,
    "invoices": VIEW_INVOICES,
    "payment_requests": VIEW_PAYMENT_REQUESTS,
    "payouts": VIEW_PAYOUTS,
    "pull_payments": VIEW_PULL_PAYMENTS,
    "wallet_history": MODIFY_STORE_SETTINGS,
}
CAPABILITIES = tuple(CAPABILITY_PERMISSIONS)

KEY_PRESETS: dict[str, dict[str, Any]] = {
    "read_only": {
        "permissions": (VIEW_STORE_SETTINGS,),
        "label": "Read-only (recommended)",
        "summary": (
            "Invoices, payments, refunds and payouts, payment requests, and a "
            "wallet check that recognises which wallet each store pays into. "
            "The key cannot change your store."
        ),
        "capabilities": (
            "store_catalog",
            "wallet_preview",
            "invoices",
            "payment_requests",
            "payouts",
            "pull_payments",
        ),
    },
    "wallet_history": {
        "permissions": (MODIFY_STORE_SETTINGS,),
        "label": "Include BTCPay wallet history",
        "summary": (
            "Adds BTCPay's own on-chain wallet history with its labels and "
            "comments. BTCPay only offers this under the 'Modify your stores' "
            "permission, which could also change the store's payout wallet. "
            "Prefer the read-only key plus a watch-only wallet when you can."
        ),
        "capabilities": CAPABILITIES,
    },
}
DEFAULT_PRESET = "read_only"
# BTCPay's authorize page either grants store permissions on every store
# (`selectiveStores=false`) or lets the user pick exactly one store
# (`selectiveStores=true`). Read-only keys default to all stores so one key can
# serve a multi-store merchant; the store-modifying preset defaults to one.
STORE_SCOPES = ("all", "single")
DEFAULT_STORE_SCOPES = {"read_only": "all", "wallet_history": "single"}


def _closure() -> dict[str, frozenset[str]]:
    resolved: dict[str, frozenset[str]] = {}

    def visit(name: str, stack: tuple[str, ...] = ()) -> frozenset[str]:
        if name in resolved:
            return resolved[name]
        if name in stack:  # pragma: no cover - table is acyclic
            return frozenset({name})
        included = {name}
        for child in _DIRECT_INCLUDES.get(name, ()):
            included |= visit(child, stack + (name,))
        resolved[name] = frozenset(included)
        return resolved[name]

    for key in _DIRECT_INCLUDES:
        visit(key)
    return resolved


_INCLUDES = _closure()


def expand_permission(name: str) -> frozenset[str]:
    return _INCLUDES.get(name, frozenset({name}))


def permission_label(name: str) -> str:
    base = str(name or "").split(":", 1)[0]
    return PERMISSION_LABELS.get(base, base)


def split_permission(value: str) -> tuple[str, str | None]:
    raw = str(value or "").strip()
    if not raw:
        return "", None
    if raw.startswith("btcpay.") and ":" in raw:
        name, store = raw.split(":", 1)
        return name, store or None
    return raw, None


@dataclass(frozen=True)
class KeyGrant:
    """Effective permissions of one API key."""

    known: bool
    global_permissions: frozenset[str] = frozenset()
    store_permissions: Mapping[str, frozenset[str]] = field(default_factory=dict)
    raw: tuple[str, ...] = ()

    @property
    def scoped_store_ids(self) -> tuple[str, ...]:
        return tuple(sorted(self.store_permissions))

    @property
    def all_stores(self) -> bool:
        return any(name.startswith("btcpay.store.") or name == UNRESTRICTED for name in self.global_permissions)

    def effective(self, store_id: str | None) -> frozenset[str]:
        granted = set()
        for name in self.global_permissions:
            granted |= expand_permission(name)
        if store_id is not None:
            for name in self.store_permissions.get(store_id, frozenset()):
                granted |= expand_permission(name)
        return frozenset(granted)

    def has(self, permission: str, store_id: str | None) -> bool:
        return permission in self.effective(store_id)

    def capabilities(self, store_id: str | None) -> dict[str, bool]:
        effective = self.effective(store_id)
        return {
            capability: permission in effective
            for capability, permission in CAPABILITY_PERMISSIONS.items()
        }


def parse_key_grant(permissions: Iterable[str] | None) -> KeyGrant:
    if permissions is None:
        return KeyGrant(known=False)
    global_permissions: set[str] = set()
    store_permissions: dict[str, set[str]] = {}
    raw: list[str] = []
    for value in permissions:
        name, store_id = split_permission(str(value))
        if not name:
            continue
        raw.append(str(value))
        if store_id:
            store_permissions.setdefault(store_id, set()).add(name)
        else:
            global_permissions.add(name)
    return KeyGrant(
        known=True,
        global_permissions=frozenset(global_permissions),
        store_permissions={key: frozenset(value) for key, value in store_permissions.items()},
        raw=tuple(sorted(raw)),
    )


def key_risk(grant: KeyGrant) -> str:
    """Summarise how much damage a leaked key could do."""

    if not grant.known:
        return "unknown"
    names = set(grant.global_permissions)
    for values in grant.store_permissions.values():
        names |= set(values)
    if UNRESTRICTED in names or any(name.startswith("btcpay.server.") and name not in READ_ONLY_PERMISSIONS for name in names):
        return "server_admin"
    expanded: set[str] = set()
    for name in names:
        expanded |= expand_permission(name)
    if MODIFY_STORE_SETTINGS in expanded:
        return "can_modify_store"
    if any(name not in READ_ONLY_PERMISSIONS for name in expanded):
        return "can_write"
    return "read_only"


def excess_permissions(grant: KeyGrant, *, wallet_history: bool = False) -> list[str]:
    """Granted permissions that go beyond what Kassiber uses.

    ``wallet_history`` accepts ``canmodifystoresettings`` as required; the
    permissions it bundles are still reported by the caller's risk summary.
    """

    if not grant.known:
        return []
    allowed = set(READ_ONLY_PERMISSIONS)
    if wallet_history:
        allowed |= expand_permission(MODIFY_STORE_SETTINGS)
    names = set(grant.global_permissions)
    for values in grant.store_permissions.values():
        names |= set(values)
    return sorted(name for name in names if name not in allowed)


def grant_summary(grant: KeyGrant) -> dict[str, Any]:
    scope = "unknown"
    if grant.known:
        scope = "all_stores" if grant.all_stores else ("selected_stores" if grant.store_permissions else "no_stores")
    return {
        "known": grant.known,
        "scope": scope,
        "store_ids": list(grant.scoped_store_ids),
        "permissions": list(grant.raw),
        "risk": key_risk(grant),
        "excess_permissions": excess_permissions(grant, wallet_history=True),
        "global_capabilities": grant.capabilities(None) if grant.known else {},
    }


def missing_for_capability(capability: str) -> str:
    return CAPABILITY_PERMISSIONS[capability]


def preset_permissions(preset: str) -> tuple[str, ...]:
    try:
        return tuple(KEY_PRESETS[preset]["permissions"])
    except KeyError as exc:
        raise AppError(
            f"Unknown BTCPay key preset '{preset}'",
            code="validation",
            hint=f"Choose one of: {', '.join(KEY_PRESETS)}.",
        ) from exc


def default_store_scope(preset: str) -> str:
    return DEFAULT_STORE_SCOPES.get(preset, "single")


def build_authorize_url(
    server_url: str,
    *,
    preset: str = DEFAULT_PRESET,
    store_ids: Sequence[str] | None = None,
    store_scope: str | None = None,
    application_name: str = "Kassiber",
) -> str:
    """Return BTCPay's ``/api-keys/authorize`` URL for a pre-filled key.

    The user opens this page in their own browser, signs in, approves, and
    copies the key back into Kassiber. ``store_scope`` ``all`` grants the
    permission on every store the account can access; ``single`` makes BTCPay
    ask for one store. Explicit ``store_ids`` scope the permission to those
    stores. Building the URL performs no network I/O.
    """

    from .client import normalize_server_url

    base = normalize_server_url(server_url)
    permissions = preset_permissions(preset)
    scope = store_scope or default_store_scope(preset)
    if scope not in STORE_SCOPES:
        raise AppError(
            f"Unknown BTCPay key store scope '{scope}'",
            code="validation",
            hint=f"Choose one of: {', '.join(STORE_SCOPES)}.",
        )
    stores = [str(store).strip() for store in (store_ids or []) if str(store or "").strip()]
    if stores:
        scoped = [f"{permission}:{store}" for permission in permissions for store in stores]
    else:
        scoped = list(permissions)
    query: list[tuple[str, str]] = [("applicationName", application_name)]
    query.extend(("permissions", permission) for permission in scoped)
    query.append(("strict", "true"))
    query.append(("selectiveStores", "true" if scope == "single" and not stores else "false"))
    return f"{base}/api-keys/authorize?{urlparse.urlencode(query)}"


def key_setup_guide(
    server_url: str | None = None,
    *,
    preset: str = DEFAULT_PRESET,
    store_scope: str | None = None,
) -> dict[str, Any]:
    """Machine-readable setup instructions for a preset (no network I/O)."""

    info = KEY_PRESETS.get(preset)
    if info is None:
        preset_permissions(preset)  # raises the typed validation error
    permissions = list(info["permissions"])
    scope = store_scope or default_store_scope(preset)
    guide: dict[str, Any] = {
        "preset": preset,
        "label": info["label"],
        "summary": info["summary"],
        "permissions": permissions,
        "permission_labels": [permission_label(name) for name in permissions],
        "capabilities": list(info["capabilities"]),
        "store_scope": scope,
        "manual_steps": [
            "Sign in to BTCPay Server with the account that owns the store.",
            "Open Account → Manage Account → API Keys and choose Generate Key.",
            "Label the key 'Kassiber' and enable only: "
            + ", ".join(f"{permission_label(name)} ({name})" for name in permissions)
            + ".",
            (
                "Leave the permission on all stores so one key serves every store, then generate the key."
                if scope == "all"
                else "Limit the permission to the store Kassiber should read, then generate the key."
            ),
            "Copy the key into Kassiber. BTCPay shows it again on the API Keys page if you need it later.",
        ],
        "presets": [
            {"id": key, "label": value["label"], "permissions": list(value["permissions"])}
            for key, value in KEY_PRESETS.items()
        ],
    }
    if server_url:
        guide["authorize_url"] = build_authorize_url(server_url, preset=preset, store_scope=scope)
    return guide


__all__ = [
    "CAPABILITIES",
    "CAPABILITY_PERMISSIONS",
    "DEFAULT_PRESET",
    "KEY_PRESETS",
    "STORE_SCOPES",
    "KeyGrant",
    "MODIFY_STORE_SETTINGS",
    "READ_ONLY_PERMISSIONS",
    "VIEW_INVOICES",
    "VIEW_PAYOUTS",
    "VIEW_PAYMENT_REQUESTS",
    "VIEW_PULL_PAYMENTS",
    "VIEW_STORE_SETTINGS",
    "build_authorize_url",
    "default_store_scope",
    "excess_permissions",
    "expand_permission",
    "grant_summary",
    "key_risk",
    "key_setup_guide",
    "parse_key_grant",
    "permission_label",
    "preset_permissions",
    "split_permission",
]

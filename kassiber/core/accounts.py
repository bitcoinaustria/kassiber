from __future__ import annotations

import uuid

from ..backends import (
    BACKEND_KINDS,
    clear_default_backend as _clear_default_backend,
    create_db_backend as _create_db_backend,
    delete_db_backend as _delete_db_backend,
    get_db_backend,
    list_backends as _list_backends,
    set_default_backend as _set_default_backend,
    update_db_backend as _update_db_backend,
)
from ..db import get_setting, set_setting
from ..errors import AppError
from ..tax_policy import (
    build_tax_policy,
    require_tax_country_supported_for_profile_mutation,
    supported_tax_countries,
)
from ..time_utils import now_iso
from ..wallet_descriptors import normalize_asset_code
from .repo import invalidate_journals, resolve_profile, resolve_scope, resolve_workspace

ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
# Union of accounting methods across all supported countries. The per-policy
# allowed subset is enforced in `_normalized_profile_algorithm`, so this tuple
# is only the argparse-level superset (the CLI rejects typos before we build
# a policy, then the policy narrows it further based on tax_country).
RP2_ACCOUNTING_METHODS = (
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
)
_DEFAULT_ACCOUNTS = (
    ("treasury", "Treasury", "asset", "BTC"),
    ("fees", "Fees", "expense", "BTC"),
    ("external", "External", "equity", None),
)


def _normalized_profile_algorithm(raw_algorithm, policy):
    normalized = str(raw_algorithm or policy.default_accounting_method).strip().upper()
    allowed = {method.upper() for method in policy.accounting_methods}
    if normalized not in allowed:
        raise AppError(
            f"Unsupported gains algorithm '{raw_algorithm}' for tax_country='{policy.tax_country}'",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(method.upper() for method in policy.accounting_methods))}",
        )
    return normalized


def normalize_code(value):
    code = str(value).strip().lower().replace(" ", "-")
    if not code:
        raise AppError("Code cannot be empty")
    return code


def create_workspace(conn, label):
    workspace_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, label, now_iso()),
    )
    set_setting(conn, "context_workspace", workspace_id)
    # A new workspace does not have a compatible current profile yet.
    set_setting(conn, "context_profile", "")
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
    created_at = now_iso()
    for code, label, account_type, asset in _DEFAULT_ACCOUNTS:
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


def create_profile(
    conn,
    workspace_ref,
    label,
    fiat_currency,
    gains_algorithm,
    tax_country,
    tax_long_term_days,
):
    workspace = resolve_workspace(conn, workspace_ref)
    if tax_long_term_days < 0:
        raise AppError("Tax long-term days cannot be negative")
    require_tax_country_supported_for_profile_mutation(tax_country)
    if gains_algorithm.upper() not in RP2_ACCOUNTING_METHODS:
        raise AppError(
            f"Unsupported gains algorithm '{gains_algorithm}'",
            code="validation",
            hint=f"Choose one of: {', '.join(RP2_ACCOUNTING_METHODS)}",
        )
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
    normalized_algo = _normalized_profile_algorithm(gains_algorithm, policy)
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
            normalized_algo,
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
    if new_country is not None:
        require_tax_country_supported_for_profile_mutation(new_country)
    try:
        policy = build_tax_policy(
            {
                "fiat_currency": merged_fiat,
                "tax_country": merged_country,
                "tax_long_term_days": merged_long_term,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc), code="validation") from exc
    normalized_algo = _normalized_profile_algorithm(merged_algo, policy)
    policy_changed = (
        policy.fiat_currency != profile["fiat_currency"]
        or policy.tax_country != profile["tax_country"]
        or policy.long_term_days != profile["tax_long_term_days"]
        or normalized_algo != profile["gains_algorithm"]
    )

    conn.execute(
        """
        UPDATE profiles
        SET label = ?, fiat_currency = ?, tax_country = ?, tax_long_term_days = ?, gains_algorithm = ?
        WHERE id = ?
        """,
        (
            merged_label,
            policy.fiat_currency,
            policy.tax_country,
            policy.long_term_days,
            normalized_algo,
            profile["id"],
        ),
    )
    if policy_changed:
        invalidate_journals(conn, profile["id"])
    conn.commit()
    return get_profile_details(conn, workspace["id"], profile["id"])


def create_account(conn, workspace_ref, profile_ref, code, label, account_type, asset=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    code = normalize_code(code)
    account_type = account_type.lower()
    if account_type not in ACCOUNT_TYPES:
        raise AppError(
            f"Unsupported account type '{account_type}'. Supported: {', '.join(sorted(ACCOUNT_TYPES))}"
        )
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


def list_backends(runtime_config):
    return _list_backends(runtime_config)


def list_backend_kinds():
    return [{"kind": kind} for kind in sorted(BACKEND_KINDS)]


def get_backend_details(conn, runtime_config, name):
    try:
        payload = get_db_backend(conn, name)
        payload["is_default"] = payload["name"] == runtime_config["default_backend"]
        return payload
    except AppError as exc:
        if exc.code != "not_found":
            raise
        normalized_name = str(name).strip().lower()
        backend = runtime_config["backends"].get(normalized_name)
        if not backend:
            raise
        return {
            "name": normalized_name,
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
            "is_default": normalized_name == runtime_config["default_backend"],
        }


def create_backend(
    conn,
    name,
    kind,
    url,
    chain=None,
    network=None,
    auth_header=None,
    token=None,
    batch_size=None,
    timeout=None,
    tor_proxy=None,
    config=None,
    notes=None,
):
    return _create_db_backend(
        conn,
        name,
        kind,
        url,
        chain=chain,
        network=network,
        auth_header=auth_header,
        token=token,
        batch_size=batch_size,
        timeout=timeout,
        tor_proxy=tor_proxy,
        config=config,
        notes=notes,
    )


def update_backend(conn, name, updates):
    return _update_db_backend(conn, name, updates)


def delete_backend(conn, name):
    return _delete_db_backend(conn, name)


def set_default_backend(conn, runtime_config, name):
    return _set_default_backend(conn, runtime_config, name)


def clear_default_backend(conn, runtime_config):
    return _clear_default_backend(conn, runtime_config)


__all__ = [
    "ACCOUNT_TYPES",
    "BACKEND_KINDS",
    "RP2_ACCOUNTING_METHODS",
    "clear_default_backend",
    "create_account",
    "create_backend",
    "create_profile",
    "create_workspace",
    "delete_backend",
    "ensure_default_accounts",
    "get_backend_details",
    "get_profile_details",
    "list_accounts",
    "list_backend_kinds",
    "list_backends",
    "list_profiles",
    "list_workspaces",
    "normalize_code",
    "set_default_backend",
    "supported_tax_countries",
    "update_backend",
    "update_profile",
]

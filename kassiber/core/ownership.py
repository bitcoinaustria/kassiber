from __future__ import annotations

"""Address / transaction-id ownership reconciliation.

Given a list of addresses and/or transaction ids (Bitcoin or Liquid, mixed),
decide whether each one belongs to any wallet in the active profile, naming the
owning wallet, branch (receive/change) and derivation index, and flagging the
externals. The use case is reconciling historic flows: telling apart payments
from transfers between your own wallets long after the fact.

This module is the single matching engine shared by the CLI (`wallets
identify`), the daemon read kind (`ui.wallets.identify`) and the AI read tool.
It performs no network I/O of its own: the on-chain verification fetcher is
injected by the caller so the read surfaces can stay cache-only while the CLI
opts in explicitly. Matching is done on canonical scriptPubKey hex wherever
possible (encoding-independent, and the same for Liquid confidential vs
unconfidential addresses), falling back to address-string comparison for inputs
the script helper cannot canonicalize (e.g. Liquid confidential addresses).

Three resolution tiers, cheapest first:

1. Free local — the durable watch-only ``wallet_utxos`` inventory plus
   ``transactions.external_id`` resolve anything already synced/imported with
   zero derivation and zero network.
2. Deep derive (offline) — descriptor wallets derive receive+change up to a
   bounded ceiling and match candidate scripts/addresses.
3. On-chain verify (opt-in) — a caller-supplied fetcher pulls an unseen
   transaction so its inputs/outputs can be classified per-leg.
"""

import csv
import io
import json
import re
import sqlite3
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable, Mapping, Sequence

from ..errors import AppError
from ..wallet_descriptors import derive_descriptor_targets
from .sync_backends import address_to_scriptpubkey
from .wallets import load_wallet_descriptor_plan_from_config, normalize_addresses

# Default per-branch derivation ceiling for an interactive reconciliation run.
# Owned candidates at low indices resolve instantly via the cheap tiers; only
# fully-external candidates force a derive up to this depth, so it is kept
# modest and the CLI exposes --scan-to-index for deep historic reconciliation.
DEFAULT_SCAN_TO_INDEX = 500
# Hard ceiling so an absurd --scan-to-index cannot wedge the daemon main thread.
MAX_SCAN_TO_INDEX = 20_000

# A txid candidate is 64 lowercase hex chars; anything else is treated as an
# address candidate and validated downstream.
_TXID_LENGTH = 64

# HRPs whose addresses are case-insensitive (bech32 / bech32m / blech32). Used
# only to normalize the lookup key; base58 stays case-sensitive.
_CASE_INSENSITIVE_PREFIXES = (
    "bc1",
    "tb1",
    "bcrt1",
    "lq1",
    "tlq1",
    "ex1",
    "tex1",
    "el1",
    "ert1",
)

# A plausible address is purely alphanumeric (bech32/bech32m/blech32 and base58
# are all alnum) within sane length bounds. Anything else (spaces, punctuation,
# a pasted label or URI) is flagged invalid rather than silently "external".
_ADDRESS_RE = re.compile(r"[0-9A-Za-z]{8,150}")

# Address HRPs that make a token unambiguously a bech32/bech32m/blech32 address.
_BECH32_ADDRESS_PREFIXES = (
    "bc1",
    "tb1",
    "bcrt1",
    "lq1",
    "tlq1",
    "ex1",
    "tex1",
    "el1",
    "ert1",
)

# Conservative header aliases for smart CSV import. Ambiguous names (bare "tx",
# "hash", "transaction") are intentionally omitted — content harvesting (strict
# 64-hex / real-address checks) catches those columns without false positives.
_ADDRESS_HEADER_ALIASES = frozenset(
    {
        "address",
        "addr",
        "addresses",
        "bitcoin address",
        "btc address",
        "wallet address",
        "receive address",
        "receiving address",
        "to address",
        "from address",
        "output address",
        "destination address",
        "liquid address",
    }
)
_TXID_HEADER_ALIASES = frozenset(
    {
        "txid",
        "tx id",
        "tx_id",
        "txhash",
        "tx hash",
        "transaction id",
        "transaction hash",
        "txn id",
        "txn hash",
    }
)

# Upper bound on tokens harvested from one CSV. The harvest loop stops once this
# many are collected (bounding the per-cell validation work on a huge local
# file), and the caller surfaces a truncation warning.
MAX_HARVEST_CANDIDATES = 10_000


@dataclass(frozen=True)
class OwnedMatch:
    """One way an owned address/script was reached."""

    wallet_id: str
    wallet_label: str
    account: str
    chain: str
    network: str
    branch_label: str
    address_index: int | None
    derivation_path: str | None
    source: str  # "inventory" | "derived" | "address_list"


@dataclass
class OwnedIndex:
    """In-memory ownership index built from a profile's wallets."""

    by_script: dict[str, list[OwnedMatch]] = field(default_factory=dict)
    by_address: dict[str, list[OwnedMatch]] = field(default_factory=dict)
    by_outpoint: dict[str, OwnedMatch] = field(default_factory=dict)
    txid_wallets: dict[str, set[tuple[str, str]]] = field(default_factory=dict)
    scanned_depth: dict[str, dict[str, int]] = field(default_factory=dict)

    def add_script(self, script_hex: str | None, match: OwnedMatch) -> None:
        if not script_hex:
            return
        self.by_script.setdefault(script_hex.lower(), []).append(match)

    def add_address(self, address: str | None, match: OwnedMatch) -> None:
        key = _address_key(address)
        if not key:
            return
        self.by_address.setdefault(key, []).append(match)

    def add_outpoint(self, txid: str | None, vout: Any, match: OwnedMatch) -> None:
        if not txid or vout is None:
            return
        self.by_outpoint.setdefault(f"{str(txid).lower()}:{int(vout)}", match)

    def note_txid(self, txid: str | None, wallet_id: str, wallet_label: str) -> None:
        if not txid:
            return
        self.txid_wallets.setdefault(str(txid).lower(), set()).add((wallet_id, wallet_label))

    def lookup_address(self, address: str) -> list[OwnedMatch]:
        return self.by_address.get(_address_key(address), [])

    def lookup_script(self, script_hex: str | None) -> list[OwnedMatch]:
        if not script_hex:
            return []
        return self.by_script.get(script_hex.lower(), [])


def _address_key(address: str | None) -> str:
    text = str(address or "").strip()
    if not text:
        return ""
    lowered = text.lower()
    if lowered.startswith(_CASE_INSENSITIVE_PREFIXES):
        return lowered
    return text


def _script_hex_for_address(address: str) -> str | None:
    """Best-effort canonical scriptPubKey hex; ``None`` when unsupported.

    ``address_to_scriptpubkey`` covers Bitcoin bech32/bech32m + base58 only, so
    Liquid confidential addresses (and anything malformed) return ``None`` and
    fall back to address-string matching.
    """
    try:
        return address_to_scriptpubkey(address).hex()
    except AppError:
        return None
    except Exception:  # defensive: never let a parse quirk abort a batch
        return None


def classify_token_type(token: str) -> str:
    """Return "txid" for a 64-char hex string, else "address"."""
    text = str(token or "").strip()
    if len(text) == _TXID_LENGTH:
        try:
            bytes.fromhex(text)
            return "txid"
        except ValueError:
            return "address"
    return "address"


def _detect_chain(address: str) -> str:
    lowered = str(address or "").strip().lower()
    if lowered.startswith(("lq1", "tlq1", "ex1", "tex1", "el1", "ert1")):
        return "liquid"
    if lowered.startswith(("bc1", "tb1", "bcrt1")):
        return "bitcoin"
    # Liquid base58 (confidential CT.. / VJL.. /VT.. /Az..) vs Bitcoin base58 is
    # ambiguous without parsing; leave unknown rather than guess wrong.
    return ""


def parse_tokens(
    addresses: Iterable[str] | None = None,
    txids: Iterable[str] | None = None,
    candidates: Iterable[str] | None = None,
    file_text: str | None = None,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Normalize mixed inputs into (parsed candidates, invalid rows).

    ``--address`` / ``--txid`` force the type; ``--candidate`` and file lines are
    auto-classified. Whitespace, blank lines and ``#`` comments are ignored.
    Duplicates (same normalized token + type) collapse to one entry.
    """
    parsed: list[dict[str, Any]] = []
    invalid: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()

    def _add(raw: str, forced_type: str | None) -> None:
        text = str(raw or "").strip()
        if not text or text.startswith("#"):
            return
        token_type = forced_type or classify_token_type(text)
        if token_type == "txid":
            normalized = text.lower()
            if len(normalized) != _TXID_LENGTH or not _is_hex(normalized):
                invalid.append({"input": text, "type": "txid", "reason": "not a 64-char hex transaction id"})
                return
            dedup_key = normalized
        else:
            if not _ADDRESS_RE.fullmatch(text):
                invalid.append({"input": text, "type": "address", "reason": "not a valid-looking address"})
                return
            normalized = text
            # bech32/blech32 HRPs are case-insensitive, so collapse case variants
            # of the same address to one entry (base58 stays case-sensitive).
            dedup_key = _address_key(text)
        key = (token_type, dedup_key)
        if key in seen:
            return
        seen.add(key)
        entry: dict[str, Any] = {"input": text, "normalized": normalized, "type": token_type}
        if token_type == "address":
            entry["chain"] = _detect_chain(text)
        parsed.append(entry)

    for value in addresses or []:
        _add(value, "address")
    for value in txids or []:
        _add(value, "txid")
    for value in candidates or []:
        _add(value, None)
    if file_text:
        for line in file_text.splitlines():
            _add(line, None)
    return parsed, invalid


def _is_hex(value: str) -> bool:
    try:
        bytes.fromhex(value)
        return True
    except ValueError:
        return False


def _is_txid_token(value: str) -> bool:
    return len(value) == _TXID_LENGTH and _is_hex(value.lower())


def _looks_like_address(value: str) -> bool:
    """Strict "is this really an address" check for CSV content harvesting.

    Bech32/blech32 are accepted by HRP prefix; base58 is accepted only when it
    checksum-validates via ``address_to_scriptpubkey``. This is deliberately
    tighter than ``_ADDRESS_RE`` so a plain word/amount in a spreadsheet cell is
    not harvested as an address.
    """
    text = str(value or "").strip()
    if not text or not _ADDRESS_RE.fullmatch(text):
        return False
    if text.lower().startswith(_BECH32_ADDRESS_PREFIXES):
        return True
    return _script_hex_for_address(text) is not None


def _normalize_header(value: Any) -> str:
    return " ".join(str(value or "").replace("\xa0", " ").split()).casefold()


def _read_csv_rows(text: str) -> list[list[str]]:
    """Parse CSV text into rows, sniffing the delimiter (comma/semicolon/tab/pipe)
    and stripping a UTF-8 BOM."""
    if not text:
        return []
    stripped = text.lstrip("\ufeff")
    if not stripped.strip():
        return []
    try:
        dialect: Any = csv.Sniffer().sniff(stripped[:8192], delimiters=",;\t|")
    except csv.Error:
        dialect = None
    reader = (
        csv.reader(io.StringIO(stripped), dialect)
        if dialect is not None
        else csv.reader(io.StringIO(stripped))
    )
    rows: list[list[str]] = []
    try:
        for row in reader:
            rows.append([str(cell) for cell in row])
    except csv.Error:
        # csv raises mid-iteration on e.g. an over-limit field; keep the rows
        # parsed so far and recover the rest by splitting on delimiters.
        # (An unbalanced quote does NOT raise — it swallows content into one
        # runaway field; extract_candidates_from_csv's raw-split safety net
        # covers that case.)
        rows.extend(re.split(r"[\s,;|\t]+", line) for line in stripped.splitlines())
    return rows


def _recognize_csv_columns(header: Sequence[str]) -> tuple[list[int], list[int]]:
    address_cols: list[int] = []
    txid_cols: list[int] = []
    for index, cell in enumerate(header):
        name = _normalize_header(cell)
        if name in _ADDRESS_HEADER_ALIASES:
            address_cols.append(index)
        elif name in _TXID_HEADER_ALIASES:
            txid_cols.append(index)
    return address_cols, txid_cols


def extract_candidates_from_csv(text: str | None) -> list[str]:
    """Harvest address/txid tokens from arbitrary CSV text.

    Every candidate is strictly validated as a 64-hex txid or a real address
    (bech32 HRP, or base58 that checksum-validates), so amounts, dates, memos,
    and labels are ignored even when they sit under a recognized
    ``address``/``txid`` header. Recognized columns are scanned first (so a named
    column is preferred in output order); then every cell is content-scanned, so
    tokens in unrecognized columns or header-less files are still found; finally
    a raw delimiter split of the source is unioned in as a safety net so a token
    swallowed into a malformed (e.g. unbalanced-quote) cell is still recovered.
    De-duplicated in first-seen order, bounded by ``MAX_HARVEST_CANDIDATES``.
    """
    raw = text or ""
    rows = _read_csv_rows(raw)
    seen: set[str] = set()
    out: list[str] = []

    def _add_strict(value: Any) -> bool:
        """Add a strictly-valid token; return True once the harvest cap is hit."""
        token = str(value or "").strip()
        if token and token not in seen and (_is_txid_token(token) or _looks_like_address(token)):
            seen.add(token)
            out.append(token)
        return len(out) > MAX_HARVEST_CANDIDATES

    if rows:
        address_cols, txid_cols = _recognize_csv_columns(rows[0])
        if address_cols or txid_cols:
            for row in rows[1:]:
                for column in (*address_cols, *txid_cols):
                    if column < len(row) and _add_strict(row[column]):
                        return out
        for row in rows:
            for cell in row:
                if _add_strict(cell):
                    return out
    # Safety net: a malformed/unbalanced-quote cell can swallow later content into
    # one runaway field, so also scan the raw text split on delimiters.
    for token in re.split(r"[\s,;|\t]+", raw):
        if _add_strict(token):
            return out
    return out


def read_text_file(path: str, *, label: str = "file") -> str:
    """Read a UTF-8 text file, rejecting binary content (NUL bytes)."""
    from pathlib import Path

    # Capitalize only the first character so acronym labels ("CSV file") survive.
    titled = label[:1].upper() + label[1:]
    try:
        text = Path(path).read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise AppError(f"{titled} not found: {path}", code="validation") from exc
    except (OSError, UnicodeDecodeError) as exc:
        raise AppError(f"Could not read {label}: {path}", code="validation") from exc
    if "\x00" in text:
        raise AppError(f"{titled} is not valid UTF-8 text", code="validation")
    return text


def load_profile_wallets(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_ids: Sequence[str] | None = None,
) -> list[sqlite3.Row]:
    """All wallets in a profile (optionally restricted), with account labels."""
    sql = (
        "SELECT w.id, w.label, w.kind, w.config_json, "
        "a.code AS account_code, a.label AS account_label "
        "FROM wallets w LEFT JOIN accounts a ON a.id = w.account_id "
        "WHERE w.profile_id = ?"
    )
    params: list[Any] = [profile_id]
    if wallet_ids:
        placeholders = ", ".join("?" for _ in wallet_ids)
        sql += f" AND w.id IN ({placeholders})"
        params.extend(wallet_ids)
    sql += " ORDER BY w.label ASC"
    return list(conn.execute(sql, params).fetchall())


def _account_label(row: Mapping[str, Any]) -> str:
    keys = row.keys() if hasattr(row, "keys") else []
    code = row["account_code"] if "account_code" in keys else None
    label = row["account_label"] if "account_label" in keys else None
    return str(code or label or "")


def build_owned_index(
    conn: sqlite3.Connection,
    profile_id: str,
    wallets: Sequence[sqlite3.Row],
    *,
    scan_to_index: int = DEFAULT_SCAN_TO_INDEX,
    derive: bool = True,
) -> tuple[OwnedIndex, list[str]]:
    """Build the ownership index for the given wallets.

    Seeds the cheap tiers (output inventory + imported txids + address-list
    wallets) unconditionally, then - when ``derive`` is set - derives descriptor
    wallets up to a per-wallet inclusive ceiling (``max(scan_to_index,
    highest_used + gap_limit)``) so historic addresses past the last synced
    index are still found. Returns the index plus any non-fatal warnings (e.g.
    a descriptor that could not be parsed).
    """
    index = OwnedIndex()
    warnings: list[str] = []
    wallet_ids = [str(w["id"]) for w in wallets]
    wallet_id_set = set(wallet_ids)

    highest_used = _seed_from_inventory(conn, index, profile_id, wallet_id_set)
    _seed_from_transactions(conn, index, profile_id, wallets)

    for wallet in wallets:
        config = _wallet_config(wallet)
        account = _account_label(wallet)
        addresses = normalize_addresses(config.get("addresses"))
        if addresses:
            chain = str(config.get("chain") or "")
            network = str(config.get("network") or "")
            for address in addresses:
                match = OwnedMatch(
                    wallet_id=str(wallet["id"]),
                    wallet_label=str(wallet["label"]),
                    account=account,
                    chain=chain,
                    network=network,
                    branch_label="address",
                    address_index=None,
                    derivation_path=None,
                    source="address_list",
                )
                index.add_address(address, match)
                index.add_script(_script_hex_for_address(address), match)
        if not derive or not config.get("descriptor"):
            continue
        try:
            _derive_wallet_into_index(
                index,
                wallet,
                config,
                account,
                scan_to_index=scan_to_index,
                highest_used=highest_used.get(str(wallet["id"]), {}),
            )
        except AppError as exc:
            warnings.append(f"Wallet '{wallet['label']}': descriptor not scanned ({exc.code})")
    return index, warnings


def _wallet_config(wallet: Mapping[str, Any]) -> dict[str, Any]:
    try:
        return json.loads(wallet["config_json"] or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}


def _seed_from_inventory(
    conn: sqlite3.Connection,
    index: OwnedIndex,
    profile_id: str,
    wallet_id_set: set[str],
) -> dict[str, dict[str, int]]:
    """Seed from the durable watch-only output inventory; return highest used
    address index per wallet/branch for ceiling computation."""
    highest: dict[str, dict[str, int]] = {}
    rows = conn.execute(
        "SELECT wallet_id, txid, vout, address, branch_label, branch_index, "
        "address_index, chain, network FROM wallet_utxos WHERE profile_id = ?",
        (profile_id,),
    ).fetchall()
    label_by_id = _wallet_label_lookup(conn, profile_id)
    for row in rows:
        wallet_id = str(row["wallet_id"])
        if wallet_id_set and wallet_id not in wallet_id_set:
            continue
        wallet_label = label_by_id.get(wallet_id, wallet_id)
        match = OwnedMatch(
            wallet_id=wallet_id,
            wallet_label=wallet_label,
            account="",
            chain=str(row["chain"] or ""),
            network=str(row["network"] or ""),
            branch_label=str(row["branch_label"] or ""),
            address_index=row["address_index"],
            derivation_path=None,
            source="inventory",
        )
        index.add_address(row["address"], match)
        index.add_script(_script_hex_for_address(row["address"]) if row["address"] else None, match)
        index.add_outpoint(row["txid"], row["vout"], match)
        index.note_txid(row["txid"], wallet_id, wallet_label)
        branch = str(row["branch_label"] or "")
        idx = row["address_index"]
        if branch and isinstance(idx, int) and idx >= 0:
            branch_map = highest.setdefault(wallet_id, {})
            branch_map[branch] = max(branch_map.get(branch, -1), idx)
    return highest


def _seed_from_transactions(
    conn: sqlite3.Connection,
    index: OwnedIndex,
    profile_id: str,
    wallets: Sequence[sqlite3.Row],
) -> None:
    label_by_id = {str(w["id"]): str(w["label"]) for w in wallets}
    if not label_by_id:
        return
    placeholders = ", ".join("?" for _ in label_by_id)
    rows = conn.execute(
        f"SELECT external_id, wallet_id FROM transactions "
        f"WHERE profile_id = ? AND external_id IS NOT NULL AND wallet_id IN ({placeholders})",
        (profile_id, *label_by_id.keys()),
    ).fetchall()
    for row in rows:
        wallet_id = str(row["wallet_id"])
        index.note_txid(row["external_id"], wallet_id, label_by_id.get(wallet_id, wallet_id))


def _wallet_label_lookup(conn: sqlite3.Connection, profile_id: str) -> dict[str, str]:
    return {
        str(row["id"]): str(row["label"])
        for row in conn.execute(
            "SELECT id, label FROM wallets WHERE profile_id = ?", (profile_id,)
        ).fetchall()
    }


def _derive_wallet_into_index(
    index: OwnedIndex,
    wallet: Mapping[str, Any],
    config: Mapping[str, Any],
    account: str,
    *,
    scan_to_index: int,
    highest_used: Mapping[str, int],
) -> None:
    plan = load_wallet_descriptor_plan_from_config(config)
    if plan is None:
        return
    gap_limit = int(getattr(plan, "gap_limit", 0) or 0)
    used_floor = max(highest_used.values(), default=-1)
    ceiling = max(int(scan_to_index) + 1, used_floor + gap_limit + 1)
    ceiling = max(1, min(ceiling, MAX_SCAN_TO_INDEX + 1))
    wallet_id = str(wallet["id"])
    depth: dict[str, int] = {}
    for target in derive_descriptor_targets(plan, branch_index=None, start=0, end=ceiling):
        match = OwnedMatch(
            wallet_id=wallet_id,
            wallet_label=str(wallet["label"]),
            account=account,
            chain=plan.chain,
            network=plan.network,
            branch_label=target.branch_label,
            address_index=target.address_index,
            derivation_path=target.derivation_path,
            source="derived",
        )
        index.add_script(target.script_pubkey, match)
        index.add_address(target.address, match)
        if target.unconfidential_address:
            index.add_address(target.unconfidential_address, match)
        depth[target.branch_label] = max(depth.get(target.branch_label, -1), target.address_index)
    index.scanned_depth[wallet_id] = depth


def _match_to_dict(match: OwnedMatch) -> dict[str, Any]:
    return {
        "wallet": match.wallet_label,
        "wallet_id": match.wallet_id,
        "account": match.account,
        "chain": match.chain,
        "network": match.network,
        "branch": match.branch_label,
        "address_index": match.address_index,
        "derivation_path": match.derivation_path,
        "match_source": match.source,
    }


def classify_address(token: Mapping[str, Any], index: OwnedIndex) -> dict[str, Any]:
    address = str(token["input"])
    matches = index.lookup_address(address)
    if not matches:
        script_hex = _script_hex_for_address(address)
        matches = index.lookup_script(script_hex)
    chain = token.get("chain") or (matches[0].chain if matches else "")
    if matches:
        primary = matches[0]
        return {
            "input": address,
            "type": "address",
            "chain": chain or primary.chain,
            "status": "owned",
            "classification": "owned_address",
            "matches": [_match_to_dict(m) for m in matches],
            "note": _ownership_note(matches),
        }
    return {
        "input": address,
        "type": "address",
        "chain": chain,
        "status": "external",
        "classification": "external_address",
        "matches": [],
        "note": "Not derived from or seen by any wallet in this profile.",
    }


def _ownership_note(matches: Sequence[OwnedMatch]) -> str:
    primary = matches[0]
    where = primary.branch_label or "address"
    if primary.address_index is not None:
        where = f"{where} #{primary.address_index}"
    note = f"Owned by '{primary.wallet_label}' ({where})."
    if len(matches) > 1:
        others = sorted({m.wallet_label for m in matches[1:]})
        if others:
            note += " Also matches: " + ", ".join(others) + "."
    return note


def classify_txid(
    token: Mapping[str, Any],
    index: OwnedIndex,
    legs: Mapping[str, Any] | None,
) -> dict[str, Any]:
    txid = str(token["normalized"])
    local_wallets = index.txid_wallets.get(txid, set())
    if legs is None:
        if local_wallets:
            wallets = sorted({label for _wid, label in local_wallets})
            return {
                "input": txid,
                "type": "txid",
                "chain": "",
                "status": "owned",
                "classification": "touches_wallet",
                "wallets": wallets,
                "owned_inputs": None,
                "owned_outputs": None,
                "external_outputs": None,
                "legs": [],
                "match_source": "inventory",
                "note": (
                    "Recorded against "
                    + ", ".join(f"'{w}'" for w in wallets)
                    + "; per-leg breakdown needs on-chain verification."
                ),
            }
        return {
            "input": txid,
            "type": "txid",
            "chain": "",
            "status": "unknown",
            "classification": "unknown",
            "wallets": [],
            "owned_inputs": None,
            "owned_outputs": None,
            "external_outputs": None,
            "legs": [],
            "match_source": "none",
            "note": (
                "Not in this profile's synced/imported history; "
                "on-chain verification is needed for a verdict."
            ),
        }

    chain = str(legs.get("chain") or token.get("chain") or "")
    source = str(legs.get("source") or "chain")
    in_legs: list[dict[str, Any]] = []
    owned_in = 0
    unresolved_inputs = 0
    involved: set[str] = set()
    for leg in legs.get("inputs", []):
        match = index.by_outpoint.get(str(leg.get("outpoint") or ""))
        if match is None:
            script = leg.get("script")
            script_matches = index.lookup_script(script)
            match = script_matches[0] if script_matches else None
            if match is None and script is None and source != "local_tx":
                unresolved_inputs += 1
        owned = match is not None
        owned_in += 1 if owned else 0
        if match is not None:
            involved.add(match.wallet_label)
        in_legs.append({"side": "input", "outpoint": leg.get("outpoint"), "owned": owned, "wallet": match.wallet_label if match else ""})

    out_legs: list[dict[str, Any]] = []
    owned_out = 0
    for leg in legs.get("outputs", []):
        script = leg.get("script")
        if _is_unspendable_output_script(script):
            # Empty scriptPubKey is the Liquid fee output; OP_RETURN/data
            # outputs are provably unspendable. Neither is a recipient.
            continue
        script_matches = index.lookup_script(script)
        match = script_matches[0] if script_matches else None
        owned = match is not None
        owned_out += 1 if owned else 0
        if match is not None:
            involved.add(match.wallet_label)
        out_legs.append(
            {
                "side": "output",
                "n": leg.get("n"),
                "owned": owned,
                "wallet": match.wallet_label if match else "",
                "branch": match.branch_label if match else "",
            }
        )
    for _wid, label in local_wallets:
        involved.add(label)

    output_count = len(out_legs)
    external_out = output_count - owned_out
    classification, status, note = _classify_legs(
        owned_in,
        owned_out,
        external_out,
        unresolved_inputs=unresolved_inputs,
        touches_wallet=bool(local_wallets),
    )
    return {
        "input": txid,
        "type": "txid",
        "chain": chain,
        "status": status,
        "classification": classification,
        "wallets": sorted(involved),
        "owned_inputs": owned_in,
        "owned_outputs": owned_out,
        "external_outputs": external_out,
        "legs": in_legs + out_legs,
        "match_source": source,
        "note": note,
    }


def _is_unspendable_output_script(script: Any) -> bool:
    if not script:
        return True
    return str(script).strip().lower().startswith("6a")


def _classify_legs(
    owned_in: int,
    owned_out: int,
    external_out: int,
    *,
    unresolved_inputs: int = 0,
    touches_wallet: bool = False,
) -> tuple[str, str, str]:
    if owned_in == 0 and owned_out == 0:
        if touches_wallet:
            return (
                "touches_wallet",
                "owned",
                "Recorded locally against this profile, but verified legs did not match "
                "known wallet scripts.",
            )
        return ("external", "external", "No inputs or outputs belong to this profile.")
    if owned_out == 0 and external_out == 0:
        # Owned inputs but no spendable outputs resolved (partial/malformed legs);
        # do not assert a self-transfer about outputs that aren't there.
        return (
            "undetermined",
            "owned",
            "Inputs are owned but no outputs were resolved; classification unavailable.",
        )
    if owned_in == 0 and owned_out > 0:
        if unresolved_inputs > 0:
            return (
                "undetermined",
                "owned",
                "Outputs include an owned address, but one or more inputs could not be resolved.",
            )
        return (
            "inbound_receipt",
            "owned",
            "Inbound: funds received to an owned address from an external sender.",
        )
    if owned_in > 0 and external_out == 0:
        return (
            "self_transfer",
            "owned",
            "Self-transfer/consolidation: all outputs return to owned addresses.",
        )
    if owned_in > 0 and owned_out > 0:
        return (
            "outbound_payment",
            "owned",
            "Outbound payment: spent from owned inputs to an external recipient (owned outputs are change).",
        )
    return (
        "outbound_payment",
        "owned",
        "Outbound payment: spent from owned inputs entirely to external recipients.",
    )


def load_local_tx_legs(
    conn: sqlite3.Connection,
    profile_id: str,
    txid: str,
) -> dict[str, Any] | None:
    """Recover input/output scripts from a locally stored transaction.

    Only Bitcoin on-chain sync persists full ``vin``/``vout`` JSON in
    ``raw_json``, in two shapes: esplora (``vout[].scriptpubkey``,
    ``vin[].prevout.scriptpubkey``) and Electrum's decoded form
    (``vout[].script_hex``, no inline prevout scripts). Both are handled.
    Liquid (component context only) and all import paths carry no vin/vout, so
    they return ``None`` and the caller falls back to on-chain verification.
    """
    rows = conn.execute(
        "SELECT raw_json FROM transactions WHERE profile_id = ? AND external_id = ?",
        (profile_id, txid),
    ).fetchall()
    for row in rows:
        legs = _legs_from_local_tx_json(row["raw_json"])
        if legs is not None:
            return legs
    return None


def _legs_from_local_tx_json(raw_json: Any) -> dict[str, Any] | None:
    try:
        raw = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    vin = raw.get("vin")
    vout = raw.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None
    inputs = []
    for entry in vin:
        if not isinstance(entry, dict):
            continue
        prevout = entry.get("prevout") or {}
        outpoint = None
        if entry.get("txid") is not None and entry.get("vout") is not None:
            outpoint = f"{str(entry.get('txid')).lower()}:{int(entry.get('vout'))}"
        # esplora carries the prevout script inline; the Electrum decode form
        # does not (input ownership then resolves via the outpoint).
        inputs.append({"outpoint": outpoint, "script": prevout.get("scriptpubkey")})
    outputs = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, dict):
            continue
        # esplora -> scriptpubkey; Electrum decode form -> script_hex.
        script = entry.get("scriptpubkey")
        if script is None:
            script = entry.get("script_hex")
        outputs.append({"n": entry.get("n", position), "script": script})
    return {"inputs": inputs, "outputs": outputs, "chain": "bitcoin", "source": "local_tx"}


VerifyFetcher = Callable[[str, str], "dict[str, Any] | None"]


def identify(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    addresses: Iterable[str] | None = None,
    txids: Iterable[str] | None = None,
    candidates: Iterable[str] | None = None,
    file_text: str | None = None,
    csv_text: str | None = None,
    wallet_ids: Sequence[str] | None = None,
    scan_to_index: int = DEFAULT_SCAN_TO_INDEX,
    verify_fetcher: VerifyFetcher | None = None,
) -> dict[str, Any]:
    """Run the full ownership reconciliation and return a structured report.

    ``verify_fetcher(txid, chain_hint) -> legs|None`` is injected by the caller
    for the opt-in on-chain tier; when ``None`` (the cache-only default) txids
    not already in local history resolve to ``unknown``. ``csv_text`` is parsed
    by the smart CSV harvester and folded into the candidate set.
    """
    scan_to_index = max(0, min(int(scan_to_index or 0), MAX_SCAN_TO_INDEX))
    pre_warnings: list[str] = []
    merged_candidates = list(candidates or [])
    if csv_text:
        harvested = extract_candidates_from_csv(csv_text)
        if len(harvested) > MAX_HARVEST_CANDIDATES:
            pre_warnings.append(
                f"CSV yielded {len(harvested)} candidates; only the first "
                f"{MAX_HARVEST_CANDIDATES} were checked."
            )
            harvested = harvested[:MAX_HARVEST_CANDIDATES]
        merged_candidates.extend(harvested)
    parsed, invalid = parse_tokens(addresses, txids, merged_candidates, file_text)
    wallets = load_profile_wallets(conn, profile_id, wallet_ids)
    index, warnings = build_owned_index(conn, profile_id, wallets, scan_to_index=scan_to_index)
    warnings = pre_warnings + warnings

    results: list[dict[str, Any]] = []
    for token in parsed:
        if token["type"] == "address":
            results.append(classify_address(token, index))
            continue
        txid = str(token["normalized"])
        legs = load_local_tx_legs(conn, profile_id, txid)
        if legs is None and verify_fetcher is not None:
            # The on-chain tier is best-effort enrichment: an unknown txid is a
            # backend 404 (a urllib HTTPError, not AppError) and is the common
            # reconciliation case, so a failed lookup must degrade this one
            # candidate to "unknown" rather than abort the whole batch.
            try:
                legs = verify_fetcher(txid, str(token.get("chain") or ""))
            except Exception as exc:  # noqa: BLE001 - network enrichment is non-fatal
                warnings.append(f"On-chain verify for {txid[:12]}…: {exc}")
                legs = None
        results.append(classify_txid(token, index, legs))

    for entry in invalid:
        results.append(
            {
                "input": entry["input"],
                "type": entry.get("type") or "invalid",
                "chain": "",
                "status": "invalid",
                "classification": "invalid",
                "note": entry["reason"],
            }
        )

    return {
        "results": results,
        "summary": summarize(results, wallets, scan_to_index, verify_fetcher is not None),
        "warnings": warnings,
    }


def summarize(
    results: Sequence[Mapping[str, Any]],
    wallets: Sequence[sqlite3.Row],
    scan_to_index: int,
    verified: bool,
) -> dict[str, Any]:
    counts: dict[str, int] = {"owned": 0, "external": 0, "unknown": 0, "invalid": 0}
    for item in results:
        status = str(item.get("status") or "")
        if status in counts:
            counts[status] += 1
    return {
        "total": len(results),
        "owned": counts["owned"],
        "external": counts["external"],
        "unknown": counts["unknown"],
        "invalid": counts["invalid"],
        "wallets_scanned": len(wallets),
        "scan_to_index": scan_to_index,
        "verified_on_chain": verified,
    }


def flatten_result_row(item: Mapping[str, Any]) -> dict[str, Any]:
    """Collapse a rich result into a uniform flat row for CSV / table output.

    Every row carries the same keys (CSV columns come from the first row), so
    address and txid results stay column-aligned regardless of order.
    """
    matches = item.get("matches") or []
    primary = matches[0] if matches else {}
    wallets = item.get("wallets") or []
    if not wallets and primary:
        wallets = [primary.get("wallet", "")]
    accounts = sorted({m.get("account", "") for m in matches if m.get("account")})
    return {
        "input": item.get("input", ""),
        "type": item.get("type", ""),
        "chain": item.get("chain", ""),
        "status": item.get("status", ""),
        "classification": item.get("classification", ""),
        "wallet": ", ".join(w for w in wallets if w),
        "account": ", ".join(accounts),
        "branch": primary.get("branch", "") if primary else "",
        "address_index": _blank_if_none(primary.get("address_index") if primary else None),
        "derivation_path": (primary.get("derivation_path") or "") if primary else "",
        "owned_inputs": _blank_if_none(item.get("owned_inputs")),
        "owned_outputs": _blank_if_none(item.get("owned_outputs")),
        "external_outputs": _blank_if_none(item.get("external_outputs")),
        "match_source": item.get("match_source") or (primary.get("match_source", "") if primary else ""),
        "note": item.get("note", ""),
    }


def _blank_if_none(value: Any) -> Any:
    return "" if value is None else value


def redact_result_for_ai(item: Mapping[str, Any]) -> dict[str, Any]:
    """AI-surface variant: keep the ownership verdict, drop wallet-structure
    detail (scriptPubKeys, derivation paths, address indices).

    Mirrors the ``_wallet_utxo_row_for_ai`` / ``snapshot_to_dict_for_ai``
    precedent: the model learns which wallet owns a candidate and the
    classification, never the descriptor geometry.
    """
    redacted: dict[str, Any] = {
        "input": item.get("input", ""),
        "type": item.get("type", ""),
        "chain": item.get("chain", ""),
        "status": item.get("status", ""),
        "classification": item.get("classification", ""),
        "note": item.get("note", ""),
    }
    wallets: list[str] = []
    matches = item.get("matches")
    if matches:
        wallets = sorted({m.get("wallet", "") for m in matches if m.get("wallet")})
    elif item.get("wallets"):
        wallets = [w for w in item["wallets"] if w]
    if wallets:
        redacted["wallets"] = wallets
    # Owned-address notes embed the branch + derivation index ("(change #0)");
    # rewrite to a geometry-free note so the AI surface matches the redaction of
    # branch/index/derivation_path elsewhere in this row.
    if item.get("classification") == "owned_address":
        owners = ", ".join(f"'{w}'" for w in wallets) or "a wallet in this profile"
        redacted["note"] = f"Owned by {owners}."
    for key in ("owned_inputs", "owned_outputs", "external_outputs", "match_source"):
        if item.get(key) is not None:
            redacted[key] = item.get(key)
    return redacted

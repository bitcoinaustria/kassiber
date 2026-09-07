"""Physical occurrence identity for explicitly retained block observations.

A TXID identifies serialized base transaction data, not a unique historical
block occurrence. Bare lookups become ambiguous when retained BIP30 occurrences
coexist. An explicit prevout occurrence never falls back to another occurrence.
Grant IDs identify observation sources, never physical transaction identity.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
import re

from .index import digest

_BLOCK_OCCURRENCE = re.compile(r"[0-9a-f]{64}:(?:0|[1-9][0-9]*)\Z")


@dataclass(frozen=True)
class SourceIdentity:
    domain_id: str
    namespace: str
    occurrence_id: str | None = None
    trusted_reference: bool = False


class OccurrenceResolver:
    def __init__(self, conn, profile_id):
        self.conn, self.profile_id = conn, profile_id
        tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.has_assertions = "chain_analysis_reference_assertions" in tables
        self.domains = {}
        self.binding = {"state": "unbound"}
        if "book_network_bindings" in tables:
            row = conn.execute("SELECT domains_json FROM book_network_bindings WHERE profile_id=?", (profile_id,)).fetchone()
            if row:
                self.domains = {(item["chain"], item["network"]): item for item in json.loads(row[0])}
                from ..book_network import resolve_book_environment
                self.binding = resolve_book_environment(conn, profile_id)
        self.cache = {}
        self.ambiguous = set()

    def accepts(self, row, *, unscoped=False):
        from ..book_network import observation_matches_binding
        if not observation_matches_binding(self.binding, row, unscoped=unscoped):
            return False
        domain = row.get("_index_domain_id") or row.get("domain_id")
        return not domain or not self.domains or domain in {value["domain_id"] for value in self.domains.values()}

    def source(self, row, chain, network):
        from ..onchain import stored_tx_mapping
        raw = stored_tx_mapping(row.get("raw_json")) or {}
        config = stored_tx_mapping(row.get("wallet_config_json")) or {}
        instance = row.get("chain_instance_id") or raw.get("chain_instance_id") or config.get("chain_instance_id")
        bound = self.domains.get((chain, network), {})
        domain = row.get("_index_domain_id") or (digest([chain, network, instance]) if instance else bound.get("domain_id"))
        domain = domain or digest([chain, network, None])
        # Unassigned regtest observations are deliberately separate from every
        # concrete lab instance. Standard network identity is globally stable.
        namespace = f"domain:{domain}:" if domain != digest([chain, network, None]) else ""
        occurrence = row.get("_index_occurrence_id")
        return SourceIdentity(domain, namespace, occurrence if isinstance(occurrence, str) and _BLOCK_OCCURRENCE.fullmatch(occurrence) else None, row.get("_index_reference") is True)

    def transaction_id(self, chain, network, txid, source, *, occurrence=None, resolve=True):
        occurrence = occurrence if isinstance(occurrence, str) and _BLOCK_OCCURRENCE.fullmatch(occurrence) else None
        if occurrence:
            return f"{chain}:{network}:domain:{source.domain_id}:occ:{occurrence}:tx:{txid}"
        candidates = self.candidates(source.domain_id, txid) if resolve else ()
        if len(candidates) == 1:
            return f"{chain}:{network}:domain:{source.domain_id}:occ:{candidates[0]}:tx:{txid}"
        ident = f"{chain}:{network}:{source.namespace}tx:{txid}"
        if len(candidates) > 1:
            self.ambiguous.add(ident)
        return ident

    def candidates(self, domain, txid):
        key = (domain, txid)
        if key not in self.cache:
            values = set()
            if self.has_assertions:
                for occurrence, status in self.conn.execute("SELECT occurrence_id,status_json FROM chain_analysis_reference_assertions WHERE profile_id=? AND active=1 AND txid=? AND domain_id=?", (self.profile_id, txid, domain)):
                    if _BLOCK_OCCURRENCE.fullmatch(occurrence):
                        try:
                            metadata = json.loads(status)
                        except (TypeError, ValueError):
                            continue
                        if metadata.get("block_hash") == occurrence.split(":", 1)[0]:
                            values.add(occurrence)
            self.cache[key] = tuple(sorted(values))
        return self.cache[key]

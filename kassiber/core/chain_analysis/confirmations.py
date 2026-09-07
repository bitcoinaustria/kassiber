"""Read confirmation depth from retained block anchors and a verified source tip.

Tip advances are a tiny overlay, not rewrites of every historical transaction.
The same mapper serves the full rebuild oracle and selected indexed node reads.
Counts are observations as of a verified tip, never estimates from wall time.
"""
from __future__ import annotations

import re

from ...time_utils import parse_iso_datetime_or_none
from ..onchain import stored_tx_mapping

_OCCURRENCE = re.compile(r"^[^:]+:[^:]+:domain:([^:]+):occ:([0-9a-f]{64}:(?:0|[1-9][0-9]*)):(?:tx|out):([0-9a-f]{64})(?::[0-9]+)?$")
TIP_FIELDS = ("verified_tip_height", "verified_tip_hash", "verified_tip_at", "last_code", "status")


class ConfirmationOverlay:
    def __init__(self, conn, profile_id):
        self.conn, self.profile_id = conn, profile_id
        columns = {row[1] for row in conn.execute("PRAGMA table_info(chain_analysis_acquisition_grants)")}
        self.available = set(TIP_FIELDS) <= columns
        self.cache = {}

    def apply(self, node):
        match = _OCCURRENCE.fullmatch(node["id"])
        if not self.available or not match:
            return node
        key = match.groups()
        if key not in self.cache:
            self.cache[key] = self._read(*key)
        return {**node, **self.cache[key]}

    def _read(self, domain, occurrence, txid):
        observations = []
        block = occurrence.split(":", 1)[0]
        rows = self.conn.execute("""SELECT a.status_json,g.verified_tip_height,g.verified_tip_hash,
            g.verified_tip_at,g.last_code FROM chain_analysis_reference_assertions a
            JOIN chain_analysis_acquisition_grants g ON g.id=a.grant_id AND g.profile_id=a.profile_id
            WHERE a.profile_id=? AND a.active=1 AND a.txid=? AND a.domain_id=? AND a.occurrence_id=?""",
            (self.profile_id, txid, domain, occurrence))
        for status, tip, tip_hash, observed_at, code in rows:
            status = stored_tx_mapping(status) or {}
            height = status.get("block_height")
            instant = parse_iso_datetime_or_none(observed_at)
            if (code in {"chain_analysis_stale", "reorg_reconciling"} or status.get("confirmed") is not True
                or status.get("removed") is True or status.get("conflicted") is True
                or status.get("block_hash") != block or type(height) is not int or height < 0
                or type(tip) is not int or tip < height or not isinstance(tip_hash, str)
                or re.fullmatch(r"[0-9a-f]{64}", tip_hash) is None or tip == height and tip_hash != block or instant is None or instant.tzinfo is None):
                continue
            observations.append((instant, height, tip - height + 1, observed_at))
        empty = {"confirmations": None, "confirmed": None, "block_height": None, "confirmation_observed_at": None}
        if not observations:
            return empty
        # Use the newest verified observation. Equally recent contradictory
        # sources cannot arbitrarily select a count or a block height.
        newest = max(item[0] for item in observations)
        latest = [item for item in observations if item[0] == newest]
        if len({(item[1], item[2]) for item in latest}) != 1:
            return empty
        _, height, count, observed_at = min(latest, key=lambda item: item[3])
        return {"confirmations": count, "confirmed": True, "block_height": height, "confirmation_observed_at": observed_at}

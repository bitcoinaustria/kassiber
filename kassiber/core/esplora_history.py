"""Conservative absence evidence for the Esplora compatibility observer.

An incomplete round retains every positively observed transaction. Only a
complete current history covering every previously tracked script can withdraw
one. This is local checkpoint state, not an accounting or ownership engine.
"""
from __future__ import annotations

from collections.abc import Mapping
from copy import deepcopy

from ..transfers import canonical_txid
from .onchain import input_script, output_script


MEMBERSHIP_VERSION = 1


def full_scan_checkpoint(checkpoint):
    """Discard scan caches while preserving candidates for absence comparison.

    Membership is observation history, including positives from incomplete
    rounds, rather than permission to skip fetching. The observer still checks
    its version, source binding and full current coverage before any retraction.
    """
    saved = checkpoint.get("esplora_history_memberships") if isinstance(checkpoint, Mapping) else None
    if not isinstance(saved, Mapping) or type(saved.get("version")) is not int or saved["version"] != MEMBERSHIP_VERSION:
        return {}
    return {"esplora_history_memberships": deepcopy(dict(saved))}


def explicit_history_count(stats):
    """Missing or malformed counters must never mean an empty history."""
    if not isinstance(stats, Mapping):
        return None
    counts = []
    for section in ("chain_stats", "mempool_stats"):
        values = stats.get(section)
        count = values.get("tx_count") if isinstance(values, Mapping) else None
        if type(count) is not int or count < 0:
            return None
        counts.append(count)
    return sum(counts)


def history_matches_script(rows, script):
    """A count-matched response for another script is not complete coverage."""
    for row in rows:
        if not isinstance(row, Mapping):
            return False
        inputs, outputs = row.get("vin"), row.get("vout")
        if not isinstance(inputs, list) or not isinstance(outputs, list):
            return False
        if not (
            any(input_script(entry) == script for entry in inputs if isinstance(entry, Mapping))
            or any(output_script(entry) == script for entry in outputs if isinstance(entry, Mapping))
        ):
            return False
    return True


class EsploraMemberships:
    def __init__(self, saved, backend_key):
        self.backend_key = backend_key
        self.prior = {}
        self.current = {}
        if (
            not isinstance(saved, Mapping)
            or type(saved.get("version")) is not int
            or saved.get("version") != MEMBERSHIP_VERSION
            or saved.get("backend_key") != backend_key
        ):
            return
        members = saved.get("scripts")
        if not isinstance(members, Mapping):
            return
        for script, entry in members.items():
            if not isinstance(script, str) or canonical_txid(script) != script or not isinstance(entry, Mapping):
                return
            txids = entry.get("txids")
            if not isinstance(txids, list) or any(not isinstance(txid, str) or canonical_txid(txid) != txid for txid in txids):
                return
        self.prior = dict(members)

    def reusable(self, script, fingerprint):
        entry = self.prior.get(script)
        if entry and entry.get("complete") is True and entry.get("fingerprint") == fingerprint:
            self.current[script] = dict(entry)
            return True
        return False

    def observe(self, script, rows, *, script_pubkey, fingerprint, complete):
        txids = [
            canonical_txid(row.get("txid")) if isinstance(row, Mapping) else None
            for row in rows
        ]
        valid = None not in txids and len(set(txids)) == len(txids)
        candidates = {
            txid for txid, row in zip(txids, rows)
            if txid and history_matches_script([row], script_pubkey)
        }
        self.current[script] = {
            "txids": sorted(candidates),
            "fingerprint": fingerprint,
            "complete": bool(complete and valid and len(candidates) == len(txids)),
        }

    def finish(self):
        covered = set(self.prior) <= set(self.current)
        complete = covered and all(entry["complete"] for entry in self.current.values())
        old_ids = {txid for entry in self.prior.values() for txid in entry["txids"]}
        current_ids = {txid for entry in self.current.values() for txid in entry["txids"]}
        retracted = sorted(old_ids - current_ids) if complete else []
        if complete:
            retained = self.current
        else:
            # Keep candidates from earlier complete and partial rounds, including
            # scripts omitted by a temporary scope shrink. A later full scan can
            # still withdraw them; no incomplete round destroys that evidence.
            retained = {}
            for script in self.prior.keys() | self.current.keys():
                prior = self.prior.get(script, {})
                current = self.current.get(script, {})
                retained[script] = {
                    "txids": sorted(set(prior.get("txids", ())) | set(current.get("txids", ()))),
                    "fingerprint": current.get("fingerprint"),
                    "complete": False,
                }
        return {
            "version": MEMBERSHIP_VERSION,
            "backend_key": self.backend_key,
            "scripts": dict(sorted(retained.items())),
        }, retracted, complete

"""Local journal certificates for reviewed routes realized by native history."""

from __future__ import annotations

from collections import defaultdict
import json
from typing import Any, Mapping, Sequence

from .custody_quantity import INTERNAL_VERIFIED


def replace_reconciliations(conn, profile, quantity_state, proofs: Sequence[Mapping[str, Any]], *, created_at: str) -> None:
    """Persist only complete selected native claims, in the journal transaction."""
    profile_id = str(profile["id"])
    conn.execute("DELETE FROM journal_custody_reconciliations WHERE profile_id = ?", (profile_id,))
    if quantity_state is None or not proofs:
        return
    selected: dict[str, int] = defaultdict(int)
    for decision in quantity_state.projection.decisions:
        if decision.state == INTERNAL_VERIFIED and decision.target is not None and decision.selected_claim_id:
            selected[decision.selected_claim_id] += decision.source.amount_msat
    for proof in proofs:
        expected = dict(proof["native_claim_amounts"])
        if not expected or any(selected[claim_id] != amount for claim_id, amount in expected.items()):
            continue
        component = conn.execute(
            "SELECT revision, state FROM custody_components WHERE profile_id = ? AND id = ?",
            (profile_id, proof["component_id"]),
        ).fetchone()
        if component is None or component["state"] != "active" or component["revision"] != proof["component_revision"]:
            continue
        conn.execute(
            """INSERT INTO journal_custody_reconciliations(
                profile_id, component_id, input_version, component_revision, proof_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)""",
            (profile_id, proof["component_id"], int(profile["journal_input_version"] or 0),
             proof["component_revision"], json.dumps(proof, sort_keys=True, separators=(",", ":")), created_at),
        )


def load_current_reconciliations(conn, profile_id: str) -> tuple[dict[str, Any], ...]:
    """Read the current derived proof; never re-run custody interpretation."""
    from .custody_journal import projection_freshness
    if not projection_freshness(conn, profile_id)["is_current"]:
        return ()
    rows = conn.execute(
        """SELECT proof.proof_json FROM journal_custody_reconciliations proof
        JOIN profiles profile ON profile.id = proof.profile_id
        JOIN custody_components component ON component.id = proof.component_id
            AND component.profile_id = proof.profile_id
        WHERE proof.profile_id = ? AND component.state = 'active'
            AND component.revision = proof.component_revision
            AND profile.journal_input_version = proof.input_version""",
        (profile_id,),
    ).fetchall()
    return tuple(json.loads(row["proof_json"]) for row in rows)

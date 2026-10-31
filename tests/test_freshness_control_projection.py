"""Freshness controls expose the shared public projection, never checkpoints."""

import json

import pytest

from kassiber import daemon_freshness
from kassiber.ai.tools import redact_ai_tool_result
from kassiber.core import freshness
from kassiber.db import open_db, set_setting
from kassiber.envelope import build_envelope


@pytest.mark.parametrize("action", ["cancel", "pause", "resume"])
def test_source_controls_keep_private_checkpoint_in_database_only(tmp_path, action):
    conn = open_db(tmp_path / "data")
    try:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Set','fixed')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','fixed')")
        set_setting(conn, "context_workspace", "w")
        set_setting(conn, "context_profile", "p")
        key = "onchain_wallet:wallet"
        job = freshness.enqueue_job(
            conn, profile_id="p", job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key=key, source_type=freshness.SOURCE_ONCHAIN, source_label="Wallet",
        )
        checkpoint = {"script_membership": {"synthetic": ["private-script-member"]}}
        conn.execute("UPDATE freshness_source_states SET checkpoint_json=?,paused=?",
                     (json.dumps(checkpoint), int(action == "resume")))
        conn.commit()
        args = {"job_id": job["id"]} if action == "cancel" else {"source_key": key}
        payload = daemon_freshness._freshness_control_payload(conn, args, action=action)
        assert "checkpoint" not in payload["result"]
        for result in (payload, build_envelope(f"ui.freshness.{action}", payload), redact_ai_tool_result(payload)):
            assert "private-script-member" not in json.dumps(result)
        state = freshness.get_source_state(conn, "p", key)
        assert state["checkpoint"] == checkpoint
        if action == "cancel":
            assert payload["result"]["status"] == "cancelled"
        else:
            assert payload["result"]["paused"] is (action == "pause")
            assert state["paused"] is (action == "pause")
            assert payload["result"]["source_key"] == key
    finally:
        conn.close()

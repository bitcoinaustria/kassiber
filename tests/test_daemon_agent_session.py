import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

from kassiber import daemon, daemon_agent_session
from kassiber.agent_access import PATH_ENV, set_agent_access
from kassiber.errors import AppError


class FakeBroker:
    """Stands in for BrokerClient with the broker's lease semantics."""

    def __init__(self, *, lease_id=None):
        self.calls: list[tuple] = []
        self.lease_id = lease_id  # a live lease someone already holds
        self.stopped = False
        self.lock_fails = False
        self.previous_mode = "manual"  # what the broker's grant replaces
        self.connection_lost = False  # lock looks "stopped" but the broker lives

    def __call__(self):
        return self

    def unlock(self, data_root, passphrase, **options):
        self.calls.append(("unlock", data_root, bytes(passphrase), options))
        if options.get("only_if_locked") and self.lease_id is not None:
            raise AppError("exists", code="operator_lease_exists")
        self.lease_id = "desktop-lease"
        return {
            "lease": "unlocked",
            "lease_id": self.lease_id,
            "expires_at": "2026-09-28T18:00:00Z",
            "previous_mode": self.previous_mode,
        }

    def ping(self):
        self.calls.append(("ping",))
        if self.stopped:
            raise ConnectionRefusedError()
        return {"generation": "g"}

    def lock(self, data_root, **conditions):
        self.calls.append(("lock", data_root, conditions))
        if self.lock_fails:
            raise AppError("busy", code="operator_protocol_error")
        if self.stopped or self.connection_lost:
            return {"broker": "stopped", "locked": True, "lease_existed": False}
        expected = conditions.get("expected_lease_id")
        if self.lease_id is not None and expected is not None and expected != self.lease_id:
            return {"locked": False, "lease_existed": True, "kept": True}
        self.lease_id = None
        return {"locked": True, "mode_restored": conditions.get("restore_manual", False)}


class DesktopAgentSessionTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        root = Path(self._tmp.name)
        env = patch.dict(os.environ, {PATH_ENV: str(root / "agent-access.json")})
        env.start()
        self.addCleanup(env.stop)
        set_agent_access(mcp_enabled=True)
        self.ctx = SimpleNamespace(
            conn=object(), db_passphrase="correct horse", data_root=str(root / "data"), agent_lease=None
        )
        self.mode = "manual"
        mode = patch.object(daemon_agent_session, "_unlock_mode", side_effect=lambda _root: self.mode)
        mode.start()
        self.addCleanup(mode.stop)
        local_restore = patch("kassiber.operator.modes.set_unlock_mode")
        self.local_restore = local_restore.start()
        self.addCleanup(local_restore.stop)

    def _with_broker(self, broker):
        patcher = patch("kassiber.operator.client.BrokerClient", broker)
        patcher.start()
        self.addCleanup(patcher.stop)
        return broker

    def _refuse_broker(self, reason):
        def refuse():
            raise AssertionError(reason)

        return self._with_broker(refuse)

    def test_status_never_contacts_the_broker(self):
        self._refuse_broker("status must not construct a broker client")
        self.assertEqual(
            daemon_agent_session.session_state(self.ctx),
            {"needed": True, "active": False, "expires_at": None},
        )
        payload = daemon._agent_access_payload(self.ctx, "ui.agent_access.status", {"args": {}})
        self.assertTrue(payload["mcp_available"])
        self.assertFalse(payload["session"]["active"])

    def test_unlock_grants_a_read_lease_only_if_none_exists(self):
        broker = self._with_broker(FakeBroker())

        state = daemon_agent_session.unlock(self.ctx)

        self.assertEqual(state, {"needed": True, "active": True, "expires_at": "2026-09-28T18:00:00Z"})
        _, data_root, secret, options = broker.calls[0]
        self.assertEqual(data_root, self.ctx.data_root)
        self.assertEqual(secret, b"correct horse")
        self.assertEqual(
            options,
            {
                "duration_seconds": daemon_agent_session.DESKTOP_AGENT_LEASE_SECONDS,
                "capability": "read",
                "authentication_method": "password",
                "only_if_locked": True,
            },
        )
        # Asking again is idempotent and does not reach the broker.
        daemon_agent_session.unlock(self.ctx)
        self.assertEqual(len(broker.calls), 1)

    def test_locking_names_the_lease_and_restores_manual_in_the_broker(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)

        daemon_agent_session.lock(self.ctx)

        self.assertEqual(
            broker.calls[-1],
            ("lock", self.ctx.data_root, {"expected_lease_id": "desktop-lease", "restore_manual": True}),
        )
        self.local_restore.assert_not_called()
        self.assertIsNone(self.ctx.agent_lease)

    def test_only_a_book_the_broker_saw_as_manual_is_restored(self):
        # The desktop read "manual", but the mode changed before the grant.
        broker = self._with_broker(FakeBroker())
        broker.previous_mode = "brokered"
        daemon_agent_session.unlock(self.ctx)
        daemon_agent_session.lock(self.ctx)
        self.assertFalse(broker.calls[-1][2]["restore_manual"])

    def test_an_existing_lease_is_neither_replaced_nor_locked(self):
        broker = self._with_broker(FakeBroker(lease_id="terminal-lease"))

        state = daemon_agent_session.unlock(self.ctx)

        self.assertTrue(state["existing_lease"])
        self.assertFalse(state["active"])
        self.assertIsNone(self.ctx.agent_lease)
        daemon_agent_session.end_lease(self.ctx)
        self.assertEqual([call[0] for call in broker.calls], ["unlock"])
        self.assertEqual(broker.lease_id, "terminal-lease")

    def test_a_stopped_broker_ends_the_lease_without_touching_the_mode(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        broker.stopped = True

        daemon_agent_session.end_lease(self.ctx)

        # Its leases ended with it; writing the mode here could race a new
        # broker and a new terminal lease.
        self.assertIsNone(self.ctx.agent_lease)
        self.local_restore.assert_not_called()

    def test_a_failed_lock_keeps_the_lease_visible_and_retries(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        broker.lock_fails = True

        with self.assertRaises(AppError) as raised:
            daemon_agent_session.lock(self.ctx)

        self.assertEqual(raised.exception.code, "agent_lease_lock_failed")
        self.assertTrue(daemon_agent_session.session_state(self.ctx)["active"])
        # Session-end paths do not raise, and keep the lease for the loop.
        daemon._clear_unlocked_passphrase(self.ctx)
        self.assertTrue(self.ctx.agent_lease.ending)
        broker.lock_fails = False
        daemon_agent_session.expire(self.ctx)  # before the retry time
        self.assertIsNotNone(self.ctx.agent_lease)
        with patch.object(
            daemon_agent_session.time,
            "monotonic",
            return_value=self.ctx.agent_lease.retry_at + 1,
        ):
            daemon_agent_session.expire(self.ctx)
        self.assertIsNone(self.ctx.agent_lease)
        self.assertIsNone(broker.lease_id)

    def test_a_lost_connection_is_not_taken_for_a_stopped_broker(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        broker.connection_lost = True

        daemon_agent_session.end_lease(self.ctx)

        self.assertTrue(self.ctx.agent_lease.ending)
        self.local_restore.assert_not_called()

    def test_the_local_deadline_starts_before_the_grant_request(self):
        self._with_broker(FakeBroker())
        with patch.object(daemon_agent_session.time, "monotonic", side_effect=[100.0, 250.0, 250.0]):
            daemon_agent_session.unlock(self.ctx)
        self.assertEqual(
            self.ctx.agent_lease.expires_monotonic,
            100.0 + daemon_agent_session.DESKTOP_AGENT_LEASE_SECONDS,
        )
        # Giving up waits from after the grant, when the broker's lease began.
        self.assertEqual(
            self.ctx.agent_lease.give_up_monotonic,
            250.0
            + daemon_agent_session.DESKTOP_AGENT_LEASE_SECONDS
            + daemon_agent_session._GIVE_UP_AFTER_EXPIRY_SECONDS,
        )

    def test_cleanup_gives_up_once_the_lease_is_long_expired(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        broker.lock_fails = True
        late = self.ctx.agent_lease.give_up_monotonic + 1
        with patch.object(daemon_agent_session.time, "monotonic", return_value=late):
            daemon_agent_session.expire(self.ctx)
        self.assertIsNone(self.ctx.agent_lease)
        self.local_restore.assert_not_called()

    def test_an_expired_lease_is_not_shown_and_is_cleaned_up(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        broker.lease_id = None  # the broker expired it on its own
        with patch.object(
            daemon_agent_session.time,
            "monotonic",
            return_value=self.ctx.agent_lease.expires_monotonic + 1,
        ):
            self.assertFalse(daemon_agent_session.session_state(self.ctx)["active"])
            daemon_agent_session.expire(self.ctx)

        self.assertIsNone(self.ctx.agent_lease)
        self.assertEqual(broker.calls[-1][0], "lock")
        self.assertTrue(broker.calls[-1][2]["restore_manual"])

    def test_expire_leaves_a_live_lease_alone(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)
        daemon_agent_session.expire(self.ctx)
        self.assertIsNotNone(self.ctx.agent_lease)
        self.assertEqual(len(broker.calls), 1)

    def test_unlock_requires_agents_on(self):
        self._refuse_broker("no broker contact while agents are off")
        set_agent_access(mcp_enabled=False)
        with self.assertRaises(AppError) as raised:
            daemon_agent_session.unlock(self.ctx)
        self.assertEqual(raised.exception.code, "mcp_disabled")

    def test_unlock_requires_an_unlocked_encrypted_book(self):
        self._refuse_broker("no broker contact without a passphrase")
        self.ctx.db_passphrase = None
        self.assertFalse(daemon_agent_session.session_state(self.ctx)["needed"])
        with self.assertRaises(AppError) as raised:
            daemon_agent_session.unlock(self.ctx)
        self.assertEqual(raised.exception.code, "agent_unlock_unavailable")

    def test_a_remembered_unlock_book_needs_no_agent_unlock(self):
        self._refuse_broker("no broker contact for unattended books")
        self.mode = "unattended"
        self.assertFalse(daemon_agent_session.session_state(self.ctx)["needed"])
        with self.assertRaises(AppError) as raised:
            daemon_agent_session.unlock(self.ctx)
        self.assertEqual(raised.exception.code, "agent_unlock_not_needed")

    def test_ending_the_desktop_session_ends_the_agent_lease(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)

        daemon._clear_unlocked_passphrase(self.ctx)

        self.assertEqual(broker.calls[-1][0], "lock")
        self.assertIsNone(broker.lease_id)
        self.assertIsNone(self.ctx.db_passphrase)

    def test_turning_agents_off_ends_the_agent_lease(self):
        broker = self._with_broker(FakeBroker())
        daemon_agent_session.unlock(self.ctx)

        payload = daemon._agent_access_payload(
            self.ctx, "ui.agent_access.configure", {"args": {"mcp_enabled": False}}
        )

        self.assertFalse(payload["mcp_available"])
        self.assertEqual(broker.calls[-1][0], "lock")
        self.assertFalse(payload["session"]["active"])

    def test_a_mock_context_never_reaches_the_broker(self):
        self._refuse_broker("mock contexts must not reach the broker")
        daemon_agent_session.end_lease(MagicMock())
        daemon_agent_session.expire(MagicMock())

    def test_unlock_and_lock_take_no_arguments(self):
        with self.assertRaises(AppError) as raised:
            daemon._agent_access_payload(
                self.ctx, "ui.agent_access.unlock", {"args": {"capability": "admin"}}
            )
        self.assertEqual(raised.exception.code, "validation")


if __name__ == "__main__":
    unittest.main()

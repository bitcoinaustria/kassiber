import json
import logging
import os
import threading
import unittest

from kassiber.log_ring import (
    LogRing,
    RingHandler,
    current_request_id,
    get_log_ring,
    install_ring_logging,
    relativize_path,
    sanitize_exception,
    sanitize_traceback_text,
)
from kassiber.redaction import _stable_hash, redact_operational_text

# A real-shaped (all-hex) txid; the value never matters, only that it is 64 hex.
TXID = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"

# BIP32 test-vector keys: public, Bitcoin-shaped material safe for fixtures.
XPRV = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"
XPUB = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8"


def _append_n(ring, count, prefix="msg"):
    return [
        ring.append("info", "kassiber.test", "kassiber/daemon.py", i + 1, f"{prefix} {i + 1}")
        for i in range(count)
    ]


class LogRingSnapshotTest(unittest.TestCase):
    def test_ids_are_monotonic_from_one(self):
        ring = LogRing()
        self.assertEqual(_append_n(ring, 5), [1, 2, 3, 4, 5])

    def test_snapshot_cursor_limit_and_order(self):
        ring = LogRing()
        _append_n(ring, 5)
        snap = ring.snapshot(after_id=2, limit=2)
        self.assertEqual([r["id"] for r in snap["records"]], [3, 4])
        self.assertEqual(snap["last_id"], 5)
        self.assertFalse(snap["gap"])

        full = ring.snapshot()
        self.assertEqual([r["id"] for r in full["records"]], [1, 2, 3, 4, 5])
        self.assertEqual(
            set(full.keys()),
            {"records", "last_id", "gap", "started_at", "buffer_bytes", "max_bytes"},
        )

    def test_snapshot_empty_ring(self):
        ring = LogRing()
        snap = ring.snapshot()
        self.assertEqual(snap["records"], [])
        self.assertEqual(snap["last_id"], 0)
        self.assertFalse(snap["gap"])
        self.assertEqual(snap["buffer_bytes"], 0)
        self.assertTrue(snap["started_at"].endswith("Z"))

    def test_gap_after_eviction_past_cursor(self):
        ring = LogRing(max_records=3)
        _append_n(ring, 5)
        snap = ring.snapshot(after_id=0)
        self.assertEqual([r["id"] for r in snap["records"]], [3, 4, 5])
        self.assertTrue(snap["gap"])
        self.assertTrue(ring.snapshot(after_id=1)["gap"])
        self.assertFalse(ring.snapshot(after_id=2)["gap"])
        self.assertFalse(ring.snapshot(after_id=4)["gap"])

    def test_count_bound_evicts_oldest_first(self):
        ring = LogRing(max_records=2)
        _append_n(ring, 4)
        snap = ring.snapshot()
        self.assertEqual([r["id"] for r in snap["records"]], [3, 4])
        self.assertEqual(snap["last_id"], 4)

    def test_byte_bound_evicts_oldest_first(self):
        ring = LogRing(max_records=1000, max_bytes=600)
        for i in range(10):
            ring.append("info", "kassiber.test", "kassiber/daemon.py", i, "x" * 80)
        snap = ring.snapshot()
        self.assertLessEqual(snap["buffer_bytes"], 600)
        self.assertLess(len(snap["records"]), 10)
        self.assertEqual(snap["records"][-1]["id"], 10)
        self.assertTrue(snap["gap"])
        self.assertEqual(snap["max_bytes"], 600)
        self.assertEqual(ring.buffer_bytes, snap["buffer_bytes"])

    def test_oversized_record_keeps_newest(self):
        ring = LogRing(max_records=10, max_bytes=50)
        ring.append("info", "kassiber.test", "kassiber/daemon.py", 1, "x" * 200)
        snap = ring.snapshot()
        self.assertEqual([r["id"] for r in snap["records"]], [1])

    def test_thread_safety_under_concurrent_append(self):
        ring = LogRing()

        def worker():
            for _ in range(200):
                ring.append("debug", "kassiber.test", "kassiber/daemon.py", 1, "tick")

        threads = [threading.Thread(target=worker) for _ in range(8)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        snap = ring.snapshot(limit=2000)
        self.assertEqual(snap["last_id"], 1600)
        ids = [r["id"] for r in snap["records"]]
        self.assertEqual(ids, sorted(set(ids)))


class SecretFloorTest(unittest.TestCase):
    def test_descriptor_in_msg_redacted_at_insert(self):
        ring = LogRing()
        ring.append("info", "kassiber.test", "kassiber/daemon.py", 1, f"loaded wpkh({XPUB}/0/*)")
        msg = ring.snapshot()["records"][0]["msg"]
        self.assertNotIn("xpub661", msg)
        self.assertIn("[redacted", msg)

    def test_api_key_assignment_in_msg_redacted(self):
        ring = LogRing()
        ring.append("info", "kassiber.test", "kassiber/daemon.py", 1, "auth with api_key=kb_live_9f3a7c21")
        msg = ring.snapshot()["records"][0]["msg"]
        self.assertNotIn("kb_live_9f3a7c21", msg)
        self.assertIn("api_key=[redacted]", msg)

    def test_text_field_values_redacted_at_insert(self):
        ring = LogRing()
        ring.append(
            "info",
            "kassiber.test",
            "kassiber/daemon.py",
            1,
            "wallet connected",
            fields={
                "note": {"type": "text", "value": f"descriptor=wpkh({XPUB}/0/*)"},
                "auth": {"type": "text", "value": "api_key=kb_live_9f3a7c21"},
                "txid": {
                    "type": "txid",
                    "value": "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
                },
            },
        )
        fields = ring.snapshot()["records"][0]["fields"]
        self.assertNotIn("xpub661", fields["note"]["value"])
        self.assertNotIn("kb_live_9f3a7c21", fields["auth"]["value"])
        # Operational kinds stay verbatim: txids are redacted at render time.
        self.assertEqual(
            fields["txid"]["value"],
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
        )

    def test_secret_typed_field_value_floored_at_insert(self):
        # The insert floor is type-agnostic: a secret riding in under a
        # non-`text` type (here `api_key`/`descriptor`) must still be scrubbed,
        # not left for render-time masking.
        ring = LogRing()
        ring.append(
            "info",
            "kassiber.test",
            "kassiber/daemon.py",
            1,
            "wallet import",
            fields={
                "key": {"type": "api_key", "value": "sk-live-ABC123secret"},
                "descr": {"type": "descriptor", "value": f"wpkh({XPUB}/0/*)"},
            },
        )
        fields = ring.snapshot()["records"][0]["fields"]
        self.assertNotIn("sk-live-ABC123secret", fields["key"]["value"])
        self.assertIn("[redacted", fields["key"]["value"])
        self.assertNotIn("xpub661", fields["descr"]["value"])

    def test_absolute_file_paths_never_stored(self):
        ring = LogRing()
        ring.append(
            "info",
            "kassiber.daemon",
            "/Users/someone/Github/kassiber/kassiber/daemon.py",
            9120,
            "request finished",
        )
        record = ring.snapshot()["records"][0]
        self.assertEqual(record["file"], "kassiber/daemon.py")


class RequestIdTest(unittest.TestCase):
    def test_contextvar_stamps_request_id_field(self):
        ring = LogRing()
        token = current_request_id.set("tauri-3")
        try:
            ring.append("info", "kassiber.daemon", "kassiber/daemon.py", 1, "request finished")
        finally:
            current_request_id.reset(token)
        fields = ring.snapshot()["records"][0]["fields"]
        self.assertEqual(fields["request_id"], {"type": "text", "value": "tauri-3"})

    def test_explicit_request_id_wins_over_contextvar(self):
        ring = LogRing()
        token = current_request_id.set("tauri-3")
        try:
            ring.append(
                "info", "kassiber.daemon", "kassiber/daemon.py", 1, "done", request_id="cli-1"
            )
        finally:
            current_request_id.reset(token)
        fields = ring.snapshot()["records"][0]["fields"]
        self.assertEqual(fields["request_id"]["value"], "cli-1")

    def test_no_request_id_field_when_unset(self):
        ring = LogRing()
        ring.append("info", "kassiber.daemon", "kassiber/daemon.py", 1, "done")
        self.assertNotIn("request_id", ring.snapshot()["records"][0]["fields"])


class RingHandlerTest(unittest.TestCase):
    def _logger(self, ring):
        logger = logging.getLogger("kassiber.test.ring_handler")
        logger.propagate = False
        logger.setLevel(1)
        handler = RingHandler(ring)
        logger.addHandler(handler)
        self.addCleanup(logger.removeHandler, handler)
        return logger

    def test_level_mapping(self):
        ring = LogRing()
        logger = self._logger(ring)
        logger.log(5, "below debug")
        logger.debug("debug")
        logger.info("info")
        logger.warning("warning")
        logger.error("error")
        logger.critical("critical")
        levels = [r["level"] for r in ring.snapshot()["records"]]
        self.assertEqual(levels, ["trace", "debug", "info", "warning", "error", "error"])

    def test_record_shape_from_stdlib(self):
        ring = LogRing()
        logger = self._logger(ring)
        logger.info("synced %d outputs", 3)
        record = ring.snapshot()["records"][0]
        self.assertEqual(record["module"], "kassiber.test.ring_handler")
        self.assertEqual(record["msg"], "synced 3 outputs")
        self.assertEqual(record["file"], "tests/test_log_ring.py")
        self.assertGreater(record["line"], 0)

    def test_kb_fields_passthrough(self):
        ring = LogRing()
        logger = self._logger(ring)
        logger.info(
            "sync finished",
            extra={"kb_fields": {"duration_ms": {"type": "duration_ms", "value": 12}}},
        )
        fields = ring.snapshot()["records"][0]["fields"]
        self.assertEqual(fields["duration_ms"], {"type": "duration_ms", "value": 12})

    def test_exc_info_captures_sanitized_traceback(self):
        ring = LogRing()
        logger = self._logger(ring)
        try:
            raise ValueError(f"backend rejected key {XPRV}")
        except ValueError:
            logger.error("sync failed", exc_info=True)
        record = ring.snapshot()["records"][0]
        tb = record["fields"]["traceback"]
        self.assertEqual(tb["type"], "text")
        self.assertIn("ValueError", tb["value"])
        self.assertIn("tests/test_log_ring.py", tb["value"])
        self.assertNotIn("xprv9s21", tb["value"])
        self.assertIn("[redacted-private-key]", tb["value"])
        self.assertNotIn("/Users/", tb["value"])

    def test_formatting_errors_do_not_raise(self):
        ring = LogRing()
        logger = self._logger(ring)
        logger.info("synced %d outputs", "not-a-number")
        records = ring.snapshot()["records"]
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["msg"], "synced %d outputs")


class SanitizeTest(unittest.TestCase):
    def test_sanitize_exception_relativizes_and_redacts(self):
        try:
            raise ValueError(f"backend rejected key {XPRV}")
        except ValueError as exc:
            text = sanitize_exception(exc)
        self.assertIn("Traceback", text)
        self.assertIn("tests/test_log_ring.py", text)
        self.assertNotIn("/Users/", text)
        self.assertNotIn("xprv9s21", text)
        self.assertIn("[redacted-private-key]", text)

    def test_sanitize_traceback_text_caps_length(self):
        text = "a" * 1000 + "b" * 9000
        capped = sanitize_traceback_text(text)
        self.assertIn("...[truncated]...", capped)
        self.assertTrue(capped.startswith("a" * 1000))
        self.assertTrue(capped.endswith("b" * 7000))
        self.assertEqual(len(capped), 1000 + len("...[truncated]...") + 7000)

    def test_sanitize_traceback_text_short_input_unchanged(self):
        self.assertEqual(sanitize_traceback_text("short message"), "short message")

    def test_sanitize_traceback_pseudonymizes_txid(self):
        # A backend exception message that interpolates a txid:vout outpoint must
        # not ride raw into the ring traceback field / error.debug / CLI export.
        text = f'  raise AppError("Liquid UTXO {TXID}:3 did not match")'
        sanitized = sanitize_traceback_text(text)
        self.assertNotIn(TXID, sanitized)
        self.assertIn(f"txid#{_stable_hash(TXID)}", sanitized)
        self.assertIn(":3", sanitized)  # vout stays readable


class OperationalRedactionTest(unittest.TestCase):
    def test_txid_becomes_stable_pseudonym(self):
        out = redact_operational_text(f"could not price {TXID} now")
        self.assertNotIn(TXID, out)
        self.assertEqual(out, f"could not price txid#{_stable_hash(TXID)} now")

    def test_same_txid_same_token_distinct_txids_distinct(self):
        other = "f" * 64
        out = redact_operational_text(f"{TXID} and {TXID} but not {other}")
        token = f"txid#{_stable_hash(TXID)}"
        self.assertEqual(out.count(token), 2)
        self.assertNotEqual(_stable_hash(TXID), _stable_hash(other))

    def test_unit_tagged_amounts_pseudonymized(self):
        for text, raw in (
            ("moving 12345678 sats", "12345678 sats"),
            ("value 0.0123 BTC here", "0.0123 BTC"),
            ("paid € 4500.00 today", "€ 4500.00"),
        ):
            out = redact_operational_text(text)
            self.assertNotIn(raw, out)
            self.assertIn("amount#", out)

    def test_market_rate_stays_readable(self):
        # A BTC/EUR rate is public market data, not the user's amount.
        out = redact_operational_text("rate BTC/EUR 64000.12 applied")
        self.assertEqual(out, "rate BTC/EUR 64000.12 applied")

    def test_bare_integer_left_unchanged(self):
        # No unit/symbol -> cannot be safely auto-detected; documented limitation.
        self.assertEqual(
            redact_operational_text("amount 12345678 invalid"),
            "amount 12345678 invalid",
        )

    def test_addresses_left_readable(self):
        # Owner scope is txid + amount only; addresses stay readable.
        text = "spent to bc1qexampleaddress000000000000000000000"
        self.assertEqual(redact_operational_text(text), text)

    def test_fnv_matches_webview_contract(self):
        # The daemon and the webview (ui-tauri/src/lib/appLogs.ts::stableHash)
        # MUST produce identical tokens so a value pseudonymized on either side
        # collapses to one token in the merged stream. This pins the contract;
        # appLogs.test.ts asserts the same literal from the TS side.
        self.assertEqual(_stable_hash("a" * 64), "d96f0f85")


class RelativizePathTest(unittest.TestCase):
    def test_site_packages_segment(self):
        self.assertEqual(
            relativize_path("/Users/dev/.venv/lib/python3.13/site-packages/embit/base58.py"),
            "embit/base58.py",
        )

    def test_repo_package_segments(self):
        self.assertEqual(
            relativize_path("/Users/dev/Github/kassiber/kassiber/daemon.py"),
            "kassiber/daemon.py",
        )
        self.assertEqual(
            relativize_path("/Users/dev/Github/kassiber/tests/test_log_ring.py"),
            "tests/test_log_ring.py",
        )

    def test_already_relative_unchanged(self):
        self.assertEqual(relativize_path("kassiber/db.py"), "kassiber/db.py")
        self.assertEqual(relativize_path("tests/test_db.py"), "tests/test_db.py")

    def test_home_prefix_replaced(self):
        home = os.path.expanduser("~").rstrip("/")
        self.assertEqual(relativize_path(f"{home}/somewhere/odd.py"), "~/somewhere/odd.py")

    def test_basename_fallback(self):
        self.assertEqual(
            relativize_path("/usr/lib/python3.13/logging/__init__.py"),
            "__init__.py",
        )

    def test_windows_drive_path_keeps_repo_segment(self):
        self.assertEqual(
            relativize_path(r"C:\dev\Github\kassiber\kassiber\daemon.py"),
            "kassiber/daemon.py",
        )

    def test_windows_user_profile_path_drops_username(self):
        # A Windows user-profile path inspected on a non-Windows host must not
        # leak the OS username; it falls through to the basename.
        relativized = relativize_path(r"C:\Users\alice\Documents\report.py")
        self.assertNotIn("alice", relativized)
        self.assertEqual(relativized, "report.py")

    def test_unc_path_drops_host_and_share(self):
        relativized = relativize_path(r"\\fileserver\team\alice\secret.py")
        self.assertNotIn("alice", relativized)
        self.assertEqual(relativized, "secret.py")

    def test_sanitize_traceback_strips_windows_username(self):
        text = (
            "Traceback (most recent call last):\n"
            + r'  File "C:\Users\alice\Documents\daemon.py", line 9, in run' + "\n"
            + r'    raise RuntimeError("boom from C:\Users\alice\wallet.sqlite")' + "\n"
            + "RuntimeError: boom\n"
        )
        sanitized = sanitize_traceback_text(text)
        self.assertNotIn("alice", sanitized)
        self.assertIn("RuntimeError", sanitized)

    def test_username_with_space_does_not_survive(self):
        # A space in the user directory must not stop path matching early and
        # leak the unmatched suffix (the username) into the sanitized text.
        self.assertNotIn(
            "John Doe",
            relativize_path("/Users/John Doe/Documents/wallet.dat"),
        )
        self.assertNotIn(
            "John Doe",
            relativize_path(r"C:\Users\John Doe\AppData\daemon.py"),
        )

    def test_sanitize_traceback_strips_spaced_username(self):
        text = (
            "Traceback (most recent call last):\n"
            + '  File "/Users/John Doe/proj/app.py", line 3, in run\n'
            + r'    raise RuntimeError("at C:\Users\John Doe\wallet.sqlite")' + "\n"
            + "RuntimeError: boom\n"
        )
        sanitized = sanitize_traceback_text(text)
        self.assertNotIn("John Doe", sanitized)
        self.assertIn("RuntimeError", sanitized)

    def test_empty_passthrough(self):
        self.assertEqual(relativize_path(""), "")


class InstallTest(unittest.TestCase):
    def _cleanup_root(self):
        root = logging.getLogger()
        prior_level = root.level

        def restore():
            for handler in list(root.handlers):
                if isinstance(handler, RingHandler):
                    root.removeHandler(handler)
            root.setLevel(prior_level)

        self.addCleanup(restore)
        return root

    def test_install_ring_logging_is_idempotent(self):
        root = self._cleanup_root()
        first = install_ring_logging()
        second = install_ring_logging()
        self.assertIs(first, second)
        self.assertIs(get_log_ring(), first)
        ring_handlers = [h for h in root.handlers if isinstance(h, RingHandler)]
        self.assertEqual(len(ring_handlers), 1)
        self.assertEqual(root.level, logging.DEBUG)

    def test_get_log_ring_does_not_touch_root_logger(self):
        root = self._cleanup_root()
        before = list(root.handlers)
        before_level = root.level
        self.assertIsInstance(get_log_ring(), LogRing)
        self.assertEqual(root.handlers, before)
        self.assertEqual(root.level, before_level)


class WireSafetyTest(unittest.TestCase):
    def test_every_snapshot_payload_is_json_serializable(self):
        ring = LogRing()
        logger = logging.getLogger("kassiber.test.wire_safety")
        logger.propagate = False
        logger.setLevel(1)
        handler = RingHandler(ring)
        logger.addHandler(handler)
        self.addCleanup(logger.removeHandler, handler)

        ring.append(
            "info",
            "kassiber.daemon",
            "/Users/someone/Github/kassiber/kassiber/daemon.py",
            9120,
            f"loaded wpkh({XPUB}/0/*)",
            fields={"unshaped": object(), "flag": {"type": "boolean", "value": True}},
        )
        try:
            raise ValueError(f"backend rejected key {XPRV}")
        except ValueError:
            logger.error("sync failed", exc_info=True)
        _append_n(ring, 5)

        for after_id in (0, 1, 3, 99):
            payload = json.dumps(ring.snapshot(after_id=after_id))
            self.assertNotIn("xprv9s21", payload)
            self.assertNotIn("xpub661", payload)
        roundtrip = json.loads(json.dumps(ring.snapshot()))
        self.assertEqual(roundtrip["last_id"], 7)
        self.assertEqual(
            roundtrip["records"][0]["fields"]["flag"], {"type": "boolean", "value": True}
        )


if __name__ == "__main__":
    unittest.main()

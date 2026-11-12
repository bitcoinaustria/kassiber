"""``tests/tax_oracle/compare_book.py`` only ever opens a disposable copy (plan 20).

The owner runs it on a copy of a real book. It must refuse Kassiber's own
data, read an encrypted copy's passphrase from a file descriptor only, and
leave nothing behind outside the copy. Each test points ``HOME`` (and
``XDG_DATA_HOME``) at a temporary directory, so "the default data root" is a
temporary directory too.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.db import DEFAULT_DATA_DIRNAME, default_state_root, open_db, resolve_database_path
from kassiber.projects import catalog_path, write_catalog

_ROOT = Path(__file__).resolve().parents[2]
NOW = "2024-01-01T00:00:00Z"


def _seed_book(conn) -> None:
    """One workspace with a generic and an Austrian profile, each with a few rows."""
    conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws-1', 'Main', ?)", (NOW,))
    for profile_id, label, country, method in (
        ("profile-generic", "Generic", "generic", "FIFO"),
        ("profile-at", "Austria", "at", "moving_average_at"),
    ):
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
                                 tax_long_term_days, gains_algorithm, created_at)
            VALUES(?, 'ws-1', ?, 'EUR', ?, 365, ?, ?)
            """,
            (profile_id, label, country, method, NOW),
        )
        account_id = f"acct-{profile_id}"
        conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
            VALUES(?, 'ws-1', ?, 'treasury', 'Treasury', 'asset', 'BTC', ?)
            """,
            (account_id, profile_id, NOW),
        )
        wallet_id = f"wallet-{profile_id}"
        conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
            VALUES(?, 'ws-1', ?, ?, 'Exchange', 'custom', '{}', ?)
            """,
            (wallet_id, profile_id, account_id, NOW),
        )
        for index, (when, direction, amount, fee, rate, kind) in enumerate(
            (
                ("2020-06-01T00:00:00Z", "inbound", 50_000_000_000, 0, 8000.0, "buy"),
                ("2022-03-01T00:00:00Z", "inbound", 20_000_000_000, 0, 40000.0, "buy"),
                ("2023-01-01T00:00:00Z", "outbound", 10_000_000_000, 1_000_000, 20000.0, "sell"),
            )
        ):
            tx_id = f"{profile_id}-tx-{index}"
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, direction, asset, amount, fee, fiat_currency,
                    fiat_rate, fiat_value, kind, raw_json, created_at
                ) VALUES(?, 'ws-1', ?, ?, ?, ?, ?, ?, 'BTC', ?, ?, 'EUR', ?, NULL, ?, '{}', ?)
                """,
                (tx_id, profile_id, wallet_id, f"exchange-{tx_id}", f"fp-{tx_id}", when, direction, amount, fee, rate, kind, when),
            )
    conn.commit()


class CompareBookTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="kassiber-compare-book-")
        self.addCleanup(scratch.cleanup)
        self.scratch = Path(scratch.name)
        self.home = self.scratch / "home"
        self.tmp = self.scratch / "tmp"
        for path in (self.home, self.tmp):
            path.mkdir()
        self.environ = {"XDG_DATA_HOME": str(self.home / "xdg")}
        self.default_root = default_state_root(environ=self.environ, home=self.home) / DEFAULT_DATA_DIRNAME

    def _run(self, *args: str, pass_fds: tuple[int, ...] = ()) -> subprocess.CompletedProcess:
        env = {**os.environ, **self.environ, "HOME": str(self.home), "TMPDIR": str(self.tmp)}
        return subprocess.run(
            [sys.executable, "-m", "tests.tax_oracle.compare_book", *args],
            cwd=_ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=300,
            pass_fds=pass_fds,
        )

    def _book(self, data_root: Path, *, passphrase: str | None = None) -> Path:
        conn = open_db(data_root, passphrase=passphrase)
        try:
            _seed_book(conn)
        finally:
            conn.close()
        return resolve_database_path(data_root)

    def _assert_refused(self, data_root: Path, reason: str) -> None:
        before = sorted((path.relative_to(self.scratch), path.read_bytes()) for path in self.scratch.rglob("*") if path.is_file())
        result = self._run("--data-root", str(data_root))
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn(reason, result.stderr)
        self.assertEqual(result.stdout, "")
        after = sorted((path.relative_to(self.scratch), path.read_bytes()) for path in self.scratch.rglob("*") if path.is_file())
        self.assertEqual(after, before, "a refused run must not open or create anything")

    def test_refuses_the_default_data_root(self):
        self._book(self.default_root)
        self._assert_refused(self.default_root, "default data root")
        link = self.scratch / "looks-like-a-copy"
        link.symlink_to(self.default_root, target_is_directory=True)
        self._assert_refused(link, "default data root")

    def test_refuses_projects_under_the_default_state_root(self):
        project = self.default_root.parent / "projects" / "savings" / DEFAULT_DATA_DIRNAME
        self._book(project)
        self._assert_refused(project, "inside Kassiber's default state root")

    def test_refuses_a_registered_project_outside_the_state_root(self):
        project = self.scratch / "Books" / "savings"
        data_root = project / DEFAULT_DATA_DIRNAME
        self._book(data_root)
        write_catalog(
            {"projects": [{"id": "savings", "name": "Savings", "path": str(project), "encrypted": False}]},
            catalog_path(self.default_root.parent),
        )
        self._assert_refused(data_root, "belongs to the registered project")

    def test_refuses_a_directory_without_a_database(self):
        empty = self.scratch / "empty-copy"
        empty.mkdir()
        self._assert_refused(empty, "contains no Kassiber database")

    def test_refuses_a_database_linked_to_another_book(self):
        live = self._book(self.scratch / "live")
        symlinked = self.scratch / "symlinked-copy"
        symlinked.mkdir()
        (symlinked / live.name).symlink_to(live)
        self._assert_refused(symlinked, "is a symbolic link")
        hard_linked = self.scratch / "hard-linked-copy"
        hard_linked.mkdir()
        os.link(live, hard_linked / live.name)
        self._assert_refused(hard_linked, "has other hard links")

    def test_refuses_a_linked_sqlite_side_file(self):
        copy = self.scratch / "copy-with-linked-wal"
        database = self._book(copy)
        live_wal = self.scratch / "live.sqlite3-wal"
        live_wal.write_bytes(b"")
        database.with_name(database.name + "-wal").symlink_to(live_wal)
        self._assert_refused(copy, "is a symbolic link")

    def _contents(self, data_root: Path) -> list[str]:
        conn = open_db(data_root, require_existing_schema=True)
        try:
            return list(conn.iterdump())
        finally:
            conn.close()

    def test_compares_every_profile_of_a_copy_and_leaves_nothing_behind(self):
        copy = self.scratch / "copy"
        self._book(copy)
        contents = self._contents(copy)
        result = self._run("--data-root", str(copy))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 2, result.stdout)
        self.assertTrue(lines[0].startswith("profile 'Austria' (profile-at): 3 projected rows"), lines[0])
        self.assertTrue(lines[1].startswith("profile 'Generic' (profile-generic): 3 projected rows"), lines[1])
        for line in lines:
            self.assertIn("production equals the oracle (", line)
            self.assertNotIn("both fail", line)
        self.assertEqual(list(self.home.rglob("*")), [])
        self.assertEqual(list(self.tmp.rglob("*")), [])
        self.assertEqual(self._contents(copy), contents)

        only = self._run("--data-root", str(copy), "--profile", "generic")
        self.assertEqual(only.returncode, 0, only.stdout + only.stderr)
        self.assertEqual(only.stdout.splitlines(), lines[1:])

    def _run_patched(self, patch: str, *args: str) -> subprocess.CompletedProcess:
        """``compare_book.main(args)`` after ``patch`` (Python source) runs in the child."""
        script = (
            "import dataclasses, sys\n"
            "from tests import tax_engine_compare as compare\n"
            "from tests.tax_oracle import compare_book\n"
            f"{patch}"
            "sys.exit(compare_book.main(sys.argv[1:]))\n"
        )
        env = {**os.environ, **self.environ, "HOME": str(self.home), "TMPDIR": str(self.tmp)}
        return subprocess.run(
            [sys.executable, "-c", script, *args],
            cwd=_ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=300,
        )

    def test_prints_each_difference_and_fails(self):
        copy = self.scratch / "copy"
        self._book(copy)
        # Production loses its last tax summary row, as a fold bug would.
        result = self._run_patched(
            "real = compare.run_production\n"
            "def run_production(profile, inputs):\n"
            "    result, error = real(profile, inputs)\n"
            "    return dataclasses.replace(result, tax_summary=result.tax_summary[:-1]), error\n"
            "compare.run_production = run_production\n",
            "--data-root",
            str(copy),
            "--profile",
            "Generic",
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("production differs from the oracle:", result.stdout)
        self.assertIn("  result['tax_summary']: length 1 != 0", result.stdout)

    def test_cannot_write_to_the_copy(self):
        copy = self.scratch / "copy"
        self._book(copy)
        contents = self._contents(copy)
        # Projection building that tried to store something.
        result = self._run_patched(
            "from kassiber.core.custody_journal import CustodyJournalBuilder\n"
            "real = CustodyJournalBuilder.build_custody_projection\n"
            "def build(self):\n"
            "    self.conn.execute(\"UPDATE profiles SET label = 'changed'\")\n"
            "    return real(self)\n"
            "CustodyJournalBuilder.build_custody_projection = build\n",
            "--data-root",
            str(copy),
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 2, result.stdout)
        for line in lines:
            self.assertIn("not compared, its custody projection does not build: attempt to write a readonly database", line)
        self.assertEqual(self._contents(copy), contents)

    def test_reads_an_encrypted_copy_only_from_the_passphrase_fd(self):
        copy = self.scratch / "encrypted-copy"
        self._book(copy, passphrase="disposable copy passphrase")
        locked = self._run("--data-root", str(copy))
        self.assertEqual(locked.returncode, 2, locked.stdout + locked.stderr)
        self.assertIn("database is encrypted", locked.stderr)

        read_fd, write_fd = os.pipe()
        self.addCleanup(os.close, read_fd)
        os.write(write_fd, b"disposable copy passphrase\n")
        os.close(write_fd)
        result = self._run("--data-root", str(copy), "--db-passphrase-fd", str(read_fd), pass_fds=(read_fd,))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count("production equals the oracle"), 2, result.stdout)
        self.assertNotIn("disposable copy passphrase", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

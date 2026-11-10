"""Shared differential harness for the main-sync slices (plan 21).

`DesktopParity` compares daemon sessions; this adds the CLI leaves on the same
book and seeds from a list of SQL statements. Both compare raw bytes and all
113 tables, plaintext or SQLCipher, masking only identities and clocks the
run itself generated (`Masker`).
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import re
import shutil

from tests import cli_runner
from tests.rust_desktop_parity import DesktopParity, run_candidate
from tests.test_rust_review_workflow import Masker


_ANY_UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
_HASH = re.compile(r"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")


class SyncMasker(Masker):
    """`Masker` for every UUID version: pair components are UUIDv5 of a
    fresh pair id, so they are generated identities too."""

    def __init__(self, since, known=(), hashes=False):
        super().__init__(since)
        self.known = set(known)
        self.hashes = hashes

    def _hashes(self, text):
        # Commitments over generated identities differ by construction; the
        # fixed-source pairing oracles pin them. Hashes the book had stay.
        if not self.hashes:
            return text
        return _HASH.sub(lambda m: m.group(0) if m.group(0) in self.known else "<hash>", text)

    def _blank(self, ids, text):
        return super()._blank(ids, _ANY_UUID.sub(
            lambda m: m.group(0) if m.group(0) in self.known else ids.get(m.group(0), "<uuid>"),
            self._hashes(text)))

    def _text(self, ids, text):
        def identity(match):
            if match.group(0) in self.known:
                return match.group(0)
            return ids.setdefault(match.group(0), f"<uuid:{len(ids)}>")
        return super()._text(ids, _ANY_UUID.sub(identity, self._hashes(text)))


class SyncParity(DesktopParity):
    SEED = None
    SQL: list[str] = []
    CONTEXT = ("ws", "profile")
    PASSPHRASE = None

    def setUp(self):
        self.started = datetime.now(timezone.utc) - timedelta(seconds=1)
        super().setUp()
        if self.PASSPHRASE:
            self.encrypt(self.PASSPHRASE)

    def seed(self):
        self.change(
            ";\n".join(
                [
                    *self.SQL,
                    "INSERT OR REPLACE INTO settings(key,value) VALUES"
                    "('context_workspace','%s'),('context_profile','%s')" % self.CONTEXT,
                ]
            )
            + ";"
        )

    def requests(self, args, kind=None):
        requests = super().requests(args, kind)
        if self.passphrase:
            # Without an unlock the daemon answers every kind auth_required.
            requests.insert(0, {
                "request_id": "unlock",
                "kind": "daemon.unlock",
                "args": {"auth_response": {"passphrase_secret": self.passphrase}},
            })
        return requests

    def compare(self, args, **options):
        responses = super().compare(args, **options)
        if self.passphrase:
            self.assertEqual(responses[0]["kind"], "daemon.unlock", responses[0])
            responses = responses[1:]
        return responses

    def masker(self, hashes=False):
        known = set()
        for rows in self.snapshot().values():
            for row in rows:
                for value in row:
                    if isinstance(value, str):
                        known.update(_ANY_UUID.findall(value))
                        known.update(_HASH.findall(value))
        return SyncMasker(self.started, known, hashes)

    def roots(self, text):
        """Each side's data root (a copy for Rust in mutating sessions)."""
        text = text.decode() if isinstance(text, bytes) else text
        return re.sub(re.escape(str(self.base)) + r'[^"]*', "<root>", text)

    def masked_read(self, args, *, kind):
        """A read-only session whose responses carry the run's own clock."""
        masker = self.masker()
        return self.compare(args, kind=kind, mask=lambda line: masker.text(self.roots(line)))

    def masked(self, args, *, kind):
        masker = self.masker()
        return self.compare(args, kind=kind, mutates=True,
                            mask=lambda line: masker.text(self.roots(line)), tables=masker.tables)

    def cli(self, commands, *, mutates=True, hashes=False):
        """Run each argv on the Python book and on a Rust copy, in order.

        Exit codes, stdout and stderr must match byte for byte after masking,
        and so must all 113 tables after the last command. Returns the parsed
        Python envelopes. ``hashes`` also masks 64-hex values the run created
        (commitments over generated identities).
        """
        candidate = self.base / f"cli-{len(list(self.base.glob('cli-*')))}" / "data"
        shutil.copytree(self.root, candidate)
        masker = self.masker(hashes)
        baseline = self.snapshot()
        envelopes = []
        for argv in commands:
            results = []
            for side, root in ((0, candidate), (1, self.root)):
                args = ["--machine", "--data-root", str(root)]
                if self.passphrase:
                    args.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
                args.extend(argv)
                if side == 0:
                    result = run_candidate(args, env=self.env, input=b"")
                else:
                    result = cli_runner.run(args, reference=True, env=self.env, input=b"",
                                            capture_output=True, timeout=300)
                results.append((result.returncode, masker.text(result.stdout, side),
                                masker.text(result.stderr, side)))
            self.assertEqual(results[0], results[1], argv)
            envelopes.append(json.loads(results[1][1]) if results[1][1] else None)
        self.assertEqual(masker.tables(self.snapshot(candidate), 0),
                         masker.tables(self.snapshot(), 1))
        if not mutates:
            self.assertEqual(self.snapshot(), baseline)
        return envelopes

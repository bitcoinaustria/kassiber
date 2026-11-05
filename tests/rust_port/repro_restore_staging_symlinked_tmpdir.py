"""Reproduce the reference refusing a restore staged under a symlinked TMPDIR.

    uv run --locked python -m tests.rust_port.repro_restore_staging_symlinked_tmpdir

`backup.pack` stages an import with `tempfile.mkdtemp()`, a lexical path
under `$TMPDIR`. On macOS that is `/var/folders/...` and `/var` links to
`/private/var`. `install._reject_links` then checks every lexical parent of
each staged source and refuses the restore with `unsafe_restore_target`,
although the link is above the staging directory and not Kassiber's.
Destinations are resolved first, so only staging is affected. The Rust store
keeps this refusal byte-identical (`restore::install` checks staged sources the
same way) until the reference changes. Scratch stays under target/.
"""
from pathlib import Path
import tempfile

from kassiber.backup.install import install_staged_backup
from kassiber.errors import AppError


def main():
    scratch = Path(__file__).resolve().parents[2] / "target/restore-staging-repro"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch.resolve()) as temporary:
        base = Path(temporary)
        (base / "private-var").mkdir()
        (base / "var").symlink_to("private-var")
        staging = base / "var" / "folders" / "kassiber-restore-x" / "stage"
        staging.mkdir(parents=True)
        (staging / "kassiber.sqlite3").write_bytes(b"synthetic staged database")
        data_root = base / "book" / "data"
        try:
            install_staged_backup(staging, data_root)
        except AppError as error:
            print(f"reproduced: {error.code}: {error}")
            assert error.code == "unsafe_restore_target"
            assert not (data_root / "kassiber.sqlite3").exists()
            return
        raise SystemExit("not reproduced: the reference installed the staged backup")


if __name__ == "__main__":
    main()

"""Check a restore staged under a symlinked TMPDIR installs.

    uv run --locked python -m tests.rust_port.repro_restore_staging_symlinked_tmpdir

`backup.pack` stages an import with `tempfile.mkdtemp()`, a lexical path
under `$TMPDIR`. On macOS that is `/var/folders/...` and `/var` links to
`/private/var`. `install._reject_links` used to check every lexical parent of
each staged source and refuse the restore with `unsafe_restore_target`,
although the link is above the staging directory and not Kassiber's. The
reference now resolves the parents of the staging directory first and keeps
refusing links at or below it. The Rust store (`restore::install`) still
refuses until it follows. Scratch stays under target/.
"""
from pathlib import Path
import tempfile

from kassiber.backup.install import install_staged_backup


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
        install_staged_backup(staging, data_root)
        installed = (data_root / "kassiber.sqlite3").read_bytes()
        assert installed == b"synthetic staged database", installed
        print("fixed: the staged backup installed under a symlinked temp root")


if __name__ == "__main__":
    main()

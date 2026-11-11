"""Exercise publisher argument handling using local AWS mocks only."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tests import test_linux_channel_renderers as fixtures


ROOT = Path(__file__).resolve().parents[2]
SHELLS = sorted({
    str(Path(path).resolve())
    for path in (
        "/bin/bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash",
        shutil.which("bash"),
    )
    if path and Path(path).is_file()
})


class PublisherShellTest(unittest.TestCase):
    def test_publisher_shell_arguments_and_failure_status(self):
        for shell in SHELLS:
            for endpoint in (None, "https://storage.invalid/custom endpoint"):
                for fail_at in (0, 1, 8):
                    with self.subTest(shell=shell, endpoint=endpoint, fail_at=fail_at), \
                            tempfile.TemporaryDirectory() as temp_dir:
                        check(Path(temp_dir), shell, endpoint, fail_at)


def check(tmp_path, shell, endpoint, fail_at):
    root = tmp_path / "repository with spaces"
    root.mkdir()
    helper = fixtures.RepositoryPublisherTest()
    apt, dnf = helper._repository_roots(root)
    helper._write_apt_metadata(apt)
    (dnf / "repodata/repomd.xml").write_text("metadata", encoding="utf-8")
    (dnf / "repodata/repomd.xml.asc").write_text("signature", encoding="utf-8")
    fake_bin = root / "bin"
    fake_bin.mkdir()
    aws = fake_bin / "aws"
    aws.write_text(
        f"#!{sys.executable}\n"
        "import json, os, pathlib, sys\n"
        "log = pathlib.Path(os.environ['KASSIBER_TEST_AWS_LOG'])\n"
        "calls = len(log.read_text().splitlines()) if log.exists() else 0\n"
        "with log.open('a') as stream:\n"
        "    stream.write(json.dumps(sys.argv[1:]) + '\\n')\n"
        "if calls + 1 == int(os.environ['KASSIBER_TEST_AWS_FAIL_AT']):\n"
        "    sys.exit(23)\n",
        encoding="utf-8",
    )
    aws.chmod(0o755)
    log = root / "aws.jsonl"
    environment = {
        **os.environ,
        "PATH": f"{fake_bin}:{os.environ['PATH']}",
        "KASSIBER_TEST_AWS_LOG": str(log),
        "KASSIBER_TEST_AWS_FAIL_AT": str(fail_at),
    }
    command = [
        shell, str(ROOT / "scripts/publish-linux-repositories-s3.sh"),
        "--apt", str(apt), "--dnf", str(dnf), "--suite", "prerelease",
        "--destination", "s3://example/kassiber",
        "--base-url", "https://packages.invalid",
    ]
    if endpoint:
        command.extend(["--endpoint", endpoint])
    result = subprocess.run(command, capture_output=True, text=True, env=environment)
    assert result.returncode == (23 if fail_at else 0), result.stderr
    assert log.exists(), result.stderr
    calls = [json.loads(line) for line in log.read_text().splitlines()]
    assert len(calls) == (fail_at or 9)
    prefix = ["--endpoint-url", endpoint] if endpoint else []
    for call in calls:
        assert call[:len(prefix)] == prefix
        assert call[len(prefix)] == "s3"
    assert calls[0] == [*prefix, "s3", "sync", str(apt / "pool"), "s3://example/kassiber/apt/pool"]
    if fail_at:
        assert "Published" not in result.stdout
        assert not any("s3://example/kassiber/dnf/prerelease/mirrorlist" in call for call in calls)
    else:
        assert "s3://example/kassiber/dnf/prerelease/mirrorlist" in calls[-1]
        assert "Published" in result.stdout


if __name__ == "__main__":
    unittest.main()

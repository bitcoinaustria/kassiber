"""Run the actual notarization handoff shell with local git/GitHub mocks only."""
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = yaml.safe_load((ROOT / ".github/workflows/notarize-macos.yml").read_text())
STEPS = {step.get("name"): step for step in WORKFLOW["jobs"]["notarize"]["steps"]}
COMMIT = "a" * 40
CANDIDATE = f"macos-candidate-{COMMIT}-123"


@pytest.fixture
def runner(tmp_path):
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    # The workflow installs Python 3.11 before these steps; mirror that runtime
    # instead of macOS's older system python3 (which lacks tomllib).
    (fake_bin / "python3").symlink_to(sys.executable)
    fake = f"#!{sys.executable}\n" + '''
import json, os, pathlib, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['MOCK_LOG'], 'a') as stream:
    stream.write(json.dumps([tool, *args]) + '\\n')
if tool == 'git':
    if args[0] == 'ls-remote':
        if os.environ.get('MOCK_TAG_FAILURE'): sys.exit(1)
        if os.environ.get('MOCK_TAG'): print('existing tag')
    elif args[0] == 'show': print('[project]\\nversion="1.2.3"')
    elif args[0] == 'rev-parse': print('a' * 40)
    elif args[0] == 'merge-base': sys.exit(1 if os.environ.get('MOCK_FOREIGN') else 0)
    else: raise AssertionError(args)
elif tool == 'gh':
    if args[:2] == ['release', 'view']:
        field = args[args.index('--json') + 1]
        if field == 'isDraft': print(os.environ.get('MOCK_DRAFT', 'true'))
        elif field == 'targetCommitish': print(os.environ.get('MOCK_TARGET', 'a' * 40))
        elif field == 'assets':
            if os.environ.get('MOCK_ASSET_FAILURE'): sys.exit(1)
            print('manifest.asc' if os.environ.get('MOCK_SIGNED') else '')
        else: raise AssertionError(args)
    elif args[:2] == ['release', 'download']:
        pathlib.Path('incoming/kassiber-macos-signing-input.dmg').write_text('signed input')
    elif args[:2] not in (['release', 'upload'], ['release', 'delete-asset']):
        raise AssertionError(args)
else: raise AssertionError(tool)
'''
    for tool in ("git", "gh"):
        executable = fake_bin / tool
        executable.write_text(fake)
        executable.chmod(0o755)
    environment = {**os.environ, "PATH": f"{fake_bin}:{os.environ['PATH']}",
        "TAG": CANDIDATE, "CANDIDATE": "true", "SOURCE_COMMIT": COMMIT,
        "GITHUB_OUTPUT": str(tmp_path / "outputs"), "GITHUB_STEP_SUMMARY": str(tmp_path / "summary"),
        "MOCK_LOG": str(tmp_path / "commands")}

    def run(name, **overrides):
        return subprocess.run(["/bin/bash", "-c", STEPS[name]["run"]], cwd=tmp_path,
                              env={**environment, **overrides}, text=True, capture_output=True)
    return tmp_path, run


def test_candidate_uses_source_version_without_a_git_tag(runner):
    root, run = runner
    result = run("Download draft and bind source commit")
    assert result.returncode == 0, result.stderr
    outputs = (root / "outputs").read_text()
    assert f"commit={COMMIT}" in outputs
    assert "version=1.2.3" in outputs
    assert f"candidate_id={CANDIDATE}" in outputs
    commands = (root / "commands").read_text()
    assert '"rev-parse"' not in commands
    assert '"merge-base", "--is-ancestor"' in commands


@pytest.mark.parametrize("override", [
    {"SOURCE_COMMIT": "short"}, {"TAG": "v1.2.3"}, {"MOCK_TARGET": "main"},
    {"MOCK_TARGET": "b" * 40}, {"MOCK_TAG": "1"}, {"MOCK_FOREIGN": "1"},
    {"MOCK_DRAFT": "false"}, {"MOCK_TAG_FAILURE": "1"},
])
def test_invalid_candidate_never_downloads_input(runner, override):
    root, run = runner
    assert run("Download draft and bind source commit", **override).returncode != 0
    assert not (root / "incoming/kassiber-macos-signing-input.dmg").exists()


def test_version_release_keeps_its_tag_path(runner):
    root, run = runner
    result = run("Download draft and bind source commit", CANDIDATE="false", SOURCE_COMMIT="", TAG="v1.2.3")
    assert result.returncode == 0, result.stderr
    assert f"commit={COMMIT}\nversion=1.2.3" in (root / "outputs").read_text()
    assert '"refs/tags/v1.2.3^{commit}"' in (root / "commands").read_text()


@pytest.mark.parametrize("override", [{}, {"MOCK_TARGET": "b" * 40}, {"MOCK_TAG": "1"}, {"MOCK_DRAFT": "false"}, {"MOCK_SIGNED": "1"}, {"MOCK_TAG_FAILURE": "1"}, {"MOCK_ASSET_FAILURE": "1"}])
def test_candidate_promotion_rechecks_draft_and_never_finalizes(runner, override):
    root, run = runner
    (root / "incoming").mkdir()
    (root / "incoming/kassiber-macos-signing-input.dmg").write_text("input")
    (root / "notarized").mkdir()
    for name in ("kassiber-macos-arm64.app.zip", "kassiber-macos-arm64.dmg", "kassiber-cli-macos-arm64.tar.gz"):
        (root / "notarized" / name).write_text("sealed")
    result = run("Update verified draft macOS artifacts",
                 COMMIT=COMMIT, CANDIDATE_ID=CANDIDATE, VERSION="1.2.3", **override)
    calls = [json.loads(line) for line in (root / "commands").read_text().splitlines()]
    mutations = [call for call in calls if call[:3] in (["gh", "release", "upload"], ["gh", "release", "delete-asset"])]
    if override:
        assert result.returncode != 0
        assert mutations == []
    else:
        assert result.returncode == 0, result.stderr
        assert len(mutations) == 2
        assert "remains unpublished" in (root / "summary").read_text()
        assert not list((root / "incoming").glob("*manifest*"))
    assert all(call[:3] != ["gh", "release", "edit"] for call in calls)


def test_workflow_forwards_candidate_mode_to_notary_and_verifier():
    body = STEPS["Notarize and verify sealed distributions"]["run"]
    assert body.count('--candidate-id "$CANDIDATE_ID"') == 2
    assert '--commit "$COMMIT" --version "$VERSION"' in body
    assert "finalize-signed-release" not in str(WORKFLOW)
    assert "gh release create" not in str(WORKFLOW)
    assert "gh release edit" not in str(WORKFLOW)

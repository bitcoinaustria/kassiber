"""Release guards run without Apple credentials or signing a real artifact."""
import argparse
import importlib.util
import json
import plistlib
import stat
import subprocess
import sys
import tarfile
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("macos_release", ROOT / "scripts/macos_release.py")
release = importlib.util.module_from_spec(spec)
sys.modules["macos_release"] = release
spec.loader.exec_module(release)
prepare_spec = importlib.util.spec_from_file_location("prepare_macos_release", ROOT / "scripts/prepare_macos_release.py")
prepare = importlib.util.module_from_spec(prepare_spec)
prepare_spec.loader.exec_module(prepare)


@pytest.mark.parametrize("name,mode", [
    ("../outside", stat.S_IFREG), ("/outside", stat.S_IFREG),
    ("Kassiber.app/../../outside", stat.S_IFREG),
    ("Kassiber.app/link", stat.S_IFLNK), ("Kassiber.app/socket", stat.S_IFSOCK),
    ("Other.app/file", stat.S_IFREG), ("Kassiber.app\\evil", stat.S_IFREG),
])
def test_reject_archive_before_platform_extraction(tmp_path, name, mode):
    archive = tmp_path / "input.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        member = zipfile.ZipInfo(name)
        member.external_attr = mode << 16
        zf.writestr(member, "bad")
    with patch.object(release, "run") as run:
        with pytest.raises(ValueError):
            release.extract_app(archive, tmp_path / "out")
        run.assert_not_called()


def test_digest_fails_closed(tmp_path):
    artifact = tmp_path / "file"
    artifact.write_bytes(b"release")
    release.check_digest(artifact, release.sha256(artifact))
    with pytest.raises(ValueError):
        release.check_digest(artifact, "0" * 64)


def test_signing_never_runs_in_ci(tmp_path, monkeypatch):
    monkeypatch.setenv("CI", "true")
    with patch.object(release, "run") as run:
        with pytest.raises(ValueError, match="locally"):
            release.sign(argparse.Namespace())
        run.assert_not_called()


def test_embedded_build_identity(tmp_path):
    app = tmp_path / "Kassiber.app"
    contents = app / "Contents"
    contents.mkdir(parents=True)
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleIdentifier": release.APP_ID,
                      "CFBundleShortVersionString": "1.2.3"}, stream)
    metadata = contents / "BUILD_INFO.json"
    metadata.write_text(json.dumps({"commit": "a" * 40, "version": "1.2.3", "channel": "release"}))
    release.validate_app(app, "a" * 40, "1.2.3")
    with pytest.raises(ValueError):
        release.validate_app(app, "b" * 40, "1.2.3")
    with pytest.raises(ValueError):
        release.validate_app(app, "a" * 12, "1.2.3")
    metadata.unlink()
    with pytest.raises(ValueError, match="provenance"):
        release.validate_app(app, "a" * 40, "1.2.3")


def candidate_source():
    return {"kind": "candidate", "candidate_id": "macos-candidate-" + "a" * 40 + "-123",
            "commit": "a" * 40, "version": "1.2.3", "build_run": "123",
            "build_attempt": 1, "unsigned_app_sha256": "d" * 64}


def test_candidate_requires_explicit_mode_and_never_satisfies_release_provenance():
    source = candidate_source()
    release.validate_source(source, "a" * 40, "1.2.3", candidate_id=source["candidate_id"], input_digest="d" * 64)
    with pytest.raises(ValueError):
        release.validate_source(source, "a" * 40, "1.2.3")
    tagged = {"tag": "v1.2.3", "commit": "a" * 40, "build_run": "123", "build_attempt": 1,
              "unsigned_app_sha256": "d" * 64}
    release.validate_source(tagged, "a" * 40, "1.2.3", input_digest="d" * 64)
    with pytest.raises(ValueError):
        release.validate_source(tagged, "a" * 40, "1.2.3", candidate_id=source["candidate_id"])


@pytest.mark.parametrize("field,value", [
    ("commit", "b" * 40), ("version", "1.2.4"), ("kind", "release"),
    ("candidate_id", "macos-candidate-" + "a" * 40 + "-124"),
    ("build_run", "124"), ("build_attempt", 0), ("build_attempt", True),
    ("unsigned_app_sha256", "bad"), ("tag", "v1.2.3"),
])
def test_candidate_provenance_mismatch_fails_closed(field, value):
    source = candidate_source()
    expected = source["candidate_id"]
    source[field] = value
    with pytest.raises(ValueError):
        release.validate_source(source, "a" * 40, "1.2.3", candidate_id=expected, input_digest="d" * 64)


@pytest.mark.parametrize("candidate_id", ["v1.2.3", "macos-candidate-short-123", "macos-candidate-" + "b" * 40 + "-123"])
def test_explicit_candidate_identity_must_match_source(candidate_id):
    with pytest.raises(ValueError):
        release.validate_source(candidate_source(), "a" * 40, "1.2.3", candidate_id=candidate_id)


def test_macho_inventory_rejects_symlinks(tmp_path):
    (tmp_path / "lib").write_bytes(bytes.fromhex("cffaedfe") + b"code")
    (tmp_path / "data").write_bytes(b"data")
    assert release.code_files(tmp_path) == [tmp_path / "lib"]
    (tmp_path / "link").symlink_to("lib")
    with pytest.raises(ValueError):
        release.code_files(tmp_path)


def linkage_output(path, slices):
    """The relevant real otool -arch all -l shape, including fat-binary headers."""
    lines = []
    for index, (install_id, dependencies, rpaths) in enumerate(slices):
        lines.append(f"{path} (architecture {'arm64' if index == 0 else 'x86_64'}):")
        commands = [("LC_SEGMENT_64", None)]
        if install_id is not None:
            commands.append(("LC_ID_DYLIB", install_id))
        commands.extend(("LC_LOAD_DYLIB", name) for name in dependencies)
        commands.extend(("LC_RPATH", name) for name in rpaths)
        for number, (command, name) in enumerate(commands):
            lines.extend([f"Load command {number}", f"          cmd {command}", "      cmdsize 64"])
            if name is not None:
                field = "path" if command == "LC_RPATH" else "name"
                lines.append(f"         {field} {name} (offset 24)")
    return "\n".join(lines) + "\n"


@pytest.fixture
def macho_linkage(tmp_path):
    app = tmp_path / "Kassiber.app"
    library = app / "Contents/Resources/sidecar/_internal/libsecp256k1_darwin_x86_64.dylib"
    library.parent.mkdir(parents=True)
    library.write_bytes(bytes.fromhex("cffaedfe"))
    state = {"slices": [("build/" + library.name, ["/usr/lib/libSystem.B.dylib"], ["@loader_path"])]}

    def fake_run(*command):
        if command[:4] == ("/usr/bin/otool", "-arch", "all", "-l"):
            assert command[4] == library
            return linkage_output(library, state["slices"])
        assert command[:2] == ("/usr/bin/install_name_tool", "-id")
        assert command[3] == library
        state["slices"] = [(command[2], deps, rpaths) for _identity, deps, rpaths in state["slices"]]
        return ""

    with patch.object(release, "run", side_effect=fake_run) as run:
        yield app, library, state, run


def test_normalize_embit_install_id_before_signing(macho_linkage):
    app, library, state, run = macho_linkage
    with pytest.raises(ValueError, match="install ID"):
        release.verify_linkage(app)
    release.prepare_linkage(app)
    release.verify_linkage(app)
    assert state["slices"][0][0] == "@rpath/" + library.name
    changes = [call for call in run.call_args_list if call.args[0] == "/usr/bin/install_name_tool"]
    assert len(changes) == 1


@pytest.mark.parametrize("kind,value", [
    ("dependency", "/opt/homebrew/opt/openssl/lib/libssl.dylib"),
    ("dependency", "build/libsecp.dylib"),
    ("dependency", "@rpath/../libsecp.dylib"),
    ("dependency", "/usr/lib/../../tmp/libsecp.dylib"),
    ("dependency", "@loader_path/../../../../../../outside.dylib"),
    ("rpath", "/private/tmp/build/lib"),
    ("rpath", "lib"),
    ("rpath", "@rpath/lib"),
    ("rpath", "@loader_path/../../../../../../outside"),
    ("rpath", "@executable_path/../../../outside"),
    ("rpath", "@loader_path"),  # Duplicate within one architecture.
])
def test_unsafe_linkage_is_rejected_before_any_mutation(macho_linkage, kind, value):
    app, _library, state, run = macho_linkage
    install_id, dependencies, rpaths = state["slices"][0]
    (dependencies if kind == "dependency" else rpaths).append(value)
    with pytest.raises(ValueError):
        release.prepare_linkage(app)
    assert not any(call.args[0] == "/usr/bin/install_name_tool" for call in run.call_args_list)


def test_universal_linkage_is_validated_per_architecture(macho_linkage):
    app, library, state, run = macho_linkage
    state["slices"] = [("@rpath/" + library.name, ["@rpath/libother.dylib"], ["@loader_path/.."]) for _ in range(2)]
    release.verify_linkage(app)  # Same rpath in two slices is not a duplicate.
    assert not any(call.args[0] == "/usr/bin/install_name_tool" for call in run.call_args_list)
    state["slices"][1][2].append("/usr/local/lib")
    with pytest.raises(ValueError):
        release.verify_linkage(app)


def test_ambiguous_universal_install_ids_fail_closed(macho_linkage):
    app, _library, state, run = macho_linkage
    state["slices"].append(("@rpath/different.dylib", [], []))
    with pytest.raises(ValueError):
        release.prepare_linkage(app)
    assert not any(call.args[0] == "/usr/bin/install_name_tool" for call in run.call_args_list)


@pytest.mark.parametrize("output", [
    "", "not Mach-O\n", "file:\nLoad command 0\n cmd LC_RPATH\n cmdsize 32\n",
    "file:\nLoad command 0\n cmd LC_DYLD_ENVIRONMENT\n cmdsize 32\n",
])
def test_unknown_or_incomplete_linkage_output_fails_closed(output):
    with patch.object(release, "run", return_value=output):
        with pytest.raises(ValueError):
            release.read_linkage(Path("library"))


@pytest.mark.parametrize("command", ["LC_LOAD_WEAK_DYLIB", "LC_REEXPORT_DYLIB",
                                     "LC_LOAD_UPWARD_DYLIB", "LC_LAZY_LOAD_DYLIB"])
def test_other_dependency_commands_are_not_ignored(command):
    output = linkage_output("library", [(None, ["/opt/local/lib/unsafe.dylib"], [])])
    with patch.object(release, "run", return_value=output.replace("LC_LOAD_DYLIB", command)):
        assert release.read_linkage(Path("library"))[0].dependencies == ("/opt/local/lib/unsafe.dylib",)


@pytest.mark.skipif(sys.platform != "darwin", reason="Apple load-command tools")
def test_real_universal_dylib_normalization_preserves_dependencies(tmp_path):
    app = tmp_path / "Kassiber.app"
    library = app / "Contents/Resources/libprobe.dylib"
    library.parent.mkdir(parents=True)
    source = tmp_path / "probe.c"
    source.write_text("int release_probe(void) { return 1; }\n")
    # No certificate/key: compile only disposable, trivial code. Exercise both
    # actual otool architecture sections and install_name_tool, not a mock format.
    subprocess.run(["/usr/bin/clang", "-dynamiclib", "-arch", "arm64", "-arch", "x86_64",
                    "-Wl,-headerpad_max_install_names", "-Wl,-install_name,build/libprobe.dylib",
                    "-Wl,-rpath,@loader_path", str(source), "-o", str(library)],
                   check=True, capture_output=True)
    original = release.read_linkage(library)
    assert len(original) == 2
    assert all(item.install_id == "build/libprobe.dylib" for item in original)
    with pytest.raises(ValueError, match="install ID"):
        release.verify_linkage(app)
    release.prepare_linkage(app)
    prepared = release.read_linkage(library)
    assert all(item.install_id == "@rpath/libprobe.dylib" for item in prepared)
    assert [(item.dependencies, item.rpaths) for item in prepared] == [
        (item.dependencies, item.rpaths) for item in original]
    digest = release.sha256(library)
    release.verify_linkage(app)
    assert release.sha256(library) == digest


@pytest.mark.parametrize("field,value", [
    ("conclusion", "failure"), ("event", "pull_request"),
    ("head_sha", "b" * 40), ("path", "attacker.yml"),
    ("head_repository", {"full_name": "attacker/kassiber"}),
])
def test_untrusted_build_never_reaches_signer(tmp_path, field, value):
    build = {"conclusion": "success", "event": "push", "head_sha": "a" * 40,
             "path": ".github/workflows/prerelease-binaries.yml",
             "head_repository": {"full_name": prepare.REPO}}
    build[field] = value
    with patch.object(prepare, "run", side_effect=[
        json.dumps({"isDraft": True, "assets": []}), json.dumps(build),
        json.dumps({"sha": "a" * 40}),
    ]), patch.object(prepare, "sign") as sign:
        with pytest.raises(ValueError):
            prepare.prepare(argparse.Namespace(tag="v1.2.3", run_id="1", work_dir=tmp_path / "work"))
        sign.assert_not_called()
        assert not (tmp_path / "work").exists()


@pytest.fixture
def release_preparation(tmp_path):
    args = argparse.Namespace(tag="v1.2.3", run_id="1", work_dir=tmp_path / "work",
                              identity="c" * 40, provisioning_profile=tmp_path / "profile",
                              submit=True)
    state = {"phase": "start", "change_after": None, "change": None}
    build = {"conclusion": "success", "event": "push", "head_sha": "a" * 40,
             "path": ".github/workflows/prerelease-binaries.yml", "run_attempt": 1,
             "head_repository": {"full_name": prepare.REPO}}

    def fake_run(*command):
        changed = state["phase"] == state["change_after"]
        if command[:3] == ("gh", "release", "view"):
            return json.dumps({"isDraft": not (changed and state["change"] == "published"),
                               "assets": [{"name": "manifest.txt.asc"}]
                               if changed and state["change"] == "signed" else []})
        if command[:2] == ("gh", "api"):
            if "/actions/runs/" in command[2]:
                return json.dumps(build)
            assert command[2] == f"repos/{prepare.REPO}/commits/{args.tag}"
            return json.dumps({"sha": "b" * 40 if changed and state["change"] == "tag" else "a" * 40})
        if command[:3] == ("gh", "release", "upload"):
            assert "--clobber" not in command
            state["phase"] = "upload"
            return ""
        assert command[:3] in (("gh", "run", "download"), ("gh", "workflow", "run"))
        return ""

    def fake_sign(_args):
        state["phase"] = "sign"

    with patch.object(prepare, "run", side_effect=fake_run) as run, \
            patch.object(prepare, "sign", side_effect=fake_sign) as sign, \
            patch.object(prepare, "sha256", return_value="d" * 64):
        yield args, state, run, sign


@pytest.mark.parametrize("change_after", ["sign", "upload"])
@pytest.mark.parametrize("change", ["published", "signed", "tag"])
def test_changed_submission_target_fails_closed(release_preparation, change_after, change):
    args, state, run, sign = release_preparation
    state.update(change_after=change_after, change=change)
    with pytest.raises(ValueError):
        prepare.prepare(args)
    sign.assert_called_once()
    uploads = [call for call in run.call_args_list if call.args[:3] == ("gh", "release", "upload")]
    assert len(uploads) == (1 if change_after == "upload" else 0)
    assert not any(call.args[:3] == ("gh", "workflow", "run") for call in run.call_args_list)


@pytest.mark.parametrize("change", ["published", "signed"])
def test_initial_release_guard_still_prevents_signing(release_preparation, change):
    args, state, run, sign = release_preparation
    state.update(change_after="start", change=change)
    with pytest.raises(ValueError, match="unsigned draft"):
        prepare.prepare(args)
    sign.assert_not_called()
    assert run.call_count == 1
    assert not args.work_dir.exists()


@pytest.mark.parametrize("submit", [False, True])
def test_unchanged_submission_target_preserves_local_and_submit_modes(release_preparation, submit):
    args, _state, run, sign = release_preparation
    args.submit = submit
    prepare.prepare(args)
    sign.assert_called_once()
    uploads = [call for call in run.call_args_list if call.args[:3] == ("gh", "release", "upload")]
    dispatches = [call for call in run.call_args_list if call.args[:3] == ("gh", "workflow", "run")]
    assert len(uploads) == len(dispatches) == int(submit)
    draft_reads = [call for call in run.call_args_list if call.args[:3] == ("gh", "release", "view")]
    assert len(draft_reads) == (3 if submit else 1)


def test_workflows_keep_keys_local_and_publication_gated():
    workflows = ROOT / ".github/workflows"
    notary = (workflows / "notarize-macos.yml").read_text()
    final = (workflows / "finalize-signed-release.yml").read_text()
    build = yaml.safe_load((workflows / "prerelease-binaries.yml").read_text())
    publish_steps = build["jobs"]["publish"]["steps"]
    assert next(s for s in publish_steps if s.get("name") == "Create or update release")["with"]["draft"] is True
    assert "scripts/macos_release.py verify" in final
    assert 'case "$actual_draft" in true|false)' in final
    assert "if: steps.release.outputs.draft == 'true'" in final
    assert "--smoke" in final
    assert final.index("Require unchanged release assets and tag") < final.index("Publish the verified draft")
    assert final.index("Publish the verified draft") < final.index("Commit Homebrew tap update")
    assert final.index("Authenticate the complete release set") < final.index("Verify actual macOS release bytes")
    assert "APPLE_CERTIFICATE" not in notary
    assert "NOTARY_KEY_P8" in notary
    assert "--keychain" in notary
    assert "--identity" not in notary
    assert "release-seal-${{ inputs.tag_name }}" in notary and "release-seal-${{ inputs.tag_name }}" in final


@pytest.mark.skipif(sys.platform != "darwin", reason="Apple tool contract")
def test_real_codesign_rejects_non_bitcoin_austria_identity():
    # Apple's system binary is validly signed, but not by our Developer ID.
    # This exercises the actual platform requirement without any private key.
    with pytest.raises(subprocess.CalledProcessError) as error:
        release.verify_code(Path("/usr/bin/true"))
    assert "code failed to satisfy specified code requirement(s)" in error.value.stderr


@pytest.mark.parametrize("authorized", [False, True])
def test_certificate_extraction_checks_profile_membership(authorized):
    leaf = b"synthetic certificate bytes"
    def extract(*command):
        assert command[:2] == ("/usr/bin/codesign", "-d")
        assert len(command) == 4
        assert str(command[2]).startswith("--extract-certificates=")
        prefix = Path(str(command[2]).split("=", 1)[1])
        prefix.with_name(prefix.name + "0").write_bytes(leaf)
        return ""
    profile = {"DeveloperCertificates": [leaf if authorized else b"another certificate"]}
    with patch.object(release, "run", side_effect=extract):
        if authorized:
            release.verify_profile_certificate(Path("sealed.app"), profile)
        else:
            with pytest.raises(ValueError, match="not authorized"):
                release.verify_profile_certificate(Path("sealed.app"), profile)


@pytest.mark.skipif(sys.platform != "darwin", reason="Apple certificate extraction")
def test_real_codesign_accepts_attached_extraction_prefix(tmp_path):
    # No keys or signing: exercise Apple's parser against signed system code.
    # Some OS versions omit the public cert chain, so only check the CLI result.
    prefix = tmp_path / "public-cert"
    subprocess.run(["/usr/bin/codesign", "-d", f"--extract-certificates={prefix}", "/usr/bin/true"],
                   check=True, capture_output=True)


@pytest.mark.parametrize("mutation", [None, "bytes", "mode", "launcher", "missing", "extra"])
def test_cli_distribution_is_exact_sealed_app(tmp_path, mutation):
    root = tmp_path / "kassiber-cli-macos-arm64"
    app = root / "Kassiber.app"
    contents = app / "Contents/Resources/bin"
    contents.mkdir(parents=True)
    binary = contents / "kassiber"
    binary.write_bytes(b"sealed launcher")
    binary.chmod(0o755)
    (root / "kassiber").symlink_to("Kassiber.app/Contents/Resources/bin/kassiber")
    if mutation == "launcher":
        (root / "kassiber").unlink()
        (root / "kassiber").symlink_to("/etc/passwd")
    if mutation == "missing":
        (root / "kassiber").unlink()
    if mutation == "extra":
        (root / "extra").write_text("unexpected")
    archive = tmp_path / "cli.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(root, arcname=root.name)
    # Change the comparison side after creating the archive.
    if mutation == "bytes":
        binary.write_bytes(b"forged launcher")
    if mutation == "mode":
        binary.chmod(0o644)
    if mutation is None:
        release.verify_cli_archive(archive, app)
    else:
        with pytest.raises(ValueError):
            release.verify_cli_archive(archive, app)


@pytest.mark.parametrize("mutation", [None, "team", "expiry", "development", "app", "group"])
def test_developer_id_profile_contract(mutation):
    data = {"TeamIdentifier": [release.TEAM], "ProvisionsAllDevices": True,
            "ExpirationDate": datetime.now(timezone.utc) + timedelta(days=10),
            "Entitlements": dict(release.APP_ENTITLEMENTS)}
    if mutation == "team":
        data["TeamIdentifier"] = ["OTHERTEAM1"]
    if mutation == "expiry":
        data["ExpirationDate"] = datetime.now(timezone.utc) - timedelta(days=1)
    if mutation == "development":
        data["ProvisionsAllDevices"] = False
    if mutation == "app":
        data["Entitlements"]["com.apple.application-identifier"] = release.TEAM + ".other"
    if mutation == "group":
        data["Entitlements"]["keychain-access-groups"] = [release.TEAM + ".other"]
    if mutation is None:
        release.validate_profile_data(data)
    else:
        with pytest.raises(ValueError):
            release.validate_profile_data(data)


def test_entitlements_are_explicit_not_inherited():
    with patch.object(release, "run", return_value=plistlib.dumps(release.APP_ENTITLEMENTS).decode()):
        release.verify_entitlements(Path("app"), release.APP_ENTITLEMENTS)
        with pytest.raises(ValueError):
            release.verify_entitlements(Path("sidecar"), {})
    for unexpected in ({}, {**release.APP_ENTITLEMENTS, "com.apple.security.get-task-allow": True}):
        with patch.object(release, "run", return_value=plistlib.dumps(unexpected).decode()):
            with pytest.raises(ValueError):
                release.verify_entitlements(Path("app"), release.APP_ENTITLEMENTS)

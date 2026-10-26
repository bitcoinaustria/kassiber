"""Cross-check portable identity serialization against today's Python implementation."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch
from kassiber.operator.build import build_identity, _frozen_location
from kassiber.operator.native_auth import _CodeIdentity

binary = Path(__file__).resolve().parent / "target/debug/r0-macos-identity"
with tempfile.TemporaryDirectory(prefix="kassiber-r0-identity-") as tmp:
    executable = str(Path(tmp, "kassiber-cli-aarch64-apple-darwin").resolve())
    Path(executable).touch()
    cdhash = "ab" * 20
    out = json.loads(subprocess.check_output([str(binary), executable, cdhash]))
    info = {"version": "0.22.78", "channel": "prerelease", "commit": "8709d4304c3cd7b76396020947ac4b6bd66e7ebe"}
    with patch.object(sys, "frozen", True, create=True), patch.object(sys, "executable", executable), patch("kassiber.operator.build.packaged_build_info", return_value=info):
        assert out["build"] == build_identity()
    helper = _CodeIdentity("at.bitcoinaustria.kassiber", "6Q4R2C3GJK", cdhash)
    assert out["helper_opaque"] == helper.opaque
    assert out["helper_requirement"] == helper.requirement
    mount_exe=Path(tmp,"mount/binaries/kassiber")
    mount_exe.parent.mkdir(parents=True)
    mount_exe.touch()
    Path(tmp,"Kassiber.AppImage").touch()
    with patch.object(sys, "frozen", True, create=True), patch.object(sys, "executable", str(mount_exe)), patch.dict(os.environ, {"APPDIR": tmp+"/mount", "APPIMAGE": tmp+"/Kassiber.AppImage"}), patch("kassiber.operator.build.packaged_build_info", return_value=info):
        image_out=json.loads(subprocess.check_output([str(binary),str(mount_exe),cdhash]))
        assert image_out["build"]==build_identity()
        assert _frozen_location() == tmp+"/Kassiber.AppImage"
        with patch.object(sys,"executable",executable):
            outside=json.loads(subprocess.check_output([str(binary),executable,cdhash]))
            assert outside["build"]==build_identity()
    print("PASS: build identity, helper opaque hash, Developer ID requirement, AppImage stable origin")
    print("macOS signing/Keychain/LocalAuthentication execution: NOT RUN (Linux x86_64)")

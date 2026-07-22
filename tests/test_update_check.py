from __future__ import annotations

import io
import json
import os
import subprocess
from argparse import Namespace
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

from kassiber import update_check
from kassiber.cli.main import main


_SEMVER_CASES = json.loads(
    (Path(__file__).parent / "fixtures" / "update_semver_cases.json").read_text(
        encoding="utf-8"
    )
)


class _Response(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, traceback):
        self.close()


class _Tty(io.StringIO):
    def isatty(self) -> bool:
        return True


def _release_response(version: str = "0.22.56") -> bytes:
    return json.dumps(
        [
            {
                "tag_name": f"v{version}",
                "draft": False,
                "prerelease": True,
            }
        ]
    ).encode()


def test_semver_comparison_handles_prereleases_and_invalid_values():
    for case in _SEMVER_CASES["comparisons"]:
        assert (
            update_check.is_newer_version(case["latest"], case["current"])
            is case["newer"]
        )
    for value in _SEMVER_CASES["invalid"]:
        assert update_check.parse_version(value) is None


def test_release_selection_uses_highest_semver_not_response_order():
    payload = json.loads(_release_response("0.22.55"))
    payload.extend(
        [
            {"tag_name": "v0.23.0-rc.1", "draft": False, "prerelease": True},
            {"tag_name": "nightly", "draft": False, "prerelease": True},
            {"tag_name": "v0.24.0", "draft": True, "prerelease": False},
            {"tag_name": "v0.23.0", "draft": False, "prerelease": False},
        ]
    )

    selected = update_check._release_from_response(payload)

    assert selected["latest_version"] == "0.23.0"
    assert selected["release_tag"] == "v0.23.0"
    assert selected["prerelease"] is False


def test_stable_channel_does_not_advertise_prereleases():
    payload = [
        {"tag_name": "v1.1.0-rc.1", "draft": False, "prerelease": True},
        {"tag_name": "v1.0.1", "draft": False, "prerelease": False},
    ]

    selected = update_check._release_from_response(payload, channel="release")

    assert selected["latest_version"] == "1.0.1"
    assert selected["prerelease"] is False


def test_default_redirect_handler_refuses_cross_origin_redirects():
    handler = update_check._NoRedirectHandler()
    assert (
        handler.redirect_request(
            None,
            None,
            302,
            "Found",
            {},
            "https://example.com/releases",
        )
        is None
    )


def test_fetch_latest_release_uses_bounded_github_request():
    captured = {}

    def opener(request, *, timeout):
        captured["url"] = request.full_url
        captured["headers"] = dict(request.header_items())
        captured["timeout"] = timeout
        return _Response(_release_response())

    with patch("kassiber.update_check.packaged_build_info", return_value={}):
        release = update_check.fetch_latest_release(opener=opener)

    assert captured["url"] == update_check.GITHUB_RELEASES_API_URL
    assert captured["timeout"] == update_check.NETWORK_TIMEOUT_SECONDS
    assert captured["headers"]["User-agent"].startswith("kassiber/")
    assert release == {
        "latest_version": "0.22.56",
        "release_tag": "v0.22.56",
        "release_url": (
            "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56"
        ),
        "prerelease": True,
    }


def test_check_writes_public_cache_and_recomputes_current_version(tmp_path: Path):
    destination = tmp_path / "update-check.json"
    checked_at = datetime(2026, 7, 22, 8, 30, tzinfo=timezone.utc)

    def opener(request, *, timeout):
        del request, timeout
        return _Response(_release_response())

    with (
        patch(
            "kassiber.update_check.packaged_build_info",
            return_value={"version": "0.22.55"},
        ),
        patch(
            "kassiber.update_check.detect_install_method",
            return_value="homebrew_cask",
        ),
    ):
        result = update_check.check_for_update(
            path=destination,
            opener=opener,
            now=checked_at,
        )
        cached = update_check.read_cache(destination)

    assert result["update_available"] is True
    assert result["update_command"] == update_check.HOMEBREW_CASK_COMMAND
    assert cached == result
    on_disk = json.loads(destination.read_text(encoding="utf-8"))
    assert "install_method" not in on_disk
    assert "current_version" not in on_disk
    assert destination.stat().st_mode & 0o077 == 0


def test_install_method_proves_formula_or_cask_before_suggesting_brew(
    tmp_path: Path,
):
    assert (
        update_check.detect_install_method(
            executable="/opt/homebrew/Cellar/kassiber-cli/0.22.55/bin/kassiber",
            argv0="kassiber",
            environ={},
        )
        == "homebrew_formula"
    )
    assert (
        update_check.detect_install_method(
            executable="/Applications/Kassiber.app/Contents/MacOS/kassiber",
            argv0="kassiber",
            environ={update_check.HOMEBREW_PACKAGE_ENV: "cask"},
        )
        == "homebrew_cask"
    )
    assert (
        update_check.detect_install_method(
            executable="/Applications/Kassiber.app/Contents/MacOS/kassiber",
            argv0="kassiber",
            environ={},
            install_context_path=tmp_path / "missing-install-context.json",
        )
        == "manual"
    )
    formula = tmp_path / "Cellar" / "kassiber-cli" / "0.22.55" / "bin" / "kassiber"
    formula.parent.mkdir(parents=True)
    formula.touch()
    if os.name == "nt":
        return
    linked_formula = tmp_path / "bin" / "kassiber"
    linked_formula.parent.mkdir()
    linked_formula.symlink_to(formula)
    assert (
        update_check.detect_install_method(
            executable=str(linked_formula),
            argv0="kassiber",
            environ={},
        )
        == "homebrew_formula"
    )


def test_linux_package_marker_requires_exact_package_ownership_and_stays_manual(
    tmp_path: Path,
):
    marker = tmp_path / "install-context.json"
    marker.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "product": "kassiber",
                "surface": "cli",
                "artifact_kind": "deb",
                "package_name": "kassiber-cli",
                "package_manager": "dpkg",
                "repository_manager": "apt",
                "repository_provenance": "probe-required",
                "executables": ["/usr/bin/kassiber"],
            }
        ),
        encoding="utf-8",
    )
    captured = {}

    def owned_runner(command, **kwargs):
        captured["command"] = command
        captured["environment"] = kwargs["env"]
        return subprocess.CompletedProcess(
            command,
            0,
            stdout=f"kassiber-cli: {marker}\n",
        )

    with patch("kassiber.update_check.shutil.which", return_value="/usr/bin/dpkg-query"):
        method = update_check.detect_install_method(
            executable="/usr/bin/kassiber",
            argv0="kassiber",
            environ={"PATH": "/usr/bin", "OPENAI_API_KEY": "do-not-forward"},
            install_context_path=marker,
            runner=owned_runner,
        )

    assert method == "linux_deb_manual"
    assert update_check.update_command_for_method(method) is None
    assert captured["command"] == ["/usr/bin/dpkg-query", "-S", str(marker)]
    assert captured["environment"] == {"PATH": "/usr/bin"}

    def wrong_owner_runner(command, **kwargs):
        del kwargs
        return subprocess.CompletedProcess(command, 0, stdout=f"other: {marker}\n")

    with patch("kassiber.update_check.shutil.which", return_value="/usr/bin/dpkg-query"):
        assert (
            update_check.detect_install_method(
                executable="/usr/bin/kassiber",
                argv0="kassiber",
                environ={"PATH": "/usr/bin"},
                install_context_path=marker,
                runner=wrong_owner_runner,
            )
            == "manual"
        )


def test_notice_colors_only_human_terminal_content():
    result = {
        "current_version": "0.22.55",
        "latest_version": "0.22.56",
        "update_available": True,
        "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
        "update_command": update_check.HOMEBREW_FORMULA_COMMAND,
    }
    colored = update_check.render_update_status(result, color=True)
    plain = update_check.render_update_status(result, color=False)

    assert "\033[1;36m" in colored
    assert update_check.HOMEBREW_FORMULA_COMMAND in colored
    assert "\033[" not in plain
    assert "0.22.55 → 0.22.56" in plain


def test_automatic_check_uses_cache_and_refreshes_without_touching_machine_output(
    tmp_path: Path,
):
    destination = tmp_path / "update-check.json"
    old = datetime.now(timezone.utc) - timedelta(days=2)
    cached = {
        "current_version": "0.22.55",
        "latest_version": "0.22.56",
        "update_available": True,
        "prerelease": True,
        "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
        "checked_at": old.isoformat().replace("+00:00", "Z"),
        "install_method": "manual",
        "update_command": None,
    }
    update_check.write_cache(cached, destination)
    args = Namespace(
        machine=False,
        non_interactive=False,
        output=None,
        command="status",
    )
    stream = _Tty()

    with (
        patch(
            "kassiber.update_check.packaged_build_info",
            return_value={"version": "0.22.55"},
        ),
        patch("kassiber.update_check.start_background_refresh") as refresh,
    ):
        update_check.show_cached_update_and_refresh(
            args,
            path=destination,
            stream=stream,
            stdout=_Tty(),
        )

    assert "Update available" in stream.getvalue()
    refresh.assert_called_once_with(destination)

    machine = Namespace(**{**vars(args), "machine": True})
    machine_stream = _Tty()
    with (
        patch(
            "kassiber.update_check.packaged_build_info",
            return_value={"version": "0.22.55"},
        ),
        patch("kassiber.update_check.start_background_refresh") as refresh,
    ):
        update_check.show_cached_update_and_refresh(
            machine,
            path=destination,
            stream=machine_stream,
            stdout=_Tty(),
        )
    assert machine_stream.getvalue() == ""
    refresh.assert_not_called()

    structured = Namespace(**{**vars(args), "format": "json"})
    with patch(
        "kassiber.update_check.packaged_build_info",
        return_value={"version": "0.22.55"},
    ):
        assert not update_check.automatic_check_allowed(
            structured,
            stream=_Tty(),
            stdout=_Tty(),
        )


def test_failed_automatic_checks_are_throttled(tmp_path: Path):
    destination = tmp_path / "update-check.json"
    attempted_at = datetime(2026, 7, 22, 8, 30, tzinfo=timezone.utc)
    update_check._write_refresh_attempt(destination, now=attempted_at)

    assert not update_check.automatic_refresh_due(
        None,
        path=destination,
        now=attempted_at + timedelta(minutes=59),
    )
    assert update_check.automatic_refresh_due(
        None,
        path=destination,
        now=attempted_at + timedelta(hours=1, seconds=1),
    )
    assert update_check._refresh_attempt_path(destination).stat().st_mode & 0o077 == 0


def test_background_refresh_passes_only_required_environment(tmp_path: Path):
    destination = tmp_path / "update-check.json"
    with (
        patch.dict(
            os.environ,
            {
                "PATH": "/usr/bin",
                "HTTPS_PROXY": "http://proxy.example:8080",
                "OPENAI_API_KEY": "must-not-reach-child",
            },
            clear=True,
        ),
        patch("kassiber.update_check.subprocess.Popen") as popen,
    ):
        update_check.start_background_refresh(destination)

    environment = popen.call_args.kwargs["env"]
    assert environment["PATH"] == "/usr/bin"
    assert environment["HTTPS_PROXY"] == "http://proxy.example:8080"
    assert environment[update_check.UPDATE_CACHE_ENV] == str(destination)
    assert "OPENAI_API_KEY" not in environment
    assert update_check._read_refresh_attempt(destination) is not None


def test_no_color_disables_ansi_not_the_human_update_check():
    args = Namespace(
        machine=False,
        non_interactive=False,
        output=None,
        command="status",
    )
    stream = _Tty()
    with (
        patch.dict("os.environ", {"NO_COLOR": "1"}, clear=True),
        patch(
            "kassiber.update_check.packaged_build_info",
            return_value={"version": "0.22.55"},
        ),
    ):
        assert update_check.automatic_check_allowed(
            args,
            stream=stream,
            stdout=_Tty(),
        )
        assert not update_check.supports_color(stream)


def test_download_verification_never_triggers_the_network_update_checker():
    stream = _Tty()
    args = Namespace(
        command="verify-download",
        machine=False,
        non_interactive=False,
        output=None,
        format="table",
    )
    with patch(
        "kassiber.update_check.packaged_build_info",
        return_value={"channel": "release", "version": "1.0.0"},
    ):
        assert not update_check.automatic_check_allowed(
            args,
            stream=stream,
            stdout=stream,
        )


def test_machine_update_command_returns_clean_structured_information():
    result = {
        "current_version": "0.22.55",
        "latest_version": "0.22.56",
        "update_available": True,
        "prerelease": True,
        "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
        "checked_at": "2026-07-22T08:30:00Z",
        "install_method": "homebrew_formula",
        "update_command": update_check.HOMEBREW_FORMULA_COMMAND,
    }
    stdout = io.StringIO()
    stderr = io.StringIO()
    with (
        patch("kassiber.cli.main.check_for_update", return_value=result),
        patch("kassiber.cli.main._configure_cli_logging"),
        patch("sys.stdout", stdout),
        patch("sys.stderr", stderr),
    ):
        exit_code = main(["--machine", "update"])

    assert exit_code == 0
    payload = json.loads(stdout.getvalue())
    assert payload["kind"] == "update"
    assert payload["data"] == result
    assert stderr.getvalue() == ""

from __future__ import annotations

import io
import json
import os
import subprocess
import threading
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


def _enabled_preference(tmp_path: Path) -> Path:
    preference = tmp_path / "update-checks.json"
    update_check.set_update_checks_enabled(True, preference)
    return preference


def test_update_check_consent_is_explicit_owner_only_and_fail_closed(
    tmp_path: Path,
):
    preference = tmp_path / "config" / "update-checks.json"
    assert not update_check.update_checks_enabled(preference)

    update_check.set_update_checks_enabled(True, preference)
    assert update_check.update_checks_enabled(preference)
    assert preference.stat().st_mode & 0o077 == 0
    lock_path = update_check.preference_lock_path(preference)
    assert lock_path.is_file()
    if os.name != "nt":
        assert lock_path.stat().st_mode & 0o077 == 0

    update_check.set_update_checks_enabled(False, preference)
    assert not update_check.update_checks_enabled(preference)
    preference.write_text("not-json\n", encoding="utf-8")
    assert not update_check.update_checks_enabled(preference)
    if os.name != "nt":
        preference.unlink()
        target = tmp_path / "enabled-target.json"
        update_check.set_update_checks_enabled(True, target)
        preference.symlink_to(target)
        assert not update_check.update_checks_enabled(preference)


def test_update_check_consent_rejects_boolean_and_float_schema_versions(
    tmp_path: Path,
):
    preference = tmp_path / "update-checks.json"
    for schema_version in (True, 1.0):
        preference.write_text(
            json.dumps({"schema_version": schema_version, "enabled": True}),
            encoding="utf-8",
        )
        assert not update_check.update_checks_enabled(preference)


def test_update_check_lock_refuses_symlinks_before_network(tmp_path: Path):
    if os.name == "nt":
        return
    preference = _enabled_preference(tmp_path)
    lock_path = update_check.preference_lock_path(preference)
    lock_path.unlink()
    target = tmp_path / "lock-target"
    target.touch()
    lock_path.symlink_to(target)
    opened = False

    try:
        update_check.set_update_checks_enabled(False, preference)
    except update_check.AppError as error:
        assert error.code == "update_check_lock_failed"
    else:
        raise AssertionError("preference write followed a symlinked lock")
    assert target.read_bytes() == b""
    assert update_check.update_checks_enabled(preference)

    def opener(request, *, timeout):
        del request, timeout
        nonlocal opened
        opened = True
        return _Response(_release_response())

    try:
        update_check.check_for_update(preference=preference, opener=opener)
    except update_check.AppError as error:
        assert error.code == "update_check_lock_failed"
    else:
        raise AssertionError("symlinked update-check lock unexpectedly succeeded")
    assert opened is False


def test_disabling_waits_for_inflight_check_and_blocks_later_network(
    tmp_path: Path,
):
    preference = _enabled_preference(tmp_path)
    cache = tmp_path / "update-check.json"
    request_started = threading.Event()
    release_request = threading.Event()
    disable_started = threading.Event()
    disable_returned = threading.Event()
    check_errors: list[Exception] = []
    disable_errors: list[Exception] = []
    opener_calls = 0

    def opener(request, *, timeout):
        del request, timeout
        nonlocal opener_calls
        opener_calls += 1
        request_started.set()
        assert release_request.wait(2)
        return _Response(_release_response())

    def run_check():
        try:
            update_check.check_for_update(
                path=cache,
                preference=preference,
                opener=opener,
            )
        except Exception as exc:
            check_errors.append(exc)

    def disable():
        disable_started.set()
        try:
            update_check.set_update_checks_enabled(False, preference)
        except Exception as exc:
            disable_errors.append(exc)
        finally:
            disable_returned.set()

    check_thread = threading.Thread(target=run_check, daemon=True)
    check_thread.start()
    assert request_started.wait(2)
    disable_thread = threading.Thread(target=disable, daemon=True)
    disable_thread.start()
    assert disable_started.wait(2)
    returned_while_request_was_inflight = disable_returned.wait(0.1)
    release_request.set()
    check_thread.join(2)
    disable_thread.join(2)

    assert not returned_while_request_was_inflight
    assert not check_thread.is_alive()
    assert not disable_thread.is_alive()
    assert check_errors == []
    assert disable_errors == []
    assert not update_check.update_checks_enabled(preference)
    assert opener_calls == 1

    calls_before_disabled_check = opener_calls
    try:
        update_check.check_for_update(
            path=cache,
            preference=preference,
            opener=opener,
        )
    except update_check.AppError as error:
        assert error.code == "update_checks_disabled"
    else:
        raise AssertionError("disabled update check unexpectedly succeeded")
    assert opener_calls == calls_before_disabled_check


def test_environment_disable_overrides_persisted_consent(tmp_path: Path):
    preference = _enabled_preference(tmp_path)
    assert not update_check.update_checks_enabled(
        preference,
        environ={update_check.DISABLE_UPDATE_CHECK_ENV: "true"},
    )


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


def test_fetch_latest_stable_release_uses_latest_object_endpoint():
    captured = {}

    def opener(request, *, timeout):
        captured["url"] = request.full_url
        captured["timeout"] = timeout
        return _Response(
            json.dumps(
                {
                    "tag_name": "v1.0.1",
                    "draft": False,
                    "prerelease": False,
                }
            ).encode()
        )

    with patch(
        "kassiber.update_check.packaged_build_info",
        return_value={"channel": "release", "version": "1.0.0"},
    ):
        release = update_check.fetch_latest_release(opener=opener)

    assert captured == {
        "url": update_check.GITHUB_LATEST_RELEASE_API_URL,
        "timeout": update_check.NETWORK_TIMEOUT_SECONDS,
    }
    assert release == {
        "latest_version": "1.0.1",
        "release_tag": "v1.0.1",
        "release_url": (
            "https://github.com/bitcoinaustria/kassiber/releases/tag/v1.0.1"
        ),
        "prerelease": False,
    }


def test_cache_rejects_boolean_and_float_schema_versions(tmp_path: Path):
    cache = tmp_path / "update-check.json"
    for schema_version in (True, 1.0):
        cache.write_text(
            json.dumps(
                {
                    "schema_version": schema_version,
                    "latest_version": "0.22.56",
                    "prerelease": True,
                    "release_url": (
                        "https://github.com/bitcoinaustria/kassiber/"
                        "releases/tag/v0.22.56"
                    ),
                    "checked_at": "2026-07-22T08:30:00Z",
                }
            ),
            encoding="utf-8",
        )
        assert update_check.read_cache(cache) is None


def test_check_writes_public_cache_and_recomputes_current_version(tmp_path: Path):
    destination = tmp_path / "update-check.json"
    preference = _enabled_preference(tmp_path)
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
            preference=preference,
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
    preference = _enabled_preference(tmp_path)
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
            preference=preference,
            stream=stream,
            stdout=_Tty(),
        )

    assert "Update available" in stream.getvalue()
    refresh.assert_called_once_with(destination, preference)

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
            preference=preference,
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
            preference=preference,
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
    preference = _enabled_preference(tmp_path)
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
        update_check.start_background_refresh(destination, preference)

    environment = popen.call_args.kwargs["env"]
    assert environment["PATH"] == "/usr/bin"
    assert environment["HTTPS_PROXY"] == "http://proxy.example:8080"
    assert environment[update_check.UPDATE_CACHE_ENV] == str(destination)
    assert environment[update_check.UPDATE_PREFERENCE_ENV] == str(preference)
    assert "OPENAI_API_KEY" not in environment
    assert update_check._read_refresh_attempt(destination) is not None


def test_disabled_background_refresh_never_spawns_a_child(tmp_path: Path):
    destination = tmp_path / "update-check.json"
    preference = tmp_path / "update-checks.json"
    update_check.set_update_checks_enabled(False, preference)

    with patch("kassiber.update_check.subprocess.Popen") as popen:
        update_check.start_background_refresh(destination, preference)

    popen.assert_not_called()
    assert update_check._read_refresh_attempt(destination) is None


def test_no_color_disables_ansi_not_the_human_update_check():
    args = Namespace(
        machine=False,
        non_interactive=False,
        output=None,
        command="status",
    )
    stream = _Tty()
    preference = Path("/tmp/kassiber-test-update-checks-enabled.json")
    with (
        patch.dict("os.environ", {"NO_COLOR": "1"}, clear=True),
        patch(
            "kassiber.update_check.update_checks_enabled",
            return_value=True,
        ),
        patch(
            "kassiber.update_check.packaged_build_info",
            return_value={"version": "0.22.55"},
        ),
    ):
        assert update_check.automatic_check_allowed(
            args,
            preference=preference,
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


def test_cli_can_disable_and_inspect_update_checks_without_network(tmp_path: Path):
    preference = tmp_path / "update-checks.json"
    stdout = io.StringIO()
    stderr = io.StringIO()
    with (
        patch.dict(
            os.environ,
            {update_check.UPDATE_PREFERENCE_ENV: str(preference)},
            clear=False,
        ),
        patch("kassiber.cli.main._configure_cli_logging"),
        patch("kassiber.cli.main.check_for_update") as check,
        patch("sys.stdout", stdout),
        patch("sys.stderr", stderr),
    ):
        exit_code = main(["--machine", "update", "--disable-checks"])

    assert exit_code == 0
    assert not update_check.update_checks_enabled(preference)
    check.assert_not_called()
    payload = json.loads(stdout.getvalue())
    assert payload["kind"] == "update.preference"
    assert payload["data"] == {"enabled": False, "contacts_github": False}
    assert stderr.getvalue() == ""


def test_cli_can_enable_consent_and_check_immediately(tmp_path: Path):
    preference = tmp_path / "update-checks.json"
    result = {
        "current_version": "0.22.55",
        "latest_version": "0.22.56",
        "update_available": True,
        "prerelease": True,
        "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
        "checked_at": "2026-07-22T08:30:00Z",
        "install_method": "manual",
        "update_command": None,
    }
    stdout = io.StringIO()
    with (
        patch.dict(
            os.environ,
            {update_check.UPDATE_PREFERENCE_ENV: str(preference)},
            clear=False,
        ),
        patch("kassiber.cli.main._configure_cli_logging"),
        patch("kassiber.cli.main.check_for_update", return_value=result) as check,
        patch("sys.stdout", stdout),
    ):
        exit_code = main(["--machine", "update", "--enable-checks"])

    assert exit_code == 0
    assert update_check.update_checks_enabled(preference)
    check.assert_called_once_with()
    assert json.loads(stdout.getvalue())["data"] == result


def test_disabled_explicit_check_fails_before_opening_network(tmp_path: Path):
    preference = tmp_path / "update-checks.json"
    opened = False

    def opener(request, *, timeout):
        del request, timeout
        nonlocal opened
        opened = True
        return _Response(_release_response())

    try:
        update_check.check_for_update(preference=preference, opener=opener)
    except update_check.AppError as error:
        assert error.code == "update_checks_disabled"
    else:
        raise AssertionError("disabled update check unexpectedly succeeded")
    assert opened is False

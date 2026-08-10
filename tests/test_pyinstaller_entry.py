import os
from unittest.mock import patch

from scripts import kassiber_pyinstaller_entry as entry


def test_frozen_ca_prefers_existing_linux_system_bundle(tmp_path):
    system_bundle = tmp_path / "system-ca-bundle.crt"
    system_bundle.write_text("system roots", encoding="utf-8")

    with patch.object(entry, "_SYSTEM_CA_BUNDLES", (system_bundle,)), patch.object(
        entry, "_loadable_ca_bundle", return_value=True
    ), patch.object(entry.sys, "platform", "linux"), patch.dict(
        os.environ, {}, clear=True
    ):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")

        assert selected == str(system_bundle)
        assert os.environ["SSL_CERT_FILE"] == str(system_bundle)
        assert os.environ["KASSIBER_HOST_CA_BUNDLE"] == "1"


def test_frozen_ca_preserves_explicit_linux_override(tmp_path):
    explicit = tmp_path / "operator.pem"
    with patch.object(entry.sys, "platform", "linux"), patch.dict(
        os.environ, {"SSL_CERT_FILE": str(explicit)}, clear=True
    ):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")
        assert os.environ["KASSIBER_HOST_CA_BUNDLE"] == "explicit"

    assert selected == str(explicit)


def test_frozen_ca_uses_certifi_outside_linux(tmp_path):
    system_bundle = tmp_path / "system-ca-bundle.crt"
    system_bundle.write_text("system roots", encoding="utf-8")
    with patch.object(entry, "_SYSTEM_CA_BUNDLES", (system_bundle,)), patch.object(
        entry.sys, "platform", "darwin"
    ), patch.dict(os.environ, {}, clear=True):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")

        assert selected == "/bundled/certifi.pem"
        assert "KASSIBER_HOST_CA_BUNDLE" not in os.environ


def test_frozen_ca_uses_certifi_when_linux_has_no_host_bundle(tmp_path):
    missing_bundle = tmp_path / "missing-ca-bundle.crt"
    with patch.object(entry, "_SYSTEM_CA_BUNDLES", (missing_bundle,)), patch.object(
        entry.sys, "platform", "linux"
    ), patch.dict(os.environ, {}, clear=True):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")

        assert selected == "/bundled/certifi.pem"
        assert os.environ["SSL_CERT_FILE"] == "/bundled/certifi.pem"
        assert "KASSIBER_HOST_CA_BUNDLE" not in os.environ


def test_frozen_ca_clears_inherited_internal_marker_on_certifi_fallback(tmp_path):
    missing_bundle = tmp_path / "missing-ca-bundle.crt"
    with patch.object(entry, "_SYSTEM_CA_BUNDLES", (missing_bundle,)), patch.object(
        entry.sys, "platform", "linux"
    ), patch.dict(
        os.environ,
        {"KASSIBER_HOST_CA_BUNDLE": "explicit"},
        clear=True,
    ):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")

        assert selected == "/bundled/certifi.pem"
        assert "KASSIBER_HOST_CA_BUNDLE" not in os.environ


def test_frozen_ca_skips_unloadable_host_bundle(tmp_path):
    broken_bundle = tmp_path / "broken-ca-bundle.crt"
    valid_bundle = tmp_path / "valid-ca-bundle.crt"
    broken_bundle.write_text("broken", encoding="utf-8")
    valid_bundle.write_text("valid", encoding="utf-8")

    with patch.object(
        entry, "_SYSTEM_CA_BUNDLES", (broken_bundle, valid_bundle)
    ), patch.object(
        entry,
        "_loadable_ca_bundle",
        side_effect=lambda path: path == valid_bundle,
    ), patch.object(entry.sys, "platform", "linux"), patch.dict(
        os.environ, {}, clear=True
    ):
        selected = entry._configure_frozen_ca_bundle("/bundled/certifi.pem")

    assert selected == str(valid_bundle)

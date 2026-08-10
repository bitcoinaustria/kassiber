"""PyInstaller entry point for prerelease CLI binaries."""

import os
import ssl
import sys
from pathlib import Path


_SYSTEM_CA_BUNDLES = (
    Path("/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem"),
    Path("/etc/pki/tls/certs/ca-bundle.crt"),
    Path("/etc/ssl/certs/ca-certificates.crt"),
    Path("/etc/ssl/cert.pem"),
)


def _loadable_ca_bundle(path):
    try:
        ssl.create_default_context(cafile=str(path))
    except (OSError, ssl.SSLError):
        return False
    return True


def _configure_frozen_ca_bundle(certifi_bundle):
    """Prefer an override, then Linux host trust, then bundled certifi."""

    os.environ.pop("KASSIBER_HOST_CA_BUNDLE", None)
    explicit = os.environ.get("SSL_CERT_FILE")
    if explicit:
        if sys.platform.startswith("linux"):
            os.environ["KASSIBER_HOST_CA_BUNDLE"] = "explicit"
        return explicit
    selected = (
        next(
            (
                path
                for path in _SYSTEM_CA_BUNDLES
                if path.is_file() and _loadable_ca_bundle(path)
            ),
            None,
        )
        if sys.platform.startswith("linux")
        else None
    )
    bundle = str(selected) if selected is not None else str(certifi_bundle)
    os.environ["SSL_CERT_FILE"] = bundle
    if selected is not None:
        os.environ["KASSIBER_HOST_CA_BUNDLE"] = "1"
    return bundle


# Frozen Linux builds prefer the installed host trust bundle; other platforms
# and minimal images fall back to the CA bundle shipped by certifi.
if getattr(sys, "frozen", False):
    import certifi

    _configure_frozen_ca_bundle(certifi.where())

from kassiber.cli.main import main


if __name__ == "__main__":
    raise SystemExit(main())

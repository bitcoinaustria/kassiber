"""Frozen-build TLS trust facts the pinned observer dependencies cannot honor.

`scripts/kassiber_pyinstaller_entry.py` records how a frozen build resolved its
CA bundle in `KASSIBER_HOST_CA_BUNDLE`, which is internal to Kassiber and not a
supported knob: `explicit` means the operator set `SSL_CERT_FILE` themselves,
`1` means the entry point discovered an installed host bundle on its own.

BDK and LWK ship Rustls/WebPKI roots and never read Python/OpenSSL's
`SSL_CERT_FILE`, so both observers use this to decide when a backend must stay on
the transport that does enforce the selected roots.
"""

from __future__ import annotations

import os


def operator_ca_override() -> bool:
    """Return whether the operator pinned this process's trust set themselves.

    Auto-discovered host bundles are deliberately excluded. They are present on
    essentially every Linux install, so treating them as a routing signal would
    take every HTTPS Esplora backend off the native observer to serve the few
    endpoints whose certificate needs a root outside WebPKI. Those endpoints get
    a per-backend `certificate` instead, which routes that one backend.
    """

    return str(os.environ.get("KASSIBER_HOST_CA_BUNDLE") or "").strip().lower() == "explicit"

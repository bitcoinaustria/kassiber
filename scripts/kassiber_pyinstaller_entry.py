"""PyInstaller entry point for prerelease CLI binaries."""

import os
import sys

# Bundled OpenSSL's baked-in CA paths only exist on the build host; route trust through certifi.
if getattr(sys, "frozen", False):
    import certifi

    os.environ.setdefault("SSL_CERT_FILE", certifi.where())

from kassiber.cli.main import main


if __name__ == "__main__":
    raise SystemExit(main())

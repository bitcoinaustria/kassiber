"""Views of Rust output without what only the Rust backend serves.

Bitpanda is a Rust-only wallet and backend kind, and the desktop's exchange
connection kinds are Rust-only daemon kinds
(docs/reference/rust-shell.md#rust-only-commands). Parity tests compare the
rest of a Rust catalog with Python's bytes after removing them.
"""
import re

# Advertised by the Rust daemon after Python's kinds and before test kinds.
RUST_ONLY_DAEMON_KINDS = (
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
)


def rust_only_daemon_kinds(kinds):
    """The Rust-only kinds among `kinds`, in their advertised order."""
    return [kind for kind in kinds if kind in RUST_ONLY_DAEMON_KINDS]


def python_daemon_kinds(kinds):
    """`kinds` without test kinds and Rust-only kinds."""
    return [kind for kind in kinds if not kind.startswith("test.") and kind not in RUST_ONLY_DAEMON_KINDS]


def without_rust_only_kinds(output: bytes) -> bytes:
    """Catalog output without the `bitpanda` kind in every output format:
    JSON objects, table and CSV rows (CRLF), plain blocks. The entry is never the
    widest row, so the rest must be Python's bytes."""
    output = re.sub(rb',\s*\{\s*"kind":\s*"bitpanda"[^{}]*\}', b"", output)
    output = re.sub(rb"(?m)^kind: bitpanda\n(?:[^\n]+\n)*\n", b"", output)
    return re.sub(rb"(?m)^bitpanda(?:[ ,][^\n]*)?\r?\n", b"", output)

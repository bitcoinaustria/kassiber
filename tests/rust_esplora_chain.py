"""The block endpoints of a fake Esplora server over a stable chain.

Each height has one block hash, so a tip that keeps its height keeps its
hash, and adding blocks raises the tip; a reorg exists only where a test
switches `branch`. Since the rounds-5/8 history anchors, Python's and
Rust's Esplora compatibility adapters read the tip hash and height and the
hash at a saved height (`block-height/<h>`) besides the script endpoints.
"""
import hashlib


def block_hash(height, branch=0):
    return hashlib.sha256(f"fixture-block-{branch}-{height}".encode()).hexdigest()


def block_response(parts, tip_height, branch=0):
    """`(status, body)` for an Esplora block endpoint, or None for another path."""
    if parts == ["blocks", "tip", "hash"]:
        return 200, block_hash(tip_height, branch).encode()
    if parts == ["blocks", "tip", "height"]:
        return 200, str(tip_height).encode()
    if len(parts) == 2 and parts[0] == "block-height" and parts[1].isascii() and parts[1].isdigit():
        height = int(parts[1])
        if height > tip_height:
            return 404, b"Block not found"
        return 200, block_hash(height, branch).encode()
    return None

"""Liquid wallets with a private view key (`ct(<xprv>/<0;1>/*, ...)`), natively.

LWK rejects a ranged view key, so Python syncs such a wallet through its
compatibility observers and unblinds with embit's derivation: the derived
view key plus `tagged_hash("elements/blindingkey", script)`. The `view_*`
phases of `crates/wallets/tests/vectors/lwk_chain.json` pay that wallet
confidential (L-BTC and an issued asset) and explicit outputs. Python's CLI
and the native binary sync copies of one book over the loopback Liquid
fakes; envelopes and every stored byte must match.
"""
import pytest

from tests.rust_liquid_chain import electrum, esplora
from tests.rust_sync_parity import connect
from tests.test_rust_liquid_compatibility import RUST_BIN, _round
from tests.test_rust_lwk_execution import VECTORS
from tests.test_rust_lwk_sync_service import wallet_config
from tests.test_rust_wallet_sync_cli import _books

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")
PHASES = VECTORS["phases"]
SEQUENCES = {
    "fresh": ["view_initial"],
    "restart": ["view_initial", "view_initial"],
    "extend": ["view_initial", "view_more"],
    "reorg": ["view_initial", "view_reorg"],
}


@pytest.mark.parametrize("sequence", sorted(SEQUENCES))
@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_view_key_books_match_python(tmp_path, protocol, sequence):
    roots, books = _books(tmp_path, wallet_config(descriptor=VECTORS["view_descriptor"]))
    state = {"phase": None}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        for index, phase in enumerate(SEQUENCES[sequence]):
            state["phase"] = PHASES[phase]
            outcome = _round(tmp_path, protocol, url, roots, books, index=index)
    data = outcome["data"][0]
    assert data["status"] == "synced", data
    assert (data["observer_route"], data["observer_compatibility_reason"]) == (
        "compatibility", "descriptor_unsupported")
    # The confidential change and the issued asset were unblinded.
    conn = connect(books[0])
    amounts = {(row[0], row[1]) for row in conn.execute(
        "SELECT external_id, amount FROM transactions WHERE amount > 0")}
    conn.close()
    assert any(txid == VECTORS["transactions"]["v_fund"] for txid, _ in amounts), amounts


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_view_key_forced_rebuild_matches_python(tmp_path, protocol):
    roots, books = _books(tmp_path, wallet_config(descriptor=VECTORS["view_descriptor"]))
    state = {"phase": PHASES["view_initial"]}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        _round(tmp_path, protocol, url, roots, books)
        state["phase"] = PHASES["view_more"]
        _round(tmp_path, protocol, url, roots, books, index=1, force_full=True)


def test_public_view_key_refuses_like_python(tmp_path):
    # A public view key derives addresses but cannot unblind: both refuse
    # before any request and write nothing.
    roots, books = _books(tmp_path, wallet_config(descriptor=VECTORS["public_view_descriptor"]))
    state = {"phase": PHASES["view_initial"]}
    with electrum(state) as url:
        outcome = _round(tmp_path, "electrum", url, roots, books)
    assert outcome["kind"] == "error" or outcome["data"][0]["status"] == "error", outcome


FIXED = {
    "slip77": ("fixed_slip77_descriptor", "fixed_slip77"),
    "view": ("fixed_view_descriptor", "fixed_view"),
    "view-uncompressed": ("fixed_view_uncompressed_descriptor", "fixed_view"),
}


@pytest.mark.parametrize("wallet", sorted(FIXED))
@pytest.mark.parametrize("protocol,extra", [("electrum", {"timeout": 45}), ("esplora", {})])
def test_fixed_branch_picks_up_confidential_outputs_like_python(tmp_path, wallet, protocol, extra):
    """A wallet with one fixed branch (`elwpkh(xpub/0/7)`) unblinds its own
    confidential outputs since Python round 14. Before, such an output failed
    the whole sync, which rolled back; a book that had synced only explicit
    outputs then picks the confidential one up on its next ordinary sync."""
    descriptor, phase = FIXED[wallet]
    roots, books = _books(tmp_path, wallet_config(descriptor=VECTORS[descriptor]))
    state = {"phase": PHASES["fixed_explicit"]}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        first = _round(tmp_path, protocol, url, roots, books, **extra)
        assert first["data"][0]["status"] == "synced", first
        state["phase"] = PHASES[phase]
        second = _round(tmp_path, protocol, url, roots, books, index=1, **extra)
    assert second["data"][0]["status"] == "synced", second
    conn = connect(books[1])
    amounts = sorted(row[0] for row in conn.execute("SELECT amount FROM transactions WHERE direction='inbound'"))
    conn.close()
    confidential = 25_000 if wallet == "slip77" else 18_000
    assert amounts == sorted([20_000 * 1000, confidential * 1000]), amounts


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_legacy_set_view_key_matches_python(tmp_path, protocol):
    # `{0,1}` is embit's legacy spelling of `<0;1>`, on both keys.
    roots, books = _books(tmp_path, wallet_config(descriptor=VECTORS["legacy_set_view_descriptor"]))
    state = {"phase": PHASES["view_initial"]}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        outcome = _round(tmp_path, protocol, url, roots, books)
        state["phase"] = PHASES["view_more"]
        outcome = _round(tmp_path, protocol, url, roots, books, index=1)
    assert outcome["data"][0]["status"] == "synced", outcome


def test_unblinding_failure_leaves_records_and_checkpoint_untouched(tmp_path):
    """Why an existing book needs nothing: an owned output that cannot be
    unblinded fails the whole sync (here an output blinded to another key,
    as a fixed branch's outputs were before round 14), so the records and
    the freshness checkpoint stay as the last good sync left them."""
    from tests.test_rust_liquid_compatibility import _round as compat_round

    roots, books = _books(tmp_path, wallet_config())
    state = {"phase": PHASES["compat_initial"]}

    def kept(book):
        conn = connect(book)
        try:
            rows = lambda sql: [tuple(row) for row in conn.execute(sql)]
            # (Reopening a book backfills Liquid asset ids into raw_json.)
            return (rows("SELECT id, external_id, direction, asset, amount, fee, occurred_at, confirmed_at "
                         "FROM transactions ORDER BY id"),
                    rows("SELECT checkpoint_json FROM freshness_source_states"),
                    rows("SELECT * FROM wallet_utxos ORDER BY id"))
        finally:
            conn.close()

    with electrum(state) as url:
        compat_round(tmp_path, "electrum", url, roots, books, timeout=45)
        before = [kept(book) for book in books]
        state["phase"] = PHASES["compat_wrong_key"]
        failed = compat_round(tmp_path, "electrum", url, roots, books, index=1, timeout=45)
    assert failed["kind"] == "error" or failed["data"][0]["status"] == "error", failed
    after = [kept(book) for book in books]
    assert after == before

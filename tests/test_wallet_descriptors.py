"""Unit tests for descriptor plan construction.

These pin the change-branch synthesis in ``load_descriptor_plan``: a wallet
configured with only a receive-chain descriptor (``.../0/*``) must still derive
its sibling change chain (``.../1/*``). Without it, change/internal addresses are
never derived or scanned and change UTXOs disappear from balances and the UTXO
list.

Single-sig addresses are checked against the canonical BIP84/BIP86 test vectors
for the standard ``abandon abandon ... about`` seed. Multisig and Liquid are
checked by equivalence: the synthesized ``<0;1>`` change branch must derive the
exact same addresses as an explicitly-configured ``/1/*`` change descriptor, so
the synthesis is provably identical to manual configuration rather than a guess.
"""

from __future__ import annotations

import unittest

from embit import bip32, bip39

from kassiber.wallet_descriptors import (
    derive_descriptor_targets,
    liquid_blinding_secret,
    liquid_plan_can_unblind,
    load_descriptor_plan,
)


_MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
# Independent key material for a multisig co-signer / Liquid master blinding key.
_COSIGNER_SEED = b"\x02" * 32
_BLINDING_SEED = b"\x03" * 32

# Canonical reference addresses for the seed above (index 0 of each chain).
BIP84_RECEIVE_0 = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
BIP84_CHANGE_0 = "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el"
BIP86_RECEIVE_0 = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"
BIP86_CHANGE_0 = "bc1p3qkhfews2uk44qtvauqyr2ttdsw7svhkl9nkm9s9c3x4ax5h60wqwruhk7"


def _account_descriptor(purpose: int, script_open: str, script_close: str) -> str:
    root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
    path = f"m/{purpose}h/0h/0h"
    fingerprint = root.my_fingerprint.hex()
    xpub = root.derive(path).to_public().to_base58()
    origin = path[2:].replace("m", "")
    return f"{script_open}[{fingerprint}/{origin}]{xpub}/0/*{script_close}"


def _cosigner_key(root, account_path: str, chain_index: int) -> str:
    fingerprint = root.my_fingerprint.hex()
    xpub = root.derive(account_path).to_public().to_base58()
    return f"[{fingerprint}/{account_path[2:]}]{xpub}/{chain_index}/*"


def _multisig_keys(account_path: str, chain_index: int) -> tuple[str, str]:
    root_a = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
    root_b = bip32.HDKey.from_seed(_COSIGNER_SEED)
    return (
        _cosigner_key(root_a, account_path, chain_index),
        _cosigner_key(root_b, account_path, chain_index),
    )


def _wsh_multisig(chain_index: int) -> str:
    key_a, key_b = _multisig_keys("m/48h/0h/0h/2h", chain_index)
    return f"wsh(sortedmulti(2,{key_a},{key_b}))"


def _nested_multisig(chain_index: int) -> str:
    key_a, key_b = _multisig_keys("m/48h/0h/0h/1h", chain_index)
    return f"sh(wsh(sortedmulti(2,{key_a},{key_b})))"


def _liquid_multisig(chain_index: int) -> str:
    key_a, key_b = _multisig_keys("m/48h/1h/0h/2h", chain_index)
    slip77_key = bip32.HDKey.from_seed(_BLINDING_SEED).key.wif()
    return f"ct(slip77({slip77_key}),elwsh(sortedmulti(2,{key_a},{key_b})))"


def _branch_address(plan, branch_index: int) -> str:
    targets = derive_descriptor_targets(plan, branch_index=branch_index, start=0, end=1)
    return targets[0].address


class ChangeBranchSynthesisTests(unittest.TestCase):
    def test_receive_only_wpkh_synthesizes_change_branch(self):
        plan = load_descriptor_plan(
            {"descriptor": _account_descriptor(84, "wpkh(", ")"), "chain": "bitcoin"}
        )

        labels = {branch.branch_index: branch.branch_label for branch in plan.branches}
        self.assertEqual(labels, {0: "receive", 1: "change"})
        self.assertEqual(_branch_address(plan, 0), BIP84_RECEIVE_0)
        self.assertEqual(_branch_address(plan, 1), BIP84_CHANGE_0)

    def test_receive_only_taproot_synthesizes_change_branch(self):
        plan = load_descriptor_plan(
            {"descriptor": _account_descriptor(86, "tr(", ")"), "chain": "bitcoin"}
        )

        self.assertEqual(_branch_address(plan, 0), BIP86_RECEIVE_0)
        self.assertEqual(_branch_address(plan, 1), BIP86_CHANGE_0)

    def test_receive_only_nested_segwit_synthesizes_change_branch(self):
        # BIP49 sh(wpkh(...)): change derives on chain index 1 as a P2SH address.
        plan = load_descriptor_plan(
            {"descriptor": _account_descriptor(49, "sh(wpkh(", "))"), "chain": "bitcoin"}
        )

        change = _branch_address(plan, 1)
        self.assertTrue(change.startswith("3"))
        self.assertNotEqual(_branch_address(plan, 0), change)

    def test_receive_only_legacy_synthesizes_change_branch(self):
        # BIP44 pkh(...): change derives on chain index 1 as a P2PKH address.
        plan = load_descriptor_plan(
            {"descriptor": _account_descriptor(44, "pkh(", ")"), "chain": "bitcoin"}
        )

        change = _branch_address(plan, 1)
        self.assertTrue(change.startswith("1"))
        self.assertNotEqual(_branch_address(plan, 0), change)

    def test_explicit_change_descriptor_is_not_overridden(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
        xpub = root.derive("m/84h/0h/0h").to_public().to_base58()

        plan = load_descriptor_plan(
            {
                "descriptor": f"wpkh({xpub}/0/*)",
                "change_descriptor": f"wpkh({xpub}/1/*)",
                "chain": "bitcoin",
            }
        )

        self.assertEqual(_branch_address(plan, 0), BIP84_RECEIVE_0)
        self.assertEqual(_branch_address(plan, 1), BIP84_CHANGE_0)

    def test_multipath_descriptor_is_unchanged(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
        xpub = root.derive("m/84h/0h/0h").to_public().to_base58()

        plan = load_descriptor_plan(
            {"descriptor": f"wpkh({xpub}/<0;1>/*)", "chain": "bitcoin"}
        )

        self.assertEqual(len(plan.branches), 2)
        self.assertEqual(_branch_address(plan, 1), BIP84_CHANGE_0)

    def test_fixed_address_descriptor_is_not_promoted(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
        xpub = root.derive("m/84h/0h/0h").to_public().to_base58()

        plan = load_descriptor_plan(
            {"descriptor": f"wpkh({xpub}/0/5)", "chain": "bitcoin"}
        )

        self.assertEqual([branch.branch_label for branch in plan.branches], ["receive"])

    def test_receive_only_liquid_descriptor_synthesizes_change_branch(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(_MNEMONIC))
        fingerprint = root.my_fingerprint.hex()
        xpub = root.derive("m/84h/1h/0h").to_public().to_base58()
        # A deterministic master blinding key keeps the fixture self-contained.
        slip77_key = bip32.HDKey.from_seed(_BLINDING_SEED).key.wif()
        descriptor = f"ct(slip77({slip77_key}),elwpkh([{fingerprint}/84h/1h/0h]{xpub}/0/*))"

        plan = load_descriptor_plan({"descriptor": descriptor, "chain": "liquid"})

        labels = {branch.branch_index: branch.branch_label for branch in plan.branches}
        self.assertEqual(labels, {0: "receive", 1: "change"})
        receive = _branch_address(plan, 0)
        change = _branch_address(plan, 1)
        self.assertTrue(receive.startswith("lq1"))
        self.assertTrue(change.startswith("lq1"))
        self.assertNotEqual(receive, change)

    def _assert_change_matches_explicit(self, descriptor_for, chain, *, count=5):
        """Assert the synthesized change branch equals an explicit `/1/*` config.

        ``descriptor_for(chain_index)`` builds the descriptor for a given chain.
        Returns the synthesized plan so callers can run extra checks.
        """
        receive_descriptor = descriptor_for(0)
        change_descriptor = descriptor_for(1)
        synthesized = load_descriptor_plan(
            {"descriptor": receive_descriptor, "chain": chain}
        )
        explicit = load_descriptor_plan(
            {
                "descriptor": receive_descriptor,
                "change_descriptor": change_descriptor,
                "chain": chain,
            }
        )
        synthesized_change = [
            target.address
            for target in derive_descriptor_targets(
                synthesized, branch_index=1, start=0, end=count
            )
        ]
        explicit_change = [
            target.address
            for target in derive_descriptor_targets(
                explicit, branch_index=1, start=0, end=count
            )
        ]
        self.assertEqual(len(synthesized_change), count)
        self.assertEqual(synthesized_change, explicit_change)
        return synthesized

    def test_multisig_change_matches_explicit_change_descriptor(self):
        self._assert_change_matches_explicit(_wsh_multisig, "bitcoin")

    def test_nested_multisig_change_matches_explicit_change_descriptor(self):
        self._assert_change_matches_explicit(_nested_multisig, "bitcoin")

    def test_liquid_multisig_change_matches_explicit_and_unblinds(self):
        plan = self._assert_change_matches_explicit(_liquid_multisig, "liquid")
        # The synthesized change branch must keep private blinding material, or
        # Liquid sync (which unblinds locally) would reject the wallet.
        self.assertTrue(liquid_plan_can_unblind(plan))
        secret, _target = liquid_blinding_secret(plan, branch_index=1, address_index=0)
        self.assertEqual(len(secret), 32)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

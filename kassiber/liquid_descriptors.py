"""Elements Taproot domain separation missing from embit's Liquid wrapper.

Imported lazily with the descriptor dependency. This class only parses and
derives public scripts; private Liquid blinding material stays in LDescriptor.
"""
from embit import script
from embit.descriptor.descriptor import Descriptor
from embit.descriptor.errors import DescriptorError
from embit.descriptor.taptree import TapLeaf, TapTree
from embit.hashes import tagged_hash
from embit.liquid.descriptor import BlindingKey, LDescriptor
from embit.misc import secp256k1


def _elements_tree_hash(tree):
    if isinstance(tree, TapTree):
        tree = tree.tree
    if tree is None:
        return b""
    if isinstance(tree, TapLeaf):
        # Elements' tapscript leaf version is 0xc4, versus Bitcoin's 0xc0.
        payload = b"\xc4" + script.Script(tree.miniscript.compile()).serialize()
        return tagged_hash("TapLeaf/elements", payload)
    left, right = sorted(_elements_tree_hash(child) for child in tree)
    return tagged_hash("TapBranch/elements", left + right)


class LiquidDescriptor(LDescriptor):
    @classmethod
    def read_from(cls, stream):
        # LDescriptor.read_from drops d.taptree when wrapping Descriptor. Keep
        # the tree so the Elements tweak commits to the authored script paths.
        start = stream.read(8)
        blinding_key = None
        if start == b"blinded(":
            blinding_key = BlindingKey.read_from(stream)
            if stream.read(1) != b",":
                raise DescriptorError("Missing bitcoin descriptor")
        else:
            stream.seek(-len(start), 1)
        spending = Descriptor.read_from(stream)
        if blinding_key is not None:
            if stream.read(1) != b")":
                raise DescriptorError("Missing ending bracket")
            if not blinding_key.slip77:
                if blinding_key.is_wildcard != spending.is_wildcard:
                    raise DescriptorError("Wildcards mismatch in blinded key and descriptor")
                if blinding_key.num_branches != spending.num_branches:
                    raise DescriptorError("Branches mismatch in blinded key and descriptor")
        return cls(
            spending.miniscript, sh=spending.sh, wsh=spending.wsh,
            key=spending.key, wpkh=spending.wpkh, taproot=spending.taproot,
            taptree=spending.taptree, blinding_key=blinding_key,
        )

    def script_pubkey(self):
        if not self.taproot:
            return super().script_pubkey()
        internal = self.key.xonly()
        tweak = tagged_hash("TapTweak/elements", internal + _elements_tree_hash(self.taptree))
        if not secp256k1.ec_seckey_verify(tweak):
            raise DescriptorError("Tweak is too large")
        point = secp256k1.ec_pubkey_parse(b"\x02" + internal)
        output = secp256k1.ec_pubkey_add(point, tweak)
        return script.Script(b"\x51\x20" + secp256k1.ec_pubkey_serialize(output)[1:33])

"""Elements Taproot domain separation missing from embit's Liquid wrapper.

Imported lazily with the descriptor dependency. This class only parses and
derives public scripts; private Liquid blinding material stays in LDescriptor.
"""
from embit import bech32, ec, script
from embit.descriptor.descriptor import Descriptor
from embit.descriptor.errors import DescriptorError
from embit.descriptor.taptree import TapLeaf, TapTree
from embit.hashes import tagged_hash
from embit.liquid.descriptor import BlindingKey, LDescriptor
from embit.liquid import blech32
from embit.liquid.networks import NETWORKS
from embit.misc import secp256k1


def addr_decode(address):
    """Decode Liquid witness addresses without losing their witness version.

    embit handles legacy addresses, but its Liquid witness decoder only
    understands Blech32 and always reconstructs witness version zero.
    """
    from embit.liquid.addresses import addr_decode as legacy_decode

    lower = address.lower()
    hrp, _, payload = lower.rpartition("1")
    if hrp not in {net["bech32"] for net in NETWORKS.values()} | {
        net.get("blech32") for net in NETWORKS.values()
    }:
        return legacy_decode(address)
    if address != lower and address != address.upper():
        raise ValueError("Invalid Liquid address")
    confidential = hrp in {net.get("blech32") for net in NETWORKS.values()}
    pub = None
    if confidential:
        if len(payload) < 13 or any(c not in blech32.CHARSET for c in payload):
            raise ValueError("Invalid Liquid address")
        data = [blech32.CHARSET.index(c) for c in payload]
        version = data[0]
        residue = 1 if version == 0 else 0x455972A3350F7A1
        if blech32.bech32_polymod(blech32.bech32_hrp_expand(hrp) + data) != residue:
            raise ValueError("Invalid Liquid address")
        decoded = blech32.convertbits(data[1:-12], 5, 8, False)
        if decoded is None or len(decoded) < 35:
            raise ValueError("Invalid Liquid address")
        pub = ec.PublicKey.parse(bytes(decoded[:33]))
        program = decoded[33:]
    else:
        version, program = bech32.decode(hrp, address)
    if version is None or not 0 <= version <= 16 or program is None:
        raise ValueError("Invalid Liquid address")
    if not 2 <= len(program) <= 40 or (version == 0 and len(program) not in (20, 32)):
        raise ValueError("Invalid Liquid address")
    opcode = 0 if version == 0 else 0x50 + version
    return script.Script(bytes([opcode, len(program)]) + bytes(program)), pub


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
    def address(self, network=NETWORKS["liquidv1"]):
        if not self.taproot or not self.is_blinded:
            return super().address(network)
        sc = self.script_pubkey()
        blinding_key = self.blinding_key.get_blinding_key(sc)
        data = [1] + blech32.convertbits(blinding_key.sec() + sc.data[2:], 8, 5)
        hrp = network["blech32"]
        # Witness v1 uses Blech32m. embit's Liquid encoder always uses the
        # witness-v0 Blech32 residue; reuse its conversion/polymod primitives.
        polymod = blech32.bech32_polymod(
            blech32.bech32_hrp_expand(hrp) + data + [0] * 12
        ) ^ 0x455972A3350F7A1
        checksum = [(polymod >> (5 * (11 - i))) & 31 for i in range(12)]
        return hrp + "1" + "".join(blech32.CHARSET[v] for v in data + checksum)

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

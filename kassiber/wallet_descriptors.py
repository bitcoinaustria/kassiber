from __future__ import annotations

import json
import re
from dataclasses import dataclass
from decimal import Decimal
from hashlib import sha256
from importlib import import_module

from .errors import AppError
from .wallet_setup import (
    BARE_XPUB_TEMPLATES,
    BSMS_DESCRIPTOR_SOURCE,
    parse_bsms_descriptor_record,
)
from .wallet_security import (
    assert_descriptor_is_watch_only,
    assert_descriptor_text_is_watch_only,
    assert_standalone_key_is_watch_only,
)


DEFAULT_DESCRIPTOR_GAP_LIMIT = 100
MAX_DESCRIPTOR_GAP_LIMIT = 5_000
HARDENED_INDEX = 0x80000000
LIQUID_POLICY_ASSET_IDS = {
    "liquidv1": "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
    "main": "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
    "liquidtestnet": "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
    "test": "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
    "elementsregtest": "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
    "regtest": "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
}
BITCOIN_NETWORK_ALIASES = {
    "main": "main",
    "mainnet": "main",
    "bitcoin": "main",
    "test": "test",
    "testnet": "test",
    "regtest": "regtest",
    "signet": "signet",
}
LIQUID_NETWORK_ALIASES = {
    "liquid": "liquidv1",
    "liquidv1": "liquidv1",
    "main": "liquidv1",
    "mainnet": "liquidv1",
    "liquidtestnet": "liquidtestnet",
    "test": "liquidtestnet",
    "testnet": "liquidtestnet",
    "elements": "elementsregtest",
    "elementsregtest": "elementsregtest",
    "regtest": "elementsregtest",
}
CHAIN_ALIASES = {
    "bitcoin": "bitcoin",
    "btc": "bitcoin",
    "onchain": "bitcoin",
    "liquid": "liquid",
    "lbtc": "liquid",
    "elements": "liquid",
}
_EMBIT_MODULES = None
_EXTENDED_PUBLIC_KEY_PREFIX_RE = re.compile(
    r"(?<![A-Za-z0-9])(?P<prefix>xpub|tpub)(?=[1-9A-HJ-NP-Za-km-z])"
)

# Fixed receive/change branch indices per script type. Stable bases keep a
# type's derived addresses and its sync checkpoint (highest_used is keyed by
# branch index) coherent when other types are enabled or disabled later: a
# type always owns the same two indices, so adding p2tr never renumbers p2wpkh.
SCRIPT_TYPE_BRANCH_BASE = {
    "p2pkh": 0,
    "p2sh-p2wpkh": 2,
    "p2wpkh": 4,
    "p2tr": 6,
}


@dataclass(frozen=True)
class DescriptorBranch:
    branch_index: int
    branch_label: str
    descriptor: object
    selector: int | None = None


@dataclass(frozen=True)
class DescriptorPlan:
    chain: str
    network: str
    gap_limit: int
    descriptor_fingerprint: str
    branches: tuple[DescriptorBranch, ...]


@dataclass(frozen=True)
class DerivedTarget:
    chain: str
    network: str
    branch_index: int
    branch_label: str
    address_index: int
    address: str
    unconfidential_address: str | None
    script_pubkey: str
    derivation_path: str | None
    derivation_paths: tuple[str, ...]
    key_origins: tuple[str, ...]


def get_embit_modules():
    global _EMBIT_MODULES
    if _EMBIT_MODULES is not None:
        return _EMBIT_MODULES
    try:
        _EMBIT_MODULES = {
            "Descriptor": import_module("embit.descriptor").Descriptor,
            "LDescriptor": import_module("embit.liquid.descriptor").LDescriptor,
            "BTC_NETWORKS": import_module("embit.networks").NETWORKS,
            "LIQUID_NETWORKS": import_module("embit.liquid.networks").NETWORKS,
            "LTransaction": import_module("embit.liquid.transaction").LTransaction,
            "PrivateKey": import_module("embit.ec").PrivateKey,
        }
    except ModuleNotFoundError as exc:
        raise AppError(
            "Descriptor-backed refresh requires the 'embit' package, "
            "but it is not available in this runtime.",
            code="dependency_missing",
            hint=(
                "Use a Kassiber desktop build that bundles embit, "
                "or reinstall the CLI with project dependencies."
            ),
            details={"missing_package": "embit"},
            retryable=False,
        ) from exc
    return _EMBIT_MODULES


def normalize_chain(value):
    chain = str(value or "bitcoin").strip().lower()
    if not chain:
        return "bitcoin"
    normalized = CHAIN_ALIASES.get(chain)
    if not normalized:
        raise ValueError(f"Unsupported wallet chain '{value}'")
    return normalized


def normalize_network(chain, value):
    chain = normalize_chain(chain)
    aliases = BITCOIN_NETWORK_ALIASES if chain == "bitcoin" else LIQUID_NETWORK_ALIASES
    default_network = "main" if chain == "bitcoin" else "liquidv1"
    network = str(value or default_network).strip().lower()
    if not network:
        network = default_network
    normalized = aliases.get(network)
    if not normalized:
        raise ValueError(f"Unsupported {chain} network '{value}'")
    return normalized


def _bitcoin_extended_key_network(*values: object) -> str | None:
    families = {
        "main" if match.group("prefix") == "xpub" else "test"
        for value in values
        for match in _EXTENDED_PUBLIC_KEY_PREFIX_RE.finditer(str(value or ""))
    }
    if len(families) > 1:
        raise AppError(
            "Bitcoin descriptors cannot mix mainnet and test-family extended public keys",
            code="validation",
            retryable=False,
        )
    return next(iter(families), None)


def _descriptor_network(chain: str, configured: object, *key_material: object) -> str:
    network = normalize_network(chain, configured)
    if chain != "bitcoin":
        return network
    key_network = _bitcoin_extended_key_network(*key_material)
    if key_network is None:
        return network
    if configured in (None, ""):
        return key_network
    configured_family = "main" if network == "main" else "test"
    if configured_family != key_network:
        raise AppError(
            "Extended public key network does not match the configured Bitcoin network",
            code="validation",
            details={"configured_network": network, "key_network": key_network},
            retryable=False,
        )
    return network


def default_policy_asset_id(network):
    return LIQUID_POLICY_ASSET_IDS.get(str(network or "").strip().lower())


def normalize_asset_code(value):
    text = str(value or "").strip()
    if not text:
        return ""
    lowered = text.lower()
    if len(lowered) == 64 and all(char in "0123456789abcdef" for char in lowered):
        return lowered
    if lowered in {"l-btc", "lbtc"}:
        return "LBTC"
    return text.upper()


def liquid_asset_code(asset_id, policy_asset_id=None):
    normalized = normalize_asset_code(asset_id)
    policy_asset_id = normalize_asset_code(policy_asset_id)
    if normalized and policy_asset_id and normalized == policy_asset_id:
        return "LBTC"
    return normalized


def _normalize_slip77_key(match):
    modules = get_embit_modules()
    secret = bytes.fromhex(match.group(1))
    return f"slip77({modules['PrivateKey'](secret).wif()})"


def _compact_descriptor_text(descriptor_text):
    # Output descriptors do not use significant whitespace, and normalizing it
    # here lets `--descriptor-file` accept readable multi-line formatting.
    return re.sub(r"\s+", "", str(descriptor_text or ""))


def normalize_descriptor_text(chain, descriptor_text):
    text = _compact_descriptor_text(descriptor_text)
    if not text:
        return ""
    if "#" in text:
        text = text.split("#", 1)[0]
    if normalize_chain(chain) != "liquid":
        return text
    normalized = text
    if normalized.startswith("ct("):
        normalized = f"blinded({normalized[3:]}"
    replacements = {
        "elwpkh(": "wpkh(",
        "elwsh(": "wsh(",
        "elsh(": "sh(",
        "eltr(": "tr(",
    }
    for source, target in replacements.items():
        normalized = normalized.replace(source, target)
    return re.sub(r"slip77\(([0-9a-fA-F]{64})\)", _normalize_slip77_key, normalized)


# Standard BIP44/49/84/86 wallets pair a receive chain (`.../0/*`) with a
# sibling change chain (`.../1/*`). Most hand-pasted descriptors and many wallet
# exports only carry the receive line, so without synthesizing the change branch
# Kassiber never derives or scans internal addresses and change UTXOs silently
# vanish from balances and the UTXO list. embit already understands the `<0;1>`
# multipath form, so promoting a single-branch receive descriptor to it lets the
# existing two-branch plan logic cover the change chain too.
_RECEIVE_CHAIN_WILDCARD_RE = re.compile(r"/0/\*")


def _promote_receive_only_to_multipath(descriptor_class, normalized_text, primary):
    """Return a `<0;1>` multipath descriptor when `primary` is receive-only.

    Falls back to the original ``primary`` whenever promotion is not applicable
    or the promoted text does not parse into exactly two branches, so a
    descriptor that loads today never starts failing because of this helper.
    """
    if getattr(primary, "num_branches", 1) >= 2:
        return primary
    if not getattr(primary, "is_wildcard", False):
        return primary
    promoted_text, substitutions = _RECEIVE_CHAIN_WILDCARD_RE.subn("/<0;1>/*", normalized_text)
    if substitutions == 0:
        return primary
    try:
        promoted = descriptor_class.from_string(promoted_text)
    except Exception:
        return primary
    if getattr(promoted, "num_branches", 1) != 2:
        return primary
    return promoted


def ordered_script_types(script_types):
    """Return the valid, deduped enabled script types in canonical branch order."""
    requested = {str(value or "").strip().lower() for value in (script_types or [])}
    return [stype for stype in SCRIPT_TYPE_BRANCH_BASE if stype in requested]


def enabled_script_branches(xpub, script_types, descriptor_class):
    """Build receive/change branches for each enabled script type of a bare xpub.

    Each type owns a fixed two-index block (receive=base, change=base+1) and is
    wrapped as a ``<0;1>`` multipath descriptor, with a fallback to explicit
    per-branch descriptors if multipath does not parse into two branches.
    """
    branches = []
    for script_type in ordered_script_types(script_types):
        base = SCRIPT_TYPE_BRANCH_BASE[script_type]
        template = BARE_XPUB_TEMPLATES[script_type]
        multipath = descriptor_class.from_string(template.format(key=xpub, branch="<0;1>"))
        if getattr(multipath, "num_branches", 1) >= 2:
            branches.append(DescriptorBranch(base, f"{script_type} receive", multipath, 0))
            branches.append(DescriptorBranch(base + 1, f"{script_type} change", multipath, 1))
        else:
            receive = descriptor_class.from_string(template.format(key=xpub, branch=0))
            change = descriptor_class.from_string(template.format(key=xpub, branch=1))
            branches.append(DescriptorBranch(base, f"{script_type} receive", receive, None))
            branches.append(DescriptorBranch(base + 1, f"{script_type} change", change, None))
    return branches


def load_descriptor_plan(config):
    descriptor_text = str(config.get("descriptor") or "").strip()
    xpub = str(config.get("xpub") or "").strip()
    # Field names are not a trust boundary: reject a private extended key even
    # when an incomplete/malformed config has placed it in the nominal xpub
    # slot and would otherwise return no descriptor plan.
    assert_standalone_key_is_watch_only(xpub)
    # Validate both named branches even when the primary plan is synthesized
    # from an xpub. Otherwise an ignored private change_descriptor could remain
    # persisted and later become active after an ordinary config edit.
    assert_descriptor_text_is_watch_only(descriptor_text)
    assert_descriptor_text_is_watch_only(config.get("change_descriptor"))
    script_types = ordered_script_types(config.get("script_types"))
    if not descriptor_text and not (xpub and script_types):
        return None
    change_text = str(config.get("change_descriptor") or "").strip()
    synthesize_change = config.get("synthesize_change") is not False
    if descriptor_text:
        bsms_descriptors = parse_bsms_descriptor_record(descriptor_text)
        if bsms_descriptors:
            descriptor_text = bsms_descriptors["descriptor"]
            synthesize_change = False
            if not change_text:
                change_text = bsms_descriptors.get("change_descriptor", "")
        elif config.get("descriptor_source") == BSMS_DESCRIPTOR_SOURCE:
            synthesize_change = False
    chain = normalize_chain(config.get("chain"))
    network = _descriptor_network(
        chain,
        config.get("network"),
        xpub,
        descriptor_text,
        change_text,
    )
    gap_limit = int(config.get("gap_limit") or DEFAULT_DESCRIPTOR_GAP_LIMIT)
    if gap_limit <= 0:
        raise ValueError("Descriptor gap limit must be positive")
    if gap_limit > MAX_DESCRIPTOR_GAP_LIMIT:
        raise ValueError(
            f"Descriptor gap limit must be {MAX_DESCRIPTOR_GAP_LIMIT} or lower"
        )
    modules = get_embit_modules()
    descriptor_class = modules["Descriptor"] if chain == "bitcoin" else modules["LDescriptor"]
    if not descriptor_text:
        # Multi-script xpub wallet: one receive/change branch pair per enabled
        # script type, each at that type's fixed branch indices.
        branches = enabled_script_branches(xpub, script_types, descriptor_class)
        for branch in branches:
            assert_descriptor_is_watch_only(branch.descriptor)
        fingerprint = sha256(
            json.dumps(
                {
                    "chain": chain,
                    "network": network,
                    "xpub": xpub,
                    "script_types": script_types,
                    "gap_limit": gap_limit,
                },
                sort_keys=True,
            ).encode("utf-8")
        ).hexdigest()
        return DescriptorPlan(
            chain=chain,
            network=network,
            gap_limit=gap_limit,
            descriptor_fingerprint=fingerprint,
            branches=tuple(branches),
        )
    normalized_primary_text = normalize_descriptor_text(chain, descriptor_text)
    primary = descriptor_class.from_string(normalized_primary_text)
    change_descriptor = (
        descriptor_class.from_string(normalize_descriptor_text(chain, change_text)) if change_text else None
    )
    assert_descriptor_is_watch_only(primary)
    if change_descriptor is not None:
        assert_descriptor_is_watch_only(change_descriptor)
    if change_descriptor is None and synthesize_change:
        # No explicit change descriptor: derive the sibling change chain from a
        # receive-only descriptor so internal/change UTXOs are not missed.
        primary = _promote_receive_only_to_multipath(
            descriptor_class, normalized_primary_text, primary
        )
    branches = []
    if change_descriptor is not None:
        branches.append(DescriptorBranch(0, "receive", primary, None))
        branches.append(DescriptorBranch(1, "change", change_descriptor, None))
    elif getattr(primary, "num_branches", 1) >= 2:
        branches.append(DescriptorBranch(0, "receive", primary, 0))
        branches.append(DescriptorBranch(1, "change", primary, 1))
    else:
        branches.append(DescriptorBranch(0, "receive", primary, None))
    fingerprint = sha256(
        json.dumps(
            {
                "chain": chain,
                "network": network,
                "descriptor": descriptor_text,
                "change_descriptor": change_text,
                "gap_limit": gap_limit,
            },
            sort_keys=True,
        ).encode("utf-8")
    ).hexdigest()
    return DescriptorPlan(
        chain=chain,
        network=network,
        gap_limit=gap_limit,
        descriptor_fingerprint=fingerprint,
        branches=tuple(branches),
    )


def branch_limits(plan):
    limits = {}
    for branch in plan.branches:
        descriptor = branch_descriptor(branch)
        limits[branch.branch_index] = plan.gap_limit if getattr(descriptor, "is_wildcard", False) else 1
    return limits


def branch_descriptor(branch):
    if branch.selector is not None and getattr(branch.descriptor, "num_branches", 1) > 1:
        return branch.descriptor.branch(branch.selector)
    return branch.descriptor


def _format_bip32_index(index):
    value = int(index)
    if value >= HARDENED_INDEX:
        return f"{value - HARDENED_INDEX}'"
    return str(value)


def _format_bip32_path(derivation):
    parts = [_format_bip32_index(index) for index in (derivation or [])]
    if not parts:
        return "m"
    return "m/" + "/".join(parts)


def _format_key_origin(origin):
    fingerprint = getattr(origin, "fingerprint", b"") or b""
    fingerprint_hex = fingerprint.hex() if isinstance(fingerprint, (bytes, bytearray)) else ""
    path = _format_bip32_path(getattr(origin, "derivation", []) or [])
    if fingerprint_hex and path != "m":
        return f"[{fingerprint_hex}/{path[2:]}]"
    if fingerprint_hex:
        return f"[{fingerprint_hex}]"
    return path


def _descriptor_derivation_metadata(derived):
    keys = list(getattr(derived, "keys", None) or [])
    if not keys and getattr(derived, "key", None) is not None:
        keys = [derived.key]
    derivation_paths = []
    key_origins = []
    for key in keys:
        origin = getattr(key, "origin", None)
        if origin is None:
            continue
        derivation_path = _format_bip32_path(getattr(origin, "derivation", []) or [])
        key_origin = _format_key_origin(origin)
        if derivation_path not in derivation_paths:
            derivation_paths.append(derivation_path)
        if key_origin not in key_origins:
            key_origins.append(key_origin)
    derivation_path = derivation_paths[0] if len(derivation_paths) == 1 else None
    return derivation_path, tuple(derivation_paths), tuple(key_origins)


def derive_descriptor_target(plan, branch_index, address_index):
    branch = next((item for item in plan.branches if item.branch_index == branch_index), None)
    if branch is None:
        raise ValueError(f"Descriptor branch '{branch_index}' is not configured")
    descriptor = branch_descriptor(branch)
    if getattr(descriptor, "is_wildcard", False):
        derived = descriptor.derive(address_index)
    else:
        if address_index != 0:
            raise ValueError(f"Descriptor branch '{branch.branch_label}' is not ranged")
        derived = descriptor
    modules = get_embit_modules()
    network_map = modules["BTC_NETWORKS"] if plan.chain == "bitcoin" else modules["LIQUID_NETWORKS"]
    network = network_map[plan.network]
    address = derived.address(network)
    script_pubkey = derived.script_pubkey().data.hex()
    unconfidential_address = None
    if plan.chain == "liquid":
        unconfidential_address = derived.script_pubkey().address(network)
    derivation_path, derivation_paths, key_origins = _descriptor_derivation_metadata(derived)
    return DerivedTarget(
        chain=plan.chain,
        network=plan.network,
        branch_index=branch.branch_index,
        branch_label=branch.branch_label,
        address_index=address_index,
        address=address,
        unconfidential_address=unconfidential_address,
        script_pubkey=script_pubkey,
        derivation_path=derivation_path,
        derivation_paths=derivation_paths,
        key_origins=key_origins,
    )


def derive_descriptor_targets(plan, branch_index=None, start=0, end=0):
    if start < 0 or end < start:
        raise ValueError("Descriptor range must satisfy 0 <= start <= end")
    selected = [
        branch for branch in plan.branches if branch_index is None or branch.branch_index == branch_index
    ]
    results = []
    for branch in selected:
        descriptor = branch_descriptor(branch)
        max_end = end
        if not getattr(descriptor, "is_wildcard", False):
            max_end = min(end, 1)
        for address_index in range(start, max_end):
            results.append(derive_descriptor_target(plan, branch.branch_index, address_index))
    return results


def liquid_plan_can_unblind(plan):
    if plan.chain != "liquid":
        return True
    for branch in plan.branches:
        descriptor = branch_descriptor(branch)
        if not getattr(descriptor, "is_blinded", False):
            return False
        blinding_key = getattr(descriptor, "blinding_key", None)
        if blinding_key is None:
            return False
        key = getattr(blinding_key, "key", None)
        if key is None:
            return False
        if not getattr(key, "is_private", False):
            return False
    return True


def liquid_blinding_secret(plan, branch_index, address_index):
    derived_target = derive_descriptor_target(plan, branch_index, address_index)
    branch = next(item for item in plan.branches if item.branch_index == branch_index)
    descriptor = branch_descriptor(branch)
    if getattr(descriptor, "is_wildcard", False):
        descriptor = descriptor.derive(address_index)
    script_pubkey = descriptor.script_pubkey()
    blinding_key = descriptor.blinding_key.get_blinding_key(script_pubkey)
    if not hasattr(blinding_key, "secret"):
        raise ValueError("Liquid descriptor does not include the private blinding material needed for unblinding")
    return blinding_key.secret, derived_target


def decode_liquid_transaction(raw_hex):
    modules = get_embit_modules()
    return modules["LTransaction"].from_string(raw_hex)


def liquid_amount_from_value(value):
    if isinstance(value, Decimal):
        return value
    return Decimal(str(value))

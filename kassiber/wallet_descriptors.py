from __future__ import annotations

import json
import re
from dataclasses import dataclass
from decimal import Decimal
from hashlib import sha256
from importlib import import_module


DEFAULT_DESCRIPTOR_GAP_LIMIT = 20
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
        raise ValueError(
            "Descriptor-backed wallet sync requires the 'embit' package. Reinstall Kassiber in a Python >= 3.10 environment."
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


def default_policy_asset_id(network):
    return LIQUID_POLICY_ASSET_IDS.get(str(network or "").strip().lower())


def normalize_asset_code(value):
    text = str(value or "").strip()
    if not text:
        return ""
    lowered = text.lower()
    if len(lowered) == 64 and all(char in "0123456789abcdef" for char in lowered):
        return lowered
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


def normalize_descriptor_text(chain, descriptor_text):
    text = str(descriptor_text or "").strip()
    if not text:
        return ""
    if "#" in text:
        text = text.split("#", 1)[0].strip()
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


def load_descriptor_plan(config):
    descriptor_text = str(config.get("descriptor") or "").strip()
    if not descriptor_text:
        return None
    chain = normalize_chain(config.get("chain"))
    network = normalize_network(chain, config.get("network"))
    gap_limit = int(config.get("gap_limit") or DEFAULT_DESCRIPTOR_GAP_LIMIT)
    if gap_limit <= 0:
        raise ValueError("Descriptor gap limit must be positive")
    modules = get_embit_modules()
    descriptor_class = modules["Descriptor"] if chain == "bitcoin" else modules["LDescriptor"]
    primary = descriptor_class.from_string(normalize_descriptor_text(chain, descriptor_text))
    change_text = str(config.get("change_descriptor") or "").strip()
    change_descriptor = (
        descriptor_class.from_string(normalize_descriptor_text(chain, change_text)) if change_text else None
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
    return DerivedTarget(
        chain=plan.chain,
        network=plan.network,
        branch_index=branch.branch_index,
        branch_label=branch.branch_label,
        address_index=address_index,
        address=address,
        unconfidential_address=unconfidential_address,
        script_pubkey=script_pubkey,
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

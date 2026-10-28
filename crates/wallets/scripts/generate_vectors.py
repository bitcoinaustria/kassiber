#!/usr/bin/env python3
"""Freeze synthetic Python wallet/descriptor behavior before the Rust port.

Run with the checkout's locked Python environment. No book or network is used.
Private inputs below are deterministic test keys, never user wallet material.
--check compares against the accepted file without rewriting it.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import importlib.metadata
import json
from pathlib import Path

from embit import bip32, ec
from embit.descriptor.checksum import add_checksum

from kassiber.core.ownership_policy_epochs import private_policy_material, policy_identity_material
from kassiber.core.wallets import (
    WALLET_KINDS, _validated_wallet_config, redact_wallet_config_for_output,
)
from kassiber.errors import AppError
from kassiber.wallet_descriptors import (
    branch_descriptor, branch_limits, derive_descriptor_targets,
    liquid_plan_can_unblind, load_descriptor_plan, normalize_descriptor_text,
)
from kassiber.wallet_setup import normalize_wallet_material
from kassiber.daemon import _preview_descriptor_payload
from kassiber.cli.handlers import resolve_descriptor_branch_index

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_descriptors.json"
REFERENCE = "56f61b18"


def capture(call):
    try:
        return {"ok": call()}
    except AppError as exc:
        return {"error": {
            "code": exc.code, "message": str(exc), "hint": exc.hint,
            "details": exc.details, "retryable": exc.retryable,
        }}
    except (ValueError, TypeError) as exc:
        return {"exception": {"type": type(exc).__name__, "message": str(exc)}}


def plan_result(config, *, start=0, end=5):
    plan = load_descriptor_plan(config)
    if plan is None:
        return None
    return {
        "chain": plan.chain, "network": plan.network, "gap_limit": plan.gap_limit,
        "descriptor_fingerprint": plan.descriptor_fingerprint,
        "can_unblind": liquid_plan_can_unblind(plan),
        "branches": [{
            "branch_index": b.branch_index, "branch_label": b.branch_label,
            "selector": b.selector, "descriptor": str(b.descriptor),
            "selected_descriptor": str(branch_descriptor(b)),
            "limit": branch_limits(plan)[b.branch_index],
        } for b in plan.branches],
        "targets": [asdict(t) for t in derive_descriptor_targets(plan, start=start, end=end)],
    }


def vectors():
    # Independent synthetic cosigners and view key; no real wallet fixture.
    roots = [bip32.HDKey.from_seed(bytes([i]) * 32) for i in (1, 2)]
    accounts = [r.derive("m/84h/0h/0h") for r in roots]
    xpub, second = [a.to_public().to_base58() for a in accounts]
    tpub = accounts[0].to_public().to_base58(version=bytes.fromhex("043587cf"))
    origin = f"[{roots[0].my_fingerprint.hex()}/84h/0h/0h]"
    key = f"{origin}{xpub}"
    pubkey = accounts[0].key.get_public_key().sec().hex()
    view = ec.PrivateKey(bytes([3]) * 32).wif()
    receive = f"wpkh({key}/0/*)"
    plans = []
    materials = []
    configs = []

    def plan(name, config, **kwargs):
        plans.append({"name": name, "config": config, **kwargs,
                      "expected": capture(lambda: plan_result(config, **kwargs))})

    def material(name, text, **kwargs):
        materials.append({"name": name, "material": text, "options": kwargs,
                          "expected": capture(lambda: normalize_wallet_material(text, **kwargs))})

    def config(name, kind, value):
        def result():
            normalized = _validated_wallet_config(kind, value)
            return {"config": normalized,
                    "config_json": json.dumps(normalized, sort_keys=True),
                    "safe_config": redact_wallet_config_for_output(normalized),
                    "private_policy_material": private_policy_material(normalized),
                    "policy_identity_material": policy_identity_material(normalized)}
        configs.append({"name": name, "kind": kind, "config": value,
                        "expected": capture(result)})

    for label, template in (
        ("pkh", "pkh({k}/0/*)"), ("wpkh", "wpkh({k}/0/*)"),
        ("sh-wpkh", "sh(wpkh({k}/0/*))"), ("tr", "tr({k}/0/*)"),
        ("wsh-multi", "wsh(multi(2,{k}/0/*,{b}/0/*))"),
        ("wsh-sortedmulti", "wsh(sortedmulti(2,{b}/0/*,{k}/0/*))"),
        ("sh-wsh-sortedmulti", "sh(wsh(sortedmulti(2,{k}/0/*,{b}/0/*)))"),
        ("sh-multi", "sh(multi(1,{k}/0/*,{b}/0/*))"),
    ):
        descriptor = template.format(k=key, b=second)
        plan(label, {"descriptor": descriptor})
        plan(label + "-multipath", {"descriptor": descriptor.replace("/0/*", "/<0;1>/*")})
        plan(label + "-pair", {"descriptor": descriptor, "change_descriptor": descriptor.replace("/0/*", "/1/*")})
        plan(label + "-fixed", {"descriptor": descriptor.replace("/0/*", "/0/7")})

    plan("origin-hardened-apostrophe", {"descriptor": receive.replace("h/", "'/").replace("h]", "']")})
    plan("origin-absent", {"descriptor": f"wpkh({xpub}/0/*)"})
    plan("static-key", {"descriptor": f"wpkh({pubkey})"})
    plan("static-xonly-tr", {"descriptor": f"tr({pubkey[2:]})"})
    plan("tr-script-tree", {"descriptor": f"tr({key}/0/*,pk({second}/0/*))"})
    plan("checksum-correct", {"descriptor": add_checksum(receive)})
    plan("checksum-invalid-ignored", {"descriptor": receive + "#00000000"})
    plan("whitespace", {"descriptor": " \n wpkh( " + key + "/0/* )\t"})
    plan("no-synthesis", {"descriptor": receive, "synthesize_change": False})
    plan("source-bsms", {"descriptor": receive, "descriptor_source": "bsms"})
    plan("gap-default-zero", {"descriptor": receive, "gap_limit": 0})
    for gap in (1, 5000, -1, 5001, "12", "invalid"):
        plan(f"gap-{gap}", {"descriptor": receive, "gap_limit": gap})
    for network in (None, "", "mainnet", "bitcoin", "test", "signet", "regtest", "invalid"):
        for prefix, k in (("xpub", xpub), ("tpub", tpub)):
            plan(f"{prefix}-network-{network}", {"descriptor": f"wpkh({k}/0/*)", "network": network})
    plan("mixed-networks", {"descriptor": f"wsh(sortedmulti(1,{xpub}/0/*,{tpub}/0/*))"})
    for chain in ("BTC", "onchain", "invalid"):
        plan(f"chain-{chain}", {"descriptor": receive, "chain": chain})
    for types in (["p2pkh"], ["p2sh-p2wpkh"], ["p2wpkh"], ["p2tr"],
                  ["p2tr", "p2wpkh", "p2wpkh"], ["p2tr", "p2pkh", "p2wpkh", "p2sh-p2wpkh"], []):
        plan("multi-script-" + "+".join(types), {"xpub": xpub, "script_types": types})

    for network in ("liquidv1", "liquidtestnet", "elementsregtest"):
        for script in ("elwpkh({k}/0/*)", "elsh(elwpkh({k}/0/*))",
                       "elwsh(sortedmulti(2,{k}/0/*,{b}/0/*))", "eltr({k}/0/*)"):
            descriptor = f"ct(slip77({view})," + script.format(k=key, b=second) + ")"
            plan(f"{network}-{script.split('(')[0]}", {"descriptor": descriptor, "chain": "liquid", "network": network})
    plan("liquid-hex-view", {"descriptor": f"ct(slip77({'03' * 32}),elwpkh({key}/0/*))", "chain": "liquid"})
    plan("liquid-unblinded", {"descriptor": receive, "chain": "liquid"})
    plan("liquid-tr-script-tree", {"descriptor": f"ct(slip77({view}),eltr({key}/0/*,pk({second}/0/*)))", "chain": "liquid"})

    versions = {"xpub": "0488b21e", "ypub": "049d7cb2", "zpub": "04b24746",
                "tpub": "043587cf", "upub": "044a5262", "vpub": "045f1cf6"}
    for prefix, version in versions.items():
        encoded = accounts[0].to_public().to_base58(version=bytes.fromhex(version))
        options = {"script_types": ["p2wpkh", "p2tr"]} if prefix in {"xpub", "tpub"} else {}
        material(prefix, encoded, **options)
        normalized = normalize_wallet_material(encoded, **options)
        plan(f"material-{prefix}", normalized)
        if prefix in {"xpub", "tpub"}:
            material(prefix + "-ambiguous", encoded)
            for stype in ("p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr", "invalid"):
                material(prefix + "-" + stype, encoded, script_type=stype)
    material("core-json", json.dumps({"descriptors": [{"desc": receive, "internal": False},
             {"desc": receive.replace("/0/*", "/1/*"), "internal": True}]}))
    material("sparrow-text", "# Wallet export\nreceive: " + receive + "\nchange: " + receive.replace("/0/*", "/1/*"))
    for restriction in ("/0/*,/1/*", "/0/*", "no path restrictions", "/0h/*", "/0/*,/1/*,/2/*"):
        text = "BSMS 1.0\n" + receive.replace("/0/*", "/**") + "\n" + restriction + "\nfirst-address"
        material("bsms-" + restriction, text)
        plan("bsms-" + restriction, {"descriptor": text})
    for name, text in (("empty", ""), ("unsupported", "not a wallet"),
                       ("invalid-base58", "zpub0"), ("bad-key-checksum", xpub[:-1] + ("1" if xpub[-1] != "1" else "2")),
                       ("bsms-incomplete", "BSMS 1.0\n" + receive)):
        material(name, text)
    for name, descriptor in (("empty", ""), ("malformed", "wpkh("),
                            ("invalid-key", "wpkh(invalid)"), ("unsupported-addr", "addr(bc1qinvalid)"),
                            ("unsupported-raw", "raw(0014abcd)"), ("unsupported-combo", f"combo({pubkey})")):
        plan(name, {"descriptor": descriptor})
    private = accounts[0].to_base58()
    for name, descriptor in (
        ("private-wpkh", f"wpkh({private}/0/*)"),
        ("private-sh", f"sh(wpkh({private}/0/*))"),
        ("private-tr", f"tr({private}/0/*)"),
        ("private-multi", f"wsh(sortedmulti(1,{private}/0/*,{xpub}/0/*))"),
        ("private-multipath", f"wpkh({private}/<0;1>/*)"),
        ("private-wif", f"wpkh({accounts[0].key.wif()})"),
        ("private-liquid", f"ct(slip77({view}),elwpkh({private}/0/*))"),
    ):
        value = {"descriptor": descriptor, **({"chain": "liquid"} if "liquid" in name else {})}
        plan(name, value)
        config(name, "descriptor", value)
        material(name, descriptor)
    plan("private-change", {"descriptor": receive, "change_descriptor": f"wpkh({private}/1/*)"})
    for types in (None, ["p2wpkh"]):
        plan("private-xpub-" + str(types), {"xpub": private, "script_types": types})
    for prefix, version in {"xprv": "0488ade4", "tprv": "04358394", "yprv": "049d7878",
                            "zprv": "04b2430c", "uprv": "044a4e28", "vprv": "045f18bc"}.items():
        encoded = accounts[0].to_base58(version=bytes.fromhex(version))
        material("standalone-" + prefix, encoded)
        plan("descriptor-" + prefix, {"descriptor": f"wpkh({encoded}/0/*)"})
    for kind in WALLET_KINDS:
        value = {"addresses": [" bc1qplaceholder ", "bc1qplaceholder"], "backend": "local"}
        if kind in {"descriptor", "xpub"}:
            value["descriptor"] = receive
        config("kind-" + kind, kind, value)
    for name, kind, value in (
        ("empty-descriptor", "descriptor", {}), ("empty-address", "address", {}),
        ("empty-coreln", "coreln", {}), ("bare-xpub-no-types", "xpub", {"xpub": xpub}),
        ("scan-depth-negative", "descriptor", {"descriptor": receive, "ownership_scan_to_index": -1}),
        ("scan-depth-string", "descriptor", {"descriptor": receive, "ownership_scan_to_index": "23"}),
        ("redaction", "descriptor", {"descriptor": receive, "change_descriptor": receive.replace("/0/*", "/1/*"),
             "unknown_secret": "do-not-expose", "blinding_key": view, "altbestand": True, "deprecated": "yes"}),
        ("liquid-no-view", "descriptor", {"descriptor": receive, "chain": "liquid", "backend": "local"}),
        ("liquid-no-backend", "descriptor", {"descriptor": f"ct(slip77({view}),elwpkh({key}/0/*))", "chain": "liquid"}),
        ("liquid-config", "descriptor", {"descriptor": f"ct(slip77({view}),elwpkh({key}/0/*))", "chain": "liquid", "backend": "local"}),
    ):
        config(name, kind, value)
    normalizations = [{"chain": chain, "text": text, "expected": capture(lambda c=chain, t=text: normalize_descriptor_text(c, t))}
                      for chain, text in (("bitcoin", receive + "#wrong"), ("bitcoin", " \n" + receive),
                          ("liquid", f"ct(slip77({'03' * 32}),elwpkh({key}/0/*))#wrong"))]
    previews = []
    for case in materials:
        args = {"wallet_material": case["material"], **case["options"], "count": 2}
        previews.append({"name": "material-" + case["name"], "args": args,
                         "expected": capture(lambda args=args: _preview_descriptor_payload(args))})
    for case in plans:
        if case["config"].get("descriptor"):
            args = {k: v for k, v in case["config"].items()
                    if k in {"descriptor", "change_descriptor", "chain", "network"}}
            args["count"] = 2
            previews.append({"name": "plan-" + case["name"], "args": args,
                             "expected": capture(lambda args=args: _preview_descriptor_payload(args))})
    for count in (None, True, False, -1, 0, 1, 2.0, "2", 21, 10**100):
        args = {"descriptor": receive, "count": count}
        previews.append({"name": "count-" + str(count), "args": args,
                         "expected": capture(lambda args=args: _preview_descriptor_payload(args))})
    branch_selections = []
    for config in ({"descriptor": receive}, {"xpub": xpub, "script_types": ["p2wpkh", "p2tr"]}):
        for branch in (None, "", "all", " all ", "receive", "external", "internal", "change",
                       "p2tr receive", "P2TR-CHANGE", "0", "4", "6", "99", "\u0664", "unknown"):
            branch_selections.append({"config": config, "branch": branch,
                "expected": capture(lambda c=config, b=branch: resolve_descriptor_branch_index(load_descriptor_plan(c), b))})
    return {"format": 1, "reference_commit": REFERENCE,
            "generator": "crates/wallets/scripts/generate_vectors.py",
            "embit_version": importlib.metadata.version("embit"),
            "data_policy": "Synthetic deterministic test keys only; generated without books or network.",
            "plans": plans, "materials": materials, "configs": configs, "normalizations": normalizations,
            "previews": previews, "branch_selections": branch_selections}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    # JSON round-trip makes tuples in dataclasses portable arrays.
    text = json.dumps(vectors(), indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit("Python descriptor vectors differ from the accepted baseline")
    else:
        DEST.parent.mkdir(parents=True, exist_ok=True)
        DEST.write_text(text, encoding="utf-8")
    payload = json.loads(text)
    print(
        f"Verified {len(payload['plans'])} plans, {len(payload['materials'])} material inputs, "
        f"{len(payload['configs'])} wallet configs, {len(payload['normalizations'])} normalizations, "
        f"{len(payload['previews'])} previews, {len(payload['branch_selections'])} branch selections"
    )


if __name__ == "__main__":
    main()

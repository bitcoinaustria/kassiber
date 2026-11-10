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
import sqlite3
from pathlib import Path

from embit import bip32, ec
from embit.base import EmbitError
from embit.descriptor.checksum import add_checksum

from kassiber.core.ownership_policy_epochs import private_policy_material, policy_identity_material
from kassiber.core.wallets import (
    WALLET_KINDS, _validated_wallet_config, redact_wallet_config_for_output,
)
from kassiber.errors import AppError
from kassiber.wallet_descriptors import (
    branch_descriptor, branch_limits, derive_descriptor_targets,
    liquid_blinding_secret, liquid_plan_can_unblind, load_descriptor_plan, normalize_descriptor_text,
)
from kassiber.wallet_setup import normalize_wallet_material
from kassiber.core.ui_snapshot import build_wallets_list_snapshot
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION
from kassiber.daemon import _preview_descriptor_payload
from kassiber.cli.handlers import resolve_descriptor_branch_index

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_descriptors.json"
REFERENCE = "79de6256"


def capture(call, *, embit_errors=True):
    try:
        try:
            return {"ok": call()}
        except EmbitError as exc:
            if not embit_errors:
                # The caller lets it escape: the daemon answers internal_error.
                return {"crash": {"type": type(exc).__name__, "message": str(exc)}}
            # `load_wallet_descriptor_plan_from_config`: embit's own errors
            # surface without their parser text.
            raise AppError("Invalid wallet descriptor", code="validation") from exc
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
    # Legacy `{a,b}` branch sets on the spending side.
    plan("legacy-set-spend", {"descriptor": f"wpkh({key}/{{0,1}}/*)"})
    plan("legacy-set-spend-liquid", {"descriptor": f"ct(slip77({view}),elwpkh({key}/{{0,1}}/*))", "chain": "liquid"})
    plan("legacy-set-spend-multi", {"descriptor": f"wsh(sortedmulti(2,{key}/{{0,1}}/*,{second}/<0;1>/*))"})

    # Non-SLIP-77 view keys: embit tweaks the (derived) view key with
    # tagged_hash("elements/blindingkey", script), not ELIP-150's tweak.
    view_root = bip32.HDKey.from_seed(bytes([3]) * 32)
    view_xprv, view_xpub = view_root.to_base58(), view_root.to_public().to_base58()
    view_tprv = view_root.to_base58(version=bytes.fromhex("04358394"))
    view_origin = f"[{view_root.my_fingerprint.hex()}/7h]"
    view_pub = view_root.key.get_public_key().sec().hex()
    uncompressed_view = ec.PrivateKey(bytes([3]) * 32, compressed=False)
    uncompressed_point = ec.PublicKey.parse(view_root.key.get_public_key().sec())
    uncompressed_point.compressed = False
    uncompressed_pub = uncompressed_point.sec().hex()
    view_keys = []
    for name, descriptor, extra in (
        ("multipath", f"ct({view_xprv}/<0;1>/*,elwpkh({key}/<0;1>/*))", {}),
        ("multipath-regtest", f"ct({view_tprv}/<0;1>/*,elwpkh({key}/<0;1>/*))", {"network": "elementsregtest"}),
        ("multipath-testnet", f"ct({view_tprv}/<0;1>/*,elwpkh({key}/<0;1>/*))", {"network": "liquidtestnet"}),
        ("wildcard-receive", f"ct({view_xprv}/*,elwpkh({key}/0/*))", {}),
        ("wildcard-path", f"ct({view_xprv}/0/*,elwpkh({key}/0/*))", {}),
        ("pair", f"ct({view_xprv}/0/*,elwpkh({key}/0/*))",
         {"change_descriptor": f"ct({view_xprv}/1/*,elwpkh({key}/1/*))"}),
        ("origin-hardened", f"ct({view_origin}{view_xprv}/7h/<0;1>/*,elwpkh({key}/<0;1>/*))", {}),
        ("apostrophe", f"ct({view_xprv}/7'/<0;1>/*,elwpkh({key}/<0;1>/*))", {}),
        ("sh", f"ct({view_xprv}/<0;1>/*,elsh(wpkh({key}/<0;1>/*)))", {}),
        ("tr", f"ct({view_xprv}/<0;1>/*,eltr({key}/<0;1>/*))", {}),
        ("wsh-multi", f"ct({view_xprv}/<0;1>/*,elwsh(sortedmulti(2,{key}/<0;1>/*,{second}/<0;1>/*)))", {}),
        ("public-xpub", f"ct({view_xpub}/<0;1>/*,elwpkh({key}/<0;1>/*))", {}),
        ("wif-static", f"ct({view},elwpkh({pubkey}))", {}),
        ("wif-fixed", f"ct({view},elwpkh({key}/0/7))", {}),
        ("xprv-fixed", f"ct({view_xprv}/0/7,elwpkh({key}/0/7))", {}),
        ("xprv-bare-fixed", f"ct({view_xprv},elwpkh({key}/0/7))", {}),
        ("public-static", f"ct({view_pub},elwpkh({pubkey}))", {}),
        ("checksum", add_checksum(f"ct({view_xprv}/<0;1>/*,elwpkh({key}/<0;1>/*))"), {}),
        ("wif-ranged", f"ct({view},elwpkh({key}/<0;1>/*))", {}),
        ("wildcard-multipath-spend", f"ct({view_xprv}/*,elwpkh({key}/<0;1>/*))", {}),
        ("three-branches", f"ct({view_xprv}/<0;1;2>/*,elwpkh({key}/<0;1>/*))", {}),
        ("hex-private", f"ct({'03' * 32},elwpkh({key}/<0;1>/*))", {}),
        ("hardened-wildcard", f"ct({view_xprv}/<0;1>/*h,elwpkh({key}/<0;1>/*))", {}),
        ("ranged-static-spend", f"ct({view_xprv}/<0;1>/*,elwpkh({pubkey}))", {}),
        # Uncompressed keys: embit still blinds with a compressed key.
        ("uncompressed-wif", f"ct({uncompressed_view.wif()},elwpkh({pubkey}))", {}),
        ("uncompressed-wif-fixed", f"ct({uncompressed_view.wif()},elwpkh({key}/0/7))", {}),
        ("uncompressed-public", f"ct({uncompressed_pub},elwpkh({pubkey}))", {}),
        # Legacy `{a,b}` branch sets, which embit spells `<a;b>`.
        ("legacy-set-view", f"ct({view_xprv}/{{0,1}}/*,elwpkh({key}/<0;1>/*))", {}),
        ("legacy-set-both", f"ct({view_xprv}/{{0,1}}/*,elwpkh({key}/{{0,1}}/*))", {}),
        ("legacy-set-hardened", f"ct({view_xprv}/{{0h,1h}}/*,elwpkh({key}/<0;1>/*))", {}),
        ("legacy-set-three", f"ct({view_xprv}/{{0,1,2}}/*,elwpkh({key}/<0;1>/*))", {}),
        ("legacy-set-public", f"ct({view_xpub}/{{0,1}}/*,elwpkh({key}/<0;1>/*))", {}),
    ):
        value = {"descriptor": descriptor, "chain": "liquid", **extra}
        plan("liquid-view-" + name, value)
        view_keys.append({"name": name, "config": value})
    for name in ("multipath", "public-xpub", "wif-static"):
        value = {**next(v["config"] for v in view_keys if v["name"] == name), "backend": "local"}
        config("liquid-view-" + name, "descriptor", value)

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
    for idx, descriptor in enumerate([
        "wpkh(", "wpkh()", "wpkh(invalid)", "WPKH(invalid)", "wpkh(invalid", "wpkh(0)",
        "wsh()", "sh()", "tr()", "pkh()", "raw(00)", "addr(invalid)",
        "wsh(sortedmulti(1,invalid))", "sh(multi(1,invalid))",
        f"wpkh({pubkey})garbage", f"wpkh({pubkey}", f"wpkh({pubkey}))", f"sh(wpkh({pubkey})",
        f"wsh(sortedmulti(0,{pubkey}))", f"wsh(sortedmulti(2,{pubkey}))",
        f"wpkh([123]{xpub}/0/*)", f"wpkh({xpub}/0h/*)", f"wpkh({xpub}/**)",
        f"wpkh({xpub}/<0;1;2>/*)",
        f"wpkh({xpub}/0/-1)", f"wpkh({xpub}/0/2147483648)",
        "wpkh(" + "02" + "00"*32 + ")",
        "wpkh(" + "01" + "11"*32 + ")",
        "wpkh(" + "0x" + "11"*32 + ")",
    ]):
        config(f"malformed-descriptor-{idx}", "descriptor", {"descriptor": descriptor})
    # User timestamps use CPython 3.11 after time_utils' trailing-Z rewrite.
    birthdays = [None, "", " ", "2025-01-02", "20250102", "2025-W01-4", "2025W014",
                 "2025-01-02Z", "20250102Z", "2025-01-02T01:02:03.999999Z",
                 "2025-01-02T01:02:03-02:30", "2025-01-02T01:02:03+00:00:30.5",
                 "2025-01-02T01:02:03+00:00:00.9", "0001-01-01", "9999-12-31",
                 "2025-02-29", "2024-02-29", "2025-01-01T24:00:00Z", "junk", False, 0]
    for clock in ("12", "12:00", "12:00:00", "1200", "120000", "12:00:00.1"):
        for separator in (" ", "\t", "X", ":", ".", ",", "  ", "\u00a0"):
            birthdays.append("2025-01-02T" + clock + separator + "+01:00")
    for idx, birthday in enumerate(birthdays):
        config(f"birthday-{idx}", "custom", {"birthday": birthday})
    config("birthday-before-deprecated-error", "custom", {"birthday": "junk", "deprecated": "junk"})
    sp = "sp(spscan1q" + "q" * 40 + ")"
    sp_base = {"sp_descriptor": sp, "backend": "local", "sp_scan_start_height": 850000}
    for name, changes in [
        ("default", {}), ("uppercase-checksum", {"sp_descriptor": sp.upper() + "#ignored"}),
        ("compact", {"sp_descriptor": " \n SP( spscan1q" + "q" * 40 + " ) #ignored"}),
        ("two-key", {"sp_descriptor": f"sp({private},{pubkey})"}),
        ("two-key-origin", {"sp_descriptor": f"sp([12345678/0h]{private},[12345678]{xpub})"}),
        ("scan-wif", {"sp_descriptor": f"sp({accounts[0].key.wif()},{pubkey})"}),
        ("scan-uncompressed", {"sp_descriptor": f"sp({'5'+'1'*30},{pubkey})"}),
        ("spend-private", {"sp_descriptor": f"sp({private},{private})"}),
        ("spend-private-nested", {"sp_descriptor": f"sp({private},musig({private},{pubkey}))"}),
        ("spspend", {"sp_descriptor": "sp(spspend1q" + "q" * 40 + ")"}),
        ("scan-public", {"sp_descriptor": f"sp({xpub},{pubkey})"}),
        ("bad-spend", {"sp_descriptor": f"sp({private},bad)"}),
        ("missing", {"sp_descriptor": ""}), ("wrong-hrp", {"network": "regtest"}),
        ("testnet", {"network": "signet", "sp_descriptor": sp.replace("spscan", "tspscan")}),
        ("wrong-scan-family", {"network": "test", "sp_descriptor": f"sp({private},{pubkey})"}),
        ("wrong-spend-family", {"sp_descriptor": f"sp({private},{tpub})"}),
        ("wrong-chain", {"chain": "liquid"}), ("no-backend", {"backend": ""}),
        ("height-string", {"sp_scan_start_height": " 8_500 "}),
        ("height-zero", {"sp_scan_start_height": 0}),
        ("height-huge", {"sp_scan_start_height": 10**100}),
        ("height-negative", {"sp_scan_start_height": -1}),
        ("height-boolean", {"sp_scan_start_height": True}),
        ("height-float", {"sp_scan_start_height": 2.5}),
        ("no-start", {"sp_scan_start_height": None}),
        ("date-original", {"sp_scan_start_height": None, "sp_scan_start_date": "2025-01-02T01:02:03.456+02:30"}),
        ("date-Z-rewrite", {"sp_scan_start_date": "2025-01-02Z"}),
        ("date-whitespace-offset", {"sp_scan_start_date": " 2025-01-02T12:00:00 +01:00 "}),
        ("date-invalid", {"sp_scan_start_date": "junk"}),
        ("full-unack", {"sp_full_history": True}),
        ("full-ack", {"sp_full_history": "yes", "sp_acknowledge_full_history_warning": True}),
        ("full-date-still-validated", {"sp_full_history": True, "sp_scan_start_date": "junk"}),
        ("server-unack", {"sp_scan_mode": "server-assisted"}),
        ("server-ack", {"sp_scan_mode": "SERVER_ASSISTED_SCAN", "sp_acknowledge_server_warning": True}),
        ("local-alias", {"sp_scan_mode": "local-scanner"}),
        ("bad-mode", {"sp_scan_mode": "invalid"}),
        ("bad-full-bool", {"sp_full_history": "invalid"}),
        ("safe-extra", {"sp_scan_start_kind": "height", "unknown_secret": "hidden"}),
    ]:
        config("sp-" + name, "silent-payment", {**sp_base, **changes})
    for field, material in (("descriptor", f"wpkh({private}/0/*)"), ("change_descriptor", f"wpkh({private}/1/*)"), ("xpub", private)):
        config("sp-inactive-private-" + field, "silent-payment", {**sp_base, field: material})
    for idx, descriptor in enumerate(["", "sp()", "sp(,)", "sp(q)", "wpkh(q)", "sp(q,q,q)",
                                      "sp([q)", "sp(q])", "sp((q)", "sp(q))", "sp(spscan1q" + "q" * 8 + ",)"]):
        config(f"sp-malformed-{idx}", "silent-payment", {**sp_base, "sp_descriptor": descriptor})
    snapshots = []
    creation = json.loads((DEST.parents[3] / "store/fixtures/creation.json").read_text())["statements"]
    def snapshot_case(name, *, current=False, excluded=False, context=True, default="local", invalid=False, version_changed=False):
        statements = [
            "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01')",
            "INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01')",
            "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('a','ws','p','treasury','Treasury','asset','2025-01-01')",
        ]
        def sql_value(value):
            return "'" + value.replace("'", "''") + "'"
        values = [
            ("descriptor", {"descriptor": receive, "birthday": "2025-01-02T00:00:00Z", "last_synced_at": " 2025-01-03T00:00:00Z "}),
            ("xpub", {"xpub": xpub, "script_types": ["p2wpkh", "p2tr", 3], "backend": "missing"}),
            ("address", {"addresses": ["synthetic"], "deprecated": "yes"}),
            ("silent-payment", {**sp_base, "sp_material_format": "bip392-spscan", "sp_scan_mode": "local_index", "sp_full_history": False}),
            ("custom", {"source_file": "/synthetic.csv", "source_format": "csv", "backend": "explicit"}),
            ("custom", {"sync_source": "btcpay", "source_file": "/synthetic.csv", "source_format": "csv", "backend": "explicit", "btcpay_provenance": [{"backend":"LOCAL", "store_id":" store ", "payment_method_id":"btc-ln"}]}),
            ("samourai", {"samourai": {"role": " parent ", "group_label": "a"*150, "watch_only": "yes", "mix_count": "2", "minimum_mix_count": -1, "mix_count_confidence": "EXACT", "privacy_event": "invalid", "sections": ["DEPOSIT", "invalid"], "round_txids": ["AB"*32, "bad"], "descriptor": receive, "unknown": "secret"}}),
            ("custom", {"script_types": {"p2tr": True, "p2wpkh": False}}),
        ]
        for i, (kind, value) in enumerate(values):
            raw = "malformed" if invalid and i == 0 else json.dumps(value, sort_keys=True)
            statements.append("INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at) VALUES(" + ",".join(sql_value(v) for v in (str(i),'ws','p','a',f'Wallet {i}',kind,raw,'2025-01-01T00:00:00Z')) + ")")
        for i in (0,1):
            statements.append(f"INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,excluded,created_at) VALUES('tx{i}','ws','p','{i}','fp{i}','2025-01-02T00:00:00Z','inbound','BTC',1000,{int(excluded)},'2025-01-01T00:00:00Z')")
        if context:
            statements += ["INSERT INTO settings(key,value) VALUES('context_workspace','ws')", "INSERT INTO settings(key,value) VALUES('context_profile','p')"]
        if current:
            statements.append("UPDATE profiles SET last_processed_at='2025-01-03',last_processed_tx_count=2,last_processed_input_version=journal_input_version")
            statements.append(f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION}")
        if version_changed:
            statements.append("UPDATE profiles SET journal_input_version=journal_input_version+1")
        runtime = {"default_backend": default, "backends": {"local":{"kind":"custom"}, "explicit":{"kind":"btcpay"}}}
        with sqlite3.connect(":memory:") as conn:
            conn.row_factory = sqlite3.Row
            for sql in [*creation, *statements]:
                conn.execute(sql)
            expected = capture(lambda: build_wallets_list_snapshot(conn, runtime))
        snapshots.append({"name":name,"statements":statements,"runtime":runtime,"expected":expected})
    snapshot_case("stale")
    snapshot_case("current", current=True)
    snapshot_case("stale-input-version", current=True, version_changed=True)
    snapshot_case("excluded-only", excluded=True)
    snapshot_case("no-context", context=False)
    snapshot_case("no-default", default=None)
    snapshot_case("malformed-legacy-config", invalid=True)
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
                             "expected": capture(lambda args=args: _preview_descriptor_payload(args),
                                                 embit_errors=False)})
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
    # `liquid_blinding_secret` per derived target: the key the compatibility
    # observers unblind owned outputs with.
    blinding_secrets = []
    slip77_cases = [
        {"name": "slip77", "config": {"descriptor": f"ct(slip77({view}),elwpkh({key}/<0;1>/*))", "chain": "liquid"}},
        # A fixed suffix: embit tweaks with the account key's script there.
        {"name": "slip77-fixed", "config": {"descriptor": f"ct(slip77({view}),elwpkh({key}/0/7))", "chain": "liquid"}},
    ]
    for case in [*slip77_cases, *view_keys]:
        try:
            loaded = load_descriptor_plan(case["config"])
        except (ValueError, TypeError, AppError, EmbitError):
            continue
        secrets = []
        for branch in loaded.branches:
            for index in range(3):
                def secret(b=branch.branch_index, i=index):
                    value, _ = liquid_blinding_secret(loaded, b, i)
                    return value.hex()
                secrets.append({"branch_index": branch.branch_index, "address_index": index,
                                "expected": capture(secret)})
        blinding_secrets.append({"name": case["name"], "config": case["config"], "secrets": secrets})
    return {"format": 1, "reference_commit": REFERENCE,
            "generator": "crates/wallets/scripts/generate_vectors.py",
            "embit_version": importlib.metadata.version("embit"),
            "data_policy": "Synthetic deterministic test keys only; generated without books or network.",
            "snapshots": snapshots, "plans": plans, "materials": materials, "configs": configs, "normalizations": normalizations,
            "previews": previews, "branch_selections": branch_selections,
            "blinding_secrets": blinding_secrets}


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
        f"{len(payload['previews'])} previews, {len(payload['branch_selections'])} branch selections, "
        f"{len(payload['snapshots'])} desktop snapshots"
    )


if __name__ == "__main__":
    main()

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any
from urllib import error, request

DEFAULT_API_URL = "http://127.0.0.1:9001"
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCENARIO = ROOT / "dev" / "regtest" / "scenarios" / "full_accounting.json"


class BoltzProbeError(RuntimeError):
    pass


def _api_url(value: str | None = None) -> str:
    return str(value or os.environ.get("KASSIBER_BOLTZ_API_URL") or DEFAULT_API_URL).rstrip("/")


def _get_json(api_url: str, path: str) -> dict[str, Any]:
    url = f"{api_url}{path}"
    try:
        with request.urlopen(url, timeout=10) as response:
            payload = response.read().decode("utf-8")
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise BoltzProbeError(f"Boltz API {path} failed with HTTP {exc.code}: {body}") from exc
    except OSError as exc:
        raise BoltzProbeError(f"Boltz API {path} is not reachable at {api_url}: {exc}") from exc
    try:
        decoded = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise BoltzProbeError(f"Boltz API {path} returned non-JSON: {payload!r}") from exc
    if not isinstance(decoded, dict):
        raise BoltzProbeError(f"Boltz API {path} returned {type(decoded).__name__}, expected object")
    return decoded


def _pair(pairs: dict[str, Any], from_asset: str, to_asset: str) -> dict[str, Any]:
    raw = pairs.get(from_asset, {})
    if not isinstance(raw, dict):
        return {}
    pair = raw.get(to_asset, {})
    return pair if isinstance(pair, dict) else {}


def _summarize_pair(pair: dict[str, Any]) -> dict[str, Any]:
    limits = pair.get("limits") if isinstance(pair.get("limits"), dict) else {}
    fees = pair.get("fees") if isinstance(pair.get("fees"), dict) else {}
    return {
        "hash": pair.get("hash"),
        "rate": pair.get("rate"),
        "limits": {
            "minimal": limits.get("minimal"),
            "maximal": limits.get("maximal"),
            "maximal_zero_conf": limits.get("maximalZeroConf"),
        },
        "fees": fees,
    }


def _require_pair(pairs: dict[str, Any], from_asset: str, to_asset: str, label: str) -> dict[str, Any]:
    pair = _pair(pairs, from_asset, to_asset)
    if not pair:
        raise BoltzProbeError(f"Boltz {label} pairs do not include {from_asset} -> {to_asset}")
    if not pair.get("hash"):
        raise BoltzProbeError(f"Boltz {label} pair {from_asset} -> {to_asset} has no pair hash")
    return _summarize_pair(pair)


def probe_boltz_liquid(api_url: str | None = None) -> dict[str, Any]:
    """Probe the local Boltz regtest API for Liquid-capable swap pairs.

    This deliberately stops at pair and chain metadata. Full swap execution is
    delegated to Boltz's official clients because they own the Taproot/MuSig
    state machine and recovery edge cases.
    """

    base = _api_url(api_url)
    version = _get_json(base, "/version")
    submarine_pairs = _get_json(base, "/v2/swap/submarine")
    reverse_pairs = _get_json(base, "/v2/swap/reverse")
    chain_pairs = _get_json(base, "/v2/swap/chain")
    liquid_height = _get_json(base, "/v2/chain/L-BTC/height")
    bitcoin_height = _get_json(base, "/v2/chain/BTC/height")

    return {
        "api_url": base,
        "version": version.get("version"),
        "heights": {
            "BTC": bitcoin_height.get("height"),
            "L-BTC": liquid_height.get("height"),
        },
        "pairs": {
            "liquid_to_lightning": _require_pair(submarine_pairs, "L-BTC", "BTC", "submarine"),
            "lightning_to_liquid": _require_pair(reverse_pairs, "BTC", "L-BTC", "reverse"),
            "bitcoin_to_liquid": _require_pair(chain_pairs, "BTC", "L-BTC", "chain"),
        },
    }


def boltz_bridge_specs(scenario: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        dict(bridge)
        for bridge in (scenario.get("stress") or {}).get("swap_bridges") or []
        if str(bridge.get("provider") or "").strip().lower() == "boltz"
    ]


def load_demo_scenario_metadata(path: Path = DEFAULT_SCENARIO) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def verify_demo_boltz_coverage(
    probe: dict[str, Any],
    scenario: dict[str, Any] | None = None,
) -> list[dict[str, Any]]:
    if scenario is None:
        scenario = load_demo_scenario_metadata()
    bridges = boltz_bridge_specs(scenario)
    if not bridges:
        raise BoltzProbeError("The full-accounting scenario has no provider=boltz swap bridge")

    available = {
        ("chain-swap", "BTC", "L-BTC"): probe.get("pairs", {}).get("bitcoin_to_liquid"),
        ("submarine-swap", "L-BTC", "BTC"): probe.get("pairs", {}).get("liquid_to_lightning"),
        ("reverse-submarine-swap", "BTC", "L-BTC"): probe.get("pairs", {}).get("lightning_to_liquid"),
    }
    covered = []
    for bridge in bridges:
        key = (
            str(bridge.get("boltz_flow") or "").strip(),
            str(bridge.get("boltz_from") or "").strip(),
            str(bridge.get("boltz_to") or "").strip(),
        )
        if key not in available or not available[key]:
            raise BoltzProbeError(f"Boltz demo bridge {bridge.get('id')} is not covered by live pair metadata: {key}")
        covered.append(
            {
                "id": bridge.get("id"),
                "flow": key[0],
                "from": key[1],
                "to": key[2],
                "pair_hash": available[key].get("hash"),
            }
        )
    return covered


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Probe a local Boltz regtest API for Liquid swap coverage.")
    parser.add_argument("--api-url", default=None, help=f"Boltz API URL, default {DEFAULT_API_URL}")
    parser.add_argument("--json", action="store_true", help="Print the full JSON summary.")
    args = parser.parse_args(argv)

    probe = probe_boltz_liquid(args.api_url)
    covered = verify_demo_boltz_coverage(probe)
    summary = {"probe": probe, "demo_coverage": covered}
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(
            "Boltz Liquid regtest ready: "
            f"version={probe.get('version')} "
            f"btc_height={probe['heights'].get('BTC')} "
            f"lbtc_height={probe['heights'].get('L-BTC')} "
            f"demo_bridges={len(covered)}"
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BoltzProbeError as exc:
        print(str(exc), file=sys.stderr)
        raise SystemExit(1)

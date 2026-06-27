/**
 * Lightweight client-side detection for what a user pasted into the
 * "wallet export" textarea. Inline feedback so they can spot a wrong
 * paste before submitting the form.
 *
 * Does not validate cryptographic correctness; the daemon does that on
 * the actual create/preview call.
 */

export type WalletMaterialKind =
  | "descriptor-json"
  | "descriptor"
  | "slip132"
  | "bare-xpub"
  | "empty"
  | "unknown";

export interface WalletMaterialDetection {
  kind: WalletMaterialKind;
  label: string;
  hint?: string;
}

/**
 * Script types we can wrap a bare xpub/tpub into when the user disambiguates it.
 * Ordered modern-first. Mirrors `BARE_XPUB_TEMPLATES` in kassiber/wallet_setup.py.
 */
export type BareXpubScriptType = "p2wpkh" | "p2sh-p2wpkh" | "p2pkh" | "p2tr";

export const BARE_XPUB_SCRIPT_TYPES: BareXpubScriptType[] = [
  "p2wpkh",
  "p2sh-p2wpkh",
  "p2pkh",
  "p2tr",
];

export interface BareXpubScriptTypeDetectionPayload {
  probed?: boolean;
  active?: unknown;
  reason?: string | null;
}

export type BareXpubScriptTypeSelection =
  | { ok: true; scriptTypes: BareXpubScriptType[] }
  | { ok: false; reason: string | null };

export function scriptTypesFromDetectionPayload(
  payload: BareXpubScriptTypeDetectionPayload | null | undefined,
): BareXpubScriptTypeSelection {
  if (!payload?.probed) {
    return {
      ok: false,
      reason:
        typeof payload?.reason === "string" && payload.reason.trim()
          ? payload.reason.trim()
          : null,
    };
  }
  const active = Array.isArray(payload.active)
    ? payload.active.filter(isBareXpubScriptType)
    : [];
  return {
    ok: true,
    scriptTypes: active.length > 0 ? active : [BARE_XPUB_SCRIPT_TYPES[0]],
  };
}

const DESCRIPTOR_PREFIXES = [
  "pkh(",
  "wpkh(",
  "sh(",
  "wsh(",
  "tr(",
  "combo(",
  "addr(",
  "raw(",
  "ct(",
  "elwpkh(",
  "elwsh(",
  "elsh(",
  "eltr(",
];

const SLIP132_PREFIXES: Record<string, string> = {
  ypub: "P2SH-wrapped SegWit (BIP49)",
  zpub: "Native SegWit (BIP84)",
  upub: "Testnet P2SH-wrapped SegWit",
  vpub: "Testnet Native SegWit",
};

function isBareXpubScriptType(value: unknown): value is BareXpubScriptType {
  return (
    typeof value === "string" &&
    BARE_XPUB_SCRIPT_TYPES.includes(value as BareXpubScriptType)
  );
}

export function detectWalletMaterial(value: string): WalletMaterialDetection {
  const trimmed = value.trim();
  if (!trimmed) {
    return { kind: "empty", label: "Empty" };
  }
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    return {
      kind: "descriptor-json",
      label: "Descriptor JSON export",
    };
  }
  const lower = trimmed.toLowerCase();
  if (DESCRIPTOR_PREFIXES.some((prefix) => lower.startsWith(prefix))) {
    return {
      kind: "descriptor",
      label: "Output descriptor",
    };
  }
  const head = trimmed.slice(0, 4);
  const slip132Label = SLIP132_PREFIXES[head];
  if (slip132Label) {
    return {
      kind: "slip132",
      label: `${head} · ${slip132Label}`,
    };
  }
  if (head === "xpub" || head === "tpub") {
    return {
      kind: "bare-xpub",
      label: `Bare ${head}`,
      hint: "Kassiber auto-detects which address types this key uses; pick one to pin it.",
    };
  }
  return {
    kind: "unknown",
    label: "Unrecognized format",
    hint: "Paste a descriptor, Bitcoin Core descriptor JSON, or a ypub/zpub/upub/vpub key.",
  };
}

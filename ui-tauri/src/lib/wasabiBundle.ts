export type WasabiImportMode = "rpc" | "bundle-file";

export interface WasabiBundleFields {
  history: string;
  coins: string;
  walletInfo: string;
  additional: string;
}

export interface WasabiBundleBuildResult {
  bundle: Record<string, unknown>;
  errors: Partial<Record<keyof WasabiBundleFields, string>>;
}

function parseJsonField(
  value: string,
  field: keyof WasabiBundleFields,
  errors: Partial<Record<keyof WasabiBundleFields, string>>,
) {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    errors[field] = "Paste valid JSON from Wasabi RPC.";
    return undefined;
  }
}

export function buildWasabiBundle(
  fields: WasabiBundleFields,
): WasabiBundleBuildResult {
  const errors: Partial<Record<keyof WasabiBundleFields, string>> = {};
  const bundle: Record<string, unknown> = {};
  const additional = parseJsonField(fields.additional, "additional", errors);
  if (additional !== undefined) {
    if (
      !additional ||
      typeof additional !== "object" ||
      Array.isArray(additional)
    ) {
      errors.additional =
        "Additional Wasabi sections must be a JSON object.";
    } else {
      Object.assign(bundle, additional);
    }
  }

  const history = parseJsonField(fields.history, "history", errors);
  if (history === undefined && !errors.history) {
    errors.history = "Paste the Wasabi gethistory JSON response.";
  } else if (history !== undefined) {
    bundle.gethistory = history;
  }

  const coins = parseJsonField(fields.coins, "coins", errors);
  if (coins !== undefined) {
    bundle.listcoins = coins;
  }

  const walletInfo = parseJsonField(fields.walletInfo, "walletInfo", errors);
  if (walletInfo !== undefined) {
    bundle.getwalletinfo = walletInfo;
  }

  return { bundle, errors };
}

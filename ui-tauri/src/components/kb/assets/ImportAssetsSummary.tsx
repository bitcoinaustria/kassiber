import { AssetScopeNotice } from "./AssetScopeNotice";
import { readImportAssets, type AssetScope } from "./importAssets";

/**
 * The arguments of the file import the user just ran, kept so that turning
 * other crypto assets on can import the same file again. Only a path the user
 * chose for this import is kept; without one the user picks the file again.
 */
export interface ImportReplay {
  wallet: string;
  source_format: string;
  source_file: string;
}

/**
 * The asset scope part of a provider import's result
 * (docs/plan/24-crypto-assets.md): the found-assets prompt, then the same
 * file imported again. Re-importing is idempotent; rows deduplicate by
 * external id.
 */
export function ImportAssetsSummary({
  wallet,
  assets,
  swapPairs,
  replay,
  disabled,
  onScopeStored,
  onReimport,
}: {
  wallet: string | null | undefined;
  assets: unknown;
  swapPairs?: number | null;
  replay: ImportReplay | null;
  disabled?: boolean;
  /** The scope is stored and nothing was imported again. */
  onScopeStored: (scope: AssetScope) => void;
  /** Import `replay` again; the caller shows the new result. */
  onReimport: (replay: ImportReplay) => Promise<void>;
}) {
  const parsed = readImportAssets(assets);
  const target = replay?.wallet ?? wallet;
  if (!target || (!parsed && !(swapPairs && swapPairs > 0))) return null;
  const usableReplay = replay && replay.source_file.trim() ? replay : null;
  return (
    <AssetScopeNotice
      wallet={target}
      scope={parsed?.scope ?? "bitcoin"}
      assets={parsed}
      swapPairs={swapPairs}
      rerun={usableReplay ? "import" : "pick_file"}
      disabled={disabled}
      onScopeChanged={async (scope) => {
        if (scope === "crypto" && usableReplay) {
          await onReimport(usableReplay);
        } else {
          onScopeStored(scope);
        }
      }}
    />
  );
}

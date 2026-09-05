export type CustodySurfaceTab = "review" | "gaps" | "components";
export type CustodyReviewMode = "transfers" | "swaps";
export type CustodyReviewView = "review" | "paired";

export interface TransfersCustodySearch {
  tab?: CustodySurfaceTab;
  mode?: CustodyReviewMode;
  view?: CustodyReviewView;
  focus?: string;
  method?: "ownership_graph";
  gap?: string;
}

const SAFE_ENTITY_ID = /^[A-Za-z0-9][A-Za-z0-9._:@+-]{0,255}$/;
function entityId(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return SAFE_ENTITY_ID.test(trimmed) ? trimmed : undefined;
}

/** Only this bounded navigation state may enter the screen or its AI context. */
export function parseTransfersCustodySearch(raw: Record<string, unknown>): TransfersCustodySearch {
  const focus = [raw.focus, raw.tx, raw.transaction].map(entityId).find(Boolean);
  const method = raw.method === "ownership_graph" ? raw.method : undefined;
  if (focus || method) {
    return { tab: "review", mode: "transfers", view: "review", ...(focus ? { focus } : {}), ...(method ? { method } : {}) };
  }
  const gap = [raw.gap, raw.gap_id].map(entityId).find(Boolean);
  return {
    ...(raw.tab === "review" || raw.tab === "gaps" || raw.tab === "components" ? { tab: raw.tab } : {}),
    ...(raw.mode === "transfers" || raw.mode === "swaps" ? { mode: raw.mode } : {}),
    ...(raw.view === "review" || raw.view === "paired" ? { view: raw.view } : {}),
    ...(gap ? { gap } : {}),
  };
}

export function custodyGapRedirectSearch(raw: Record<string, unknown>): TransfersCustodySearch {
  const gap = [raw.gap, raw.gap_id].map(entityId).find(Boolean);
  return { tab: "gaps", ...(gap ? { gap } : {}) };
}

export function transfersCustodyView(raw: Record<string, unknown>, developerSurfacesEnabled: boolean) {
  const search = parseTransfersCustodySearch(raw);
  if (!developerSurfacesEnabled && (search.tab === "gaps" || search.tab === "components")) {
    return { tab: "review" as const, mode: "transfers" as const, view: "review" as const };
  }
  return { ...search, tab: search.tab ?? "review", mode: search.mode ?? "transfers", view: search.view ?? "review" };
}

/**
 * Navigation state of the Transfers screen (`/transfers`).
 *
 * The screen is one page: what waits on the owner, the suggestions, moves
 * they unlinked recently and every linked move. Two things open on top of it
 * from the URL so other screens can send the owner there: "Link manually"
 * (`link=<transaction id>`, or `link=new`) and a saved view of the
 * suggestions (`savedView=<id>`). The developer-gated advanced tools are the
 * one secondary section (`section=advanced`: missing wallets, custody
 * components, scan coverage).
 *
 * Links from before the redesign keep working: a focused candidate
 * (`focus=`, `/swaps?tab=review&focus=`) opens "Link manually" on that
 * transaction, the old matching tools open the page, a gap or component tab
 * opens the advanced tools.
 */
export type TransfersSection = "overview" | "advanced";
export type CustodySurfaceTab = "gaps" | "components" | "coverage";

export interface TransfersCustodySearch {
  section?: "advanced";
  tab?: CustodySurfaceTab;
  gap?: string;
  /** Open "Link manually": on this transaction, or `new` to pick both sides. */
  link?: string;
  /** Show the suggestions one saved view selects. */
  savedView?: string;
}

export interface TransfersCustodyView {
  section: TransfersSection;
  tab: CustodySurfaceTab;
  gap?: string;
  link?: string;
  savedView?: string;
}

/** `link=new`: "Link manually" with nothing picked yet. */
export const LINK_NEW = "new";

const SAFE_ENTITY_ID = /^[A-Za-z0-9][A-Za-z0-9._:@+-]{0,255}$/;
function entityId(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return SAFE_ENTITY_ID.test(trimmed) ? trimmed : undefined;
}

function advancedTab(value: unknown): CustodySurfaceTab | undefined {
  return value === "gaps" || value === "components" || value === "coverage" ? value : undefined;
}

/** Only this bounded navigation state may enter the screen or its AI context. */
export function parseTransfersCustodySearch(raw: Record<string, unknown>): TransfersCustodySearch {
  const link = [raw.link, raw.focus, raw.tx, raw.transaction].map(entityId).find(Boolean);
  if (link) return { link };
  const gap = [raw.gap, raw.gap_id].map(entityId).find(Boolean);
  if (gap) return { section: "advanced", tab: "gaps", gap };
  const tab = advancedTab(raw.tab);
  if (raw.section === "advanced" || tab) {
    return { section: "advanced", ...(tab ? { tab } : {}) };
  }
  // The old matching tools' saved views now select suggestions on the page.
  const savedView = entityId(raw.savedView ?? raw.saved_view);
  return savedView ? { savedView } : {};
}

export function custodyGapRedirectSearch(raw: Record<string, unknown>): TransfersCustodySearch {
  const gap = [raw.gap, raw.gap_id].map(entityId).find(Boolean);
  return { section: "advanced", tab: "gaps", ...(gap ? { gap } : {}) };
}

/** The section the screen shows; the advanced tools fall back to the overview while gated. */
export function transfersCustodyView(raw: Record<string, unknown>, developerSurfacesEnabled: boolean): TransfersCustodyView {
  const search = parseTransfersCustodySearch(raw);
  const tab = search.tab ?? "gaps";
  if (search.section === "advanced") {
    if (!developerSurfacesEnabled) return { section: "overview", tab: "gaps" };
    return { section: "advanced", tab, ...(search.gap ? { gap: search.gap } : {}) };
  }
  return {
    section: "overview",
    tab: "gaps",
    ...(search.link ? { link: search.link } : {}),
    ...(search.savedView ? { savedView: search.savedView } : {}),
  };
}

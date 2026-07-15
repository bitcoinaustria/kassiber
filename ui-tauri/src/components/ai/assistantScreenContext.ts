import type {
  AssistantReturnPath,
  AssistantScreenContext,
  AssistantToolCapability,
} from "./assistantSession";

const TRANSACTION_TABS = new Set([
  "details",
  "classify",
  "pricing",
  "tax",
  "linked",
]);

const TRANSACTION_QUICK_FILTERS = new Set([
  "external_flow",
  "review_queue",
  "no_explorer_id",
  "missing_price",
  "failed_import",
]);

const EXIT_TAX_DESTINATIONS = new Set(["eu_eea", "third_country"]);

/**
 * Entity ids are opaque local/public identifiers, never paths or URLs. Keeping
 * this deliberately narrower than the daemon's bounded-string validation
 * prevents renderer-controlled navigation state from becoming an egress seam.
 */
const SAFE_ENTITY_ID = /^[A-Za-z0-9][A-Za-z0-9._:@+-]{0,255}$/;

function safeEntityId(value: string | null | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed && SAFE_ENTITY_ID.test(trimmed) ? trimmed : undefined;
}

function safePathSegment(value: string | undefined): string | undefined {
  if (!value) return undefined;
  try {
    return safeEntityId(decodeURIComponent(value));
  } catch {
    return undefined;
  }
}

function firstSafeEntity(
  params: URLSearchParams,
  keys: readonly string[],
): string | undefined {
  for (const key of keys) {
    const candidate = safeEntityId(params.get(key));
    if (candidate) return candidate;
  }
  return undefined;
}

function optionalFilters(
  entries: Array<[string, string | number | boolean | undefined]>,
): Record<string, unknown> | undefined {
  const filters = Object.fromEntries(
    entries.filter((entry): entry is [string, string | number | boolean] =>
      entry[1] !== undefined,
    ),
  );
  return Object.keys(filters).length > 0 ? filters : undefined;
}

function context(
  route: AssistantReturnPath,
  capabilities: AssistantToolCapability[],
  extra: Omit<AssistantScreenContext, "route" | "capabilities"> = {},
): AssistantScreenContext {
  return { route, capabilities, ...extra };
}

function normalizedPathname(pathname: string): string {
  if (!pathname.startsWith("/") || pathname.includes("\\")) return "/overview";
  const withoutTrailingSlash = pathname.replace(/\/+$/, "");
  return withoutTrailingSlash || "/overview";
}

/** Build the small, positive-allowlist UI context sent with one AI turn. */
export function assistantScreenContextFor(
  pathname: string,
  search = "",
): AssistantScreenContext {
  const path = normalizedPathname(pathname);
  const params = new URLSearchParams(search.startsWith("?") ? search : `?${search}`);

  if (path === "/transactions") {
    const entityId = firstSafeEntity(params, [
      "tx",
      "transaction",
      "transactionId",
    ]);
    const tab = params.get("tab") ?? undefined;
    const quick = params.get("quick") ?? undefined;
    return context("/transactions", ["transactions"], {
      ...(entityId
        ? { entityType: "transaction" as const, entityId }
        : {}),
      filters: optionalFilters([
        ["tab", tab && TRANSACTION_TABS.has(tab) ? tab : undefined],
        [
          "quick",
          quick && TRANSACTION_QUICK_FILTERS.has(quick) ? quick : undefined,
        ],
      ]),
    });
  }

  if (path === "/activity") {
    return context("/activity", ["transactions", "operations"]);
  }

  if (path === "/reports") {
    const year = Number(params.get("year"));
    return context("/reports", ["reports"], {
      filters: optionalFilters([
        [
          "year",
          Number.isInteger(year) && year >= 2009 && year <= 2100
            ? year
            : undefined,
        ],
      ]),
    });
  }

  if (path === "/privacy-mirror") {
    return context("/privacy-mirror", ["privacy"]);
  }

  if (path === "/egress") {
    return context("/egress", ["privacy"]);
  }

  if (path === "/exit-tax") {
    const destination = params.get("destination") ?? undefined;
    const departureDate = params.get("departure_date") ?? undefined;
    return context("/exit-tax", ["reports"], {
      entityType: "report",
      entityId: "exit-tax",
      filters: optionalFilters([
        [
          "destination",
          destination && EXIT_TAX_DESTINATIONS.has(destination)
            ? destination
            : undefined,
        ],
        [
          "departure_date",
          departureDate && /^\d{4}-\d{2}-\d{2}$/.test(departureDate)
            ? departureDate
            : undefined,
        ],
      ]),
    });
  }

  if (path === "/source-of-funds") {
    const transaction = firstSafeEntity(params, ["tx", "transaction"]);
    const sourceFundsCase = firstSafeEntity(params, ["case", "case_id"]);
    return context("/source-of-funds", ["source_funds", "transactions"], {
      ...(sourceFundsCase
        ? {
            entityType: "source_funds_case" as const,
            entityId: sourceFundsCase,
          }
        : transaction
          ? { entityType: "transaction" as const, entityId: transaction }
          : {}),
    });
  }

  if (path === "/journals") {
    const transaction = firstSafeEntity(params, ["tx", "transaction"]);
    return context("/journals", ["transactions", "reports", "operations"], {
      ...(transaction
        ? { entityType: "transaction" as const, entityId: transaction }
        : {}),
    });
  }

  if (path === "/quarantine") {
    return context("/quarantine", ["transactions", "reports", "operations"]);
  }

  if (path === "/swaps" || path === "/transfers") {
    const transaction = firstSafeEntity(params, ["focus", "tx", "transaction"]);
    const method = params.get("method");
    return context("/swaps", ["transfers", "transactions"], {
      ...(transaction
        ? { entityType: "transaction" as const, entityId: transaction }
        : {}),
      filters: optionalFilters([
        ["method", method === "ownership_graph" ? method : undefined],
      ]),
    });
  }

  if (path === "/custody-gaps") {
    const gap = firstSafeEntity(params, ["gap", "gap_id"]);
    return context("/custody-gaps", ["transfers", "transactions", "wallets"], {
      ...(gap ? { entityType: "custody_gap" as const, entityId: gap } : {}),
    });
  }

  if (path === "/reconcile") {
    return context("/reconcile", ["wallets", "transactions"]);
  }

  if (path === "/connections") {
    return context("/connections", ["wallets", "operations"]);
  }

  const connectionMatch = /^\/connections\/([^/]+)$/.exec(path);
  if (connectionMatch) {
    const entityId = safePathSegment(connectionMatch[1]);
    return context("/connections", ["wallets", "operations"], {
      ...(entityId ? { entityType: "connection" as const, entityId } : {}),
    });
  }

  if (path === "/imports" || path === "/Imports") {
    return context("/imports", ["wallets", "merchant", "transactions"]);
  }

  if (
    path === "/books" ||
    path === "/profiles" ||
    /^\/books\/[^/]+\/birds-eye$/.test(path)
  ) {
    return context("/books", [
      "wallets",
      "reports",
      "operations",
    ]);
  }

  if (path === "/logs" || path === "/diagnostics") {
    return context("/logs", ["operations"]);
  }

  if (path === "/settings") {
    return context("/settings", ["wallets", "privacy", "operations"]);
  }

  return context("/overview", [
    "transactions",
    "reports",
    "operations",
  ]);
}

/** Read live same-screen query state, but preserve the prior screen on /assistant. */
export function currentAssistantScreenContext(
  fallback: AssistantScreenContext,
): AssistantScreenContext {
  if (typeof window === "undefined" || window.location.pathname === "/assistant") {
    return fallback;
  }
  return assistantScreenContextFor(
    window.location.pathname,
    window.location.search,
  );
}

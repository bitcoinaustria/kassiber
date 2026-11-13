import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ActiveBook } from "@/lib/activeBook";
import {
  backgroundRunHasNewObservations,
  BURST_COOLDOWN_MS,
  candidateForOccurrence,
  configureOsNotifications,
  DEDUPE_WINDOW_MS,
  destinationForActivation,
  evidenceWatchCandidate,
  newActivityCandidate,
  offerOsNotification,
  osNotificationEligibility,
  randomToken,
  recordOsNotificationActivation,
  resetOsNotifications,
  resolveActivation,
  syncOsNotificationPreferences,
  TOKEN_TTL_MS,
  useOsNotificationActivation,
  type OsNotificationCandidate,
  type OsNotificationContext,
  type OsNotificationDeps,
} from "@/lib/osNotifications";
import {
  conditionKey,
  onNewOccurrence,
  resetNotificationStore,
  useNotificationStore,
  type ConditionInput,
  type NewOccurrence,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

const BOOK = '["Ws","Savings"]';
const OTHER_BOOK = '["Ws","Other"]';
const ACTIVE: ActiveBook = {
  key: BOOK,
  expected: { workspace_id: "Ws", profile_id: "Savings" },
};
const OTHER_ACTIVE: ActiveBook = {
  key: OTHER_BOOK,
  expected: { workspace_id: "Ws", profile_id: "Other" },
};
// Planted identifiers that must never cross the native boundary.
const SECRET_LABEL = "Cold storage bc1qplantedaddress";
const SECRET_ERROR =
  "connect ECONNREFUSED electrum.example.org:50002 for /Users/alice/wallet.json 0.42 BTC";

const failure: ConditionInput = {
  code: "source.refresh-failed",
  scope: { book: BOOK, source: "wallet:1" },
  severity: "error",
  priority: "attention",
  params: { sources: [SECRET_LABEL] },
  action: { type: "navigate", to: "/connections" },
  details: [{ label: SECRET_LABEL, message: SECRET_ERROR }],
};

const candidate: OsNotificationCandidate = {
  category: "needs-attention",
  dedupeKey: "condition:x",
  isNew: true,
  destination: { kind: "center" },
};

function context(overrides: Partial<OsNotificationContext> = {}): OsNotificationContext {
  return {
    enabled: true,
    available: true,
    windowFocused: false,
    now: 1_000_000,
    lastDeliveredAt: null,
    lastDeliveredByKey: new Map(),
    ...overrides,
  };
}

interface Harness {
  deps: OsNotificationDeps;
  invoke: ReturnType<typeof vi.fn>;
  clock: { now: number };
  focus: { value: boolean | null };
  enabled: { value: boolean };
  tokens: string[];
}

function harness(): Harness {
  const clock = { now: 1_000_000 };
  const focus: { value: boolean | null } = { value: false };
  const enabled = { value: true };
  const tokens: string[] = [];
  const invoke = vi.fn(async (command: string) =>
    command === "deliver_os_notification" ? "delivered" : null,
  );
  const deps: OsNotificationDeps = {
    enabled: () => enabled.value,
    available: () => true,
    windowFocused: async () => focus.value,
    invoke,
    now: () => clock.now,
    token: () => {
      const token = randomToken();
      tokens.push(token);
      return token;
    },
  };
  configureOsNotifications(deps);
  return { deps, invoke, clock, focus, enabled, tokens };
}

beforeEach(() => {
  resetNotificationStore();
  resetOsNotifications();
});

afterEach(() => {
  resetOsNotifications();
  vi.restoreAllMocks();
});

describe("osNotificationEligibility", () => {
  it.each([
    ["the setting is off", { enabled: false }, "disabled"],
    ["outside the native shell", { available: false }, "unavailable"],
    ["the window is focused", { windowFocused: true }, "focused"],
    ["focus is unknown", { windowFocused: null }, "focused"],
  ] as const)("refuses when %s", (_label, overrides, reason) => {
    expect(osNotificationEligibility(candidate, context(overrides))).toEqual({
      eligible: false,
      reason,
    });
  });

  it("delivers a new event while enabled and unfocused", () => {
    expect(osNotificationEligibility(candidate, context())).toEqual({
      eligible: true,
    });
  });

  it("refuses reconciled or repeated-before-seen events", () => {
    expect(
      osNotificationEligibility({ ...candidate, isNew: false }, context()),
    ).toEqual({ eligible: false, reason: "not-new" });
  });

  it("refuses a category outside the allowlist", () => {
    expect(
      osNotificationEligibility(
        { ...candidate, category: "receipt" as never },
        context(),
      ),
    ).toEqual({ eligible: false, reason: "category" });
  });

  it("dedupes the same event and cools down after any delivery", () => {
    const now = 10 * DEDUPE_WINDOW_MS;
    const byKey = new Map([[candidate.dedupeKey, now - DEDUPE_WINDOW_MS + 1]]);
    expect(
      osNotificationEligibility(
        candidate,
        context({ now, lastDeliveredByKey: byKey }),
      ),
    ).toEqual({ eligible: false, reason: "duplicate" });
    expect(
      osNotificationEligibility(
        candidate,
        context({ now, lastDeliveredByKey: new Map([[candidate.dedupeKey, now - DEDUPE_WINDOW_MS]]) }),
      ),
    ).toEqual({ eligible: true });
    expect(
      osNotificationEligibility(
        { ...candidate, dedupeKey: "other" },
        context({ now, lastDeliveredAt: now - BURST_COOLDOWN_MS + 1 }),
      ),
    ).toEqual({ eligible: false, reason: "cooldown" });
    expect(
      osNotificationEligibility(
        { ...candidate, dedupeKey: "other" },
        context({ now, lastDeliveredAt: now - BURST_COOLDOWN_MS }),
      ),
    ).toEqual({ eligible: true });
  });
});

describe("new occurrences", () => {
  function collect(): NewOccurrence[] {
    const seen: NewOccurrence[] = [];
    onNewOccurrence((occurrence) => seen.push(occurrence));
    return seen;
  }

  it("come only from raiseCondition and addResult, never a reconcile", () => {
    const seen = collect();
    const store = useNotificationStore.getState();
    store.reconcileConditions({ present: [failure] });
    store.reconcileConditions({ present: [failure] });
    expect(seen).toHaveLength(0);

    store.raiseCondition(failure);
    store.addResult({
      code: "book-refresh.finished",
      scope: { book: BOOK },
      severity: "info",
    });
    expect(seen.map((occurrence) => occurrence.lane)).toEqual([
      "condition",
      "result",
    ]);
  });

  it("are news for the OS only when new or already seen", () => {
    const seen = collect();
    const store = useNotificationStore.getState();
    store.raiseCondition(failure);
    store.raiseCondition(failure);
    useNotificationStore.getState().markSeen();
    useNotificationStore.getState().raiseCondition(failure);
    expect(seen.map((occurrence) => candidateForOccurrence(occurrence)?.isNew)).toEqual([
      true,
      false,
      true,
    ]);
  });

  it("map to allowlisted categories", () => {
    const seen = collect();
    const store = useNotificationStore.getState();
    store.raiseCondition(failure);
    store.addResult({ code: "journals.processed", scope: { book: BOOK }, severity: "info" });
    store.addResult({ code: "book-refresh.problems", scope: { book: BOOK }, severity: "warning" });
    expect(seen.map((occurrence) => candidateForOccurrence(occurrence)?.category)).toEqual([
      "needs-attention",
      "job-finished",
      "needs-attention",
    ]);
    // The live review backlog is reconciled, never raised; even if it were,
    // it is not a background outcome.
    store.raiseCondition({
      code: "review.pending",
      scope: { book: BOOK },
      severity: "warning",
      priority: "attention",
    });
    expect(candidateForOccurrence(seen[3])).toBeNull();
  });

  it("count only committed imports as new observations", () => {
    expect(backgroundRunHasNewObservations(null)).toBe(false);
    expect(
      backgroundRunHasNewObservations({
        completed: [
          { status: "done", result: { imported: 0, updated: 4 } },
          { status: "error", result: { imported: 3 } },
        ],
      }),
    ).toBe(false);
    expect(
      backgroundRunHasNewObservations({
        completed: [{ status: "done", result: { imported: 2 } }],
      }),
    ).toBe(true);
  });
});

describe("delivery", () => {
  it("sends only the category and an opaque token", async () => {
    const h = harness();
    const seen: NewOccurrence[] = [];
    onNewOccurrence((occurrence) => seen.push(occurrence));
    useNotificationStore.getState().raiseCondition(failure);

    await expect(offerOsNotification(candidateForOccurrence(seen[0]))).resolves.toEqual({
      eligible: true,
    });

    expect(h.invoke).toHaveBeenCalledTimes(1);
    const [command, args] = h.invoke.mock.calls[0];
    expect(command).toBe("deliver_os_notification");
    expect(Object.keys(args).sort()).toEqual(["category", "token"]);
    expect(args.category).toBe("needs-attention");
    expect(args.token).toMatch(/^[0-9a-f]{32}$/);
    const wire = JSON.stringify(h.invoke.mock.calls);
    for (const planted of [SECRET_LABEL, SECRET_ERROR, BOOK, "wallet:1", "/connections"]) {
      expect(wire).not.toContain(planted);
    }
  });

  it("does not touch the native side while the setting is off", async () => {
    const h = harness();
    h.enabled.value = false;
    await expect(offerOsNotification(candidate)).resolves.toEqual({
      eligible: false,
      reason: "disabled",
    });
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("does not deliver while the native window is focused", async () => {
    const h = harness();
    h.focus.value = true;
    await expect(offerOsNotification(candidate)).resolves.toEqual({
      eligible: false,
      reason: "focused",
    });
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("lets one notification through a burst", async () => {
    const h = harness();
    const results = await Promise.all(
      ["a", "b", "c"].map((key) =>
        offerOsNotification({ ...candidate, dedupeKey: key }),
      ),
    );
    expect(results.map((result) => result.eligible)).toEqual([true, false, false]);
    expect(h.invoke).toHaveBeenCalledTimes(1);

    h.clock.now += BURST_COOLDOWN_MS;
    await expect(
      offerOsNotification({ ...candidate, dedupeKey: "d" }),
    ).resolves.toEqual({ eligible: true });
  });

  it("records nothing when the native side refuses", async () => {
    const h = harness();
    h.invoke.mockResolvedValueOnce("cooldown");
    await expect(offerOsNotification(candidate)).resolves.toEqual({
      eligible: false,
      reason: "cooldown",
    });
    // Not recorded, so the next offer is not held back by the TS cooldown.
    await expect(offerOsNotification(candidate)).resolves.toEqual({ eligible: true });
  });

  it("mirrors the opt-in and language with enum values only", async () => {
    const h = harness();
    await syncOsNotificationPreferences(true, "de");
    await syncOsNotificationPreferences(false, "en");
    expect(h.invoke.mock.calls).toEqual([
      ["set_os_notification_preferences", { enabled: true, language: "de" }],
      ["set_os_notification_preferences", { enabled: false, language: "en" }],
    ]);
  });

  it("makes no daemon or network call when enabled and fed events", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const h = harness();
    // The real opt-in, as the shell wires it.
    configureOsNotifications({
      ...h.deps,
      enabled: () => useUiStore.getState().desktopNotifications,
    });
    useUiStore.getState().setDesktopNotifications(true);
    const unsubscribe = onNewOccurrence((occurrence) => {
      void offerOsNotification(candidateForOccurrence(occurrence));
    });
    try {
      const store = useNotificationStore.getState();
      store.raiseCondition(failure);
      store.raiseCondition({ ...failure, code: "freshness.worker-error" });
      store.addResult({ code: "book-refresh.finished", scope: { book: BOOK }, severity: "info" });
      store.reconcileConditions({ present: [failure] });
      await offerOsNotification(evidenceWatchCandidate(BOOK));
      h.clock.now += BURST_COOLDOWN_MS;
      await offerOsNotification(newActivityCandidate(BOOK));
    } finally {
      unsubscribe();
      useUiStore.getState().setDesktopNotifications(false);
    }

    expect(fetchSpy).not.toHaveBeenCalled();
    const commands = new Set(h.invoke.mock.calls.map(([command]) => command));
    expect([...commands]).toEqual(["deliver_os_notification"]);
    expect(h.invoke).toHaveBeenCalledTimes(2);
  });
});

describe("activation", () => {
  const conditionRef = conditionKey(failure.code, failure.scope);

  async function deliverFailure(h: Harness): Promise<string> {
    const seen: NewOccurrence[] = [];
    const unsubscribe = onNewOccurrence((occurrence) => seen.push(occurrence));
    useNotificationStore.getState().raiseCondition(failure);
    unsubscribe();
    await offerOsNotification(candidateForOccurrence(seen[0]));
    return h.tokens.at(-1)!;
  }

  function routeFor(currentBook: ActiveBook | null) {
    const pending = useOsNotificationActivation.getState().pending!;
    const { conditions, results } = useNotificationStore.getState();
    return resolveActivation(pending, {
      currentBook,
      developerToolsEnabled: false,
      conditions,
      results,
    });
  }

  it("opens the item when its token is fresh and its book is open", async () => {
    const h = harness();
    const token = await deliverFailure(h);
    recordOsNotificationActivation(token, h.clock.now + 1000);
    expect(routeFor(ACTIVE)).toEqual({ kind: "navigate", to: "/connections" });
  });

  it("opens the center for another book, a resolved entry, or a second use", async () => {
    const h = harness();
    const token = await deliverFailure(h);
    recordOsNotificationActivation(token, h.clock.now);
    expect(routeFor(OTHER_ACTIVE)).toEqual({ kind: "center" });
    expect(routeFor(ACTIVE)).toEqual({ kind: "navigate", to: "/connections" });

    useNotificationStore.getState().resolveCondition(conditionRef);
    expect(routeFor(ACTIVE)).toEqual({ kind: "center" });

    // Consumed: the same token again is unknown.
    expect(destinationForActivation(token, h.clock.now)).toEqual({ kind: "center" });
  });

  it("falls back to the center when the token expired or the app restarted", async () => {
    const h = harness();
    const token = await deliverFailure(h);
    expect(destinationForActivation(token, h.clock.now + TOKEN_TTL_MS + 1)).toEqual({
      kind: "center",
    });
    expect(destinationForActivation(randomToken())).toEqual({ kind: "center" });
    expect(destinationForActivation(42)).toEqual({ kind: "center" });
  });

  it("never repeats an operation; it opens the operation's review page", () => {
    const route = resolveActivation(
      {
        kind: "entry",
        scope: { book: BOOK },
        action: {
          type: "operation",
          operation: { kind: "refresh-book", forceFull: true },
          review: "/connections",
        },
      },
      { currentBook: ACTIVE, developerToolsEnabled: false, conditions: {}, results: [] },
    );
    expect(route).toEqual({ kind: "navigate", to: "/connections" });
  });

  it("refuses routes outside the allowlist", () => {
    const route = resolveActivation(
      {
        kind: "entry",
        scope: { book: BOOK },
        action: { type: "navigate", to: "https://example.org" },
      },
      { currentBook: ACTIVE, developerToolsEnabled: false, conditions: {}, results: [] },
    );
    expect(route).toEqual({ kind: "center" });
  });
});

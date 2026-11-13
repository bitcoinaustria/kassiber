import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { bookFreshnessState, type BookFreshnessInputs } from "@/lib/bookFreshness";
import { BookFreshnessView } from "./BookFreshnessIndicator";

const IDLE: BookFreshnessInputs = {
  syncing: false,
  processingJournals: false,
  loadingChanges: false,
  refreshFailed: false,
  needsJournals: false,
  lastChange: null,
};

describe("book freshness state", () => {
  it("names the most urgent state", () => {
    expect(bookFreshnessState(IDLE)).toEqual({ kind: "current", change: null });
    expect(
      bookFreshnessState({ ...IDLE, needsJournals: true, refreshFailed: true }),
    ).toEqual({ kind: "failed" });
    expect(bookFreshnessState({ ...IDLE, needsJournals: true })).toEqual({
      kind: "needs-journals",
    });
    // A running update outranks an earlier failure: it may clear it.
    expect(
      bookFreshnessState({ ...IDLE, refreshFailed: true, loadingChanges: true }),
    ).toEqual({ kind: "updating", reason: "loading" });
    expect(
      bookFreshnessState({ ...IDLE, syncing: true, processingJournals: true }),
    ).toEqual({ kind: "updating", reason: "syncing" });
  });

  it("does not claim the book is current while a change summary is stale", () => {
    const change = { transactionDelta: 2, balanceDeltaSat: 5_000, at: 1 };
    expect(
      bookFreshnessState({ ...IDLE, lastChange: change, needsJournals: true }),
    ).toEqual({ kind: "needs-journals" });
    expect(bookFreshnessState({ ...IDLE, lastChange: change })).toEqual({
      kind: "current",
      change,
    });
  });
});

describe("book freshness view", () => {
  it("says what the last refresh changed, as a status", () => {
    const html = renderToStaticMarkup(
      <BookFreshnessView
        state={{
          kind: "current",
          change: { transactionDelta: 3, balanceDeltaSat: 1_200_000, at: 1 },
        }}
        hideSensitive={false}
      />,
    );
    expect(html).toContain('role="status"');
    expect(html).toContain("Up to date");
    expect(html).toContain("3 new transactions");
    expect(html).toContain("+ ₿ 0.01200000");
  });

  it("hides the balance move in privacy mode", () => {
    const html = renderToStaticMarkup(
      <BookFreshnessView
        state={{
          kind: "current",
          change: { transactionDelta: -1, balanceDeltaSat: -500, at: 1 },
        }}
        hideSensitive
      />,
    );
    expect(html).toContain("1 transaction removed");
    // Masked, not only blurred: the status region is read aloud.
    expect(html).not.toContain("0.00000500");
    expect(html).toContain('class="sensitive">••••');
  });

  it("names why the book is updating", () => {
    const html = renderToStaticMarkup(
      <BookFreshnessView
        state={{ kind: "updating", reason: "processing" }}
        hideSensitive={false}
      />,
    );
    expect(html).toContain("Processing journals");
    expect(html).toContain('data-state="updating"');
  });
});

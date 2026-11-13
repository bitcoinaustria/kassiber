import { beforeEach, describe, expect, it } from "vitest";

import { resolveEntryAction } from "@/components/kb/notificationRouting";
import type { ActiveBook } from "@/lib/activeBook";
import { useFeedbackStore } from "@/lib/feedback";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";

import {
  endWorkspaceRefresh,
  recordWorkspaceRefreshed,
  recordWorkspaceRefreshFailed,
  startWorkspaceRefresh,
} from "./workspaceNotifications";

const ANY_BOOK: ActiveBook = {
  key: "any-book",
  expected: { workspace_id: "ws", profile_id: "pf-any" },
};

beforeEach(() => {
  resetNotificationStore();
  useFeedbackStore.setState({ items: [] });
});

const clean = { books: 3, synced_books: 3, errors: 0, reports_blocked: 0 };

describe("refreshing all books", () => {
  it("keeps overlapping refreshes apart", () => {
    const first = startWorkspaceRefresh("ws-1");
    const second = startWorkspaceRefresh("ws-1");
    expect(Object.keys(useNotificationStore.getState().jobs)).toHaveLength(2);
    endWorkspaceRefresh(first);
    expect(Object.keys(useNotificationStore.getState().jobs)).toEqual([second]);
    endWorkspaceRefresh(second);
  });

  it("opens that workspace's overview of all books from any open book", () => {
    const job = startWorkspaceRefresh("ws-1");
    recordWorkspaceRefreshed("ws-1", { ...clean, synced_books: 2, reports_blocked: 1 });
    endWorkspaceRefresh(job);
    const { conditions, jobs } = useNotificationStore.getState();
    expect(jobs).toEqual({});
    const [condition] = Object.values(conditions);
    expect(condition).toMatchObject({
      code: "workspace.books-need-attention",
      priority: "blocking",
      scope: { book: null, source: "workspace:ws-1" },
    });
    expect(resolveEntryAction(condition.action, condition.scope, ANY_BOOK, false)).toEqual({
      kind: "open-workspace",
      workspaceId: "ws-1",
    });
  });

  it("clears a failure and the problems once a refresh comes back clean", () => {
    recordWorkspaceRefreshFailed("ws-1", "daemon restarted");
    recordWorkspaceRefreshed("ws-1", { ...clean, errors: 1 });
    expect(Object.values(useNotificationStore.getState().conditions).map((entry) => entry.code)).toEqual([
      "workspace.books-need-attention",
    ]);
    recordWorkspaceRefreshed("ws-1", clean);
    expect(useNotificationStore.getState().conditions).toEqual({});
    expect(useNotificationStore.getState().results).toMatchObject([
      { code: "workspace.refreshed", severity: "info", params: { count: 3, entries: 3 } },
    ]);
  });
});

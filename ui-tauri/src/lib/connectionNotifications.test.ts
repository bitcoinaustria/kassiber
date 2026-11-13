import { beforeEach, describe, expect, it } from "vitest";

import { endConnectionSync, startConnectionSync } from "@/lib/connectionNotifications";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";

beforeEach(() => resetNotificationStore());

describe("connection sync jobs", () => {
  it("keeps overlapping syncs of one connection apart", () => {
    const ref = { book: { book: "book-a" }, connectionId: "w-1", label: "Cold" };
    const first = startConnectionSync(ref);
    const second = startConnectionSync(ref);
    expect(first).not.toBe(second);
    expect(Object.keys(useNotificationStore.getState().jobs)).toHaveLength(2);
    endConnectionSync(first);
    expect(Object.keys(useNotificationStore.getState().jobs)).toEqual([second]);
  });
});

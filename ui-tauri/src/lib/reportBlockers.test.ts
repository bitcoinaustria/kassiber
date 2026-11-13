import { beforeEach, describe, expect, it } from "vitest";

import { blockingSourceCount } from "@/lib/reportBlockers";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";

const BOOK = "book-a";

beforeEach(() => resetNotificationStore());

describe("sources blocking reports", () => {
  it("counts the open book's blocking source failures, as the bell shows them", () => {
    const store = useNotificationStore.getState();
    const failure = (book: string, source: string, priority: "blocking" | "attention") =>
      store.raiseCondition({
        code: "source.refresh-failed",
        scope: { book, source },
        severity: "error",
        priority,
      });
    failure(BOOK, "s1", "blocking");
    failure(BOOK, "s2", "blocking");
    failure(BOOK, "s3", "attention");
    failure("book-b", "s1", "blocking");
    const { conditions } = useNotificationStore.getState();
    expect(blockingSourceCount(conditions, BOOK)).toBe(2);
    expect(blockingSourceCount(conditions, "book-b")).toBe(1);
    expect(blockingSourceCount(conditions, null)).toBe(0);
  });
});

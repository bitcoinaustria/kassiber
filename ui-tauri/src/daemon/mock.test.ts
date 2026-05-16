import { describe, expect, it } from "vitest";

import { mockDaemon } from "./mock";

describe("mock daemon backend settings", () => {
  it("supports settings list and CRUD for demo mode", async () => {
    const list = await mockDaemon.invoke<{
      backends: Array<{ name: string }>;
    }>({ kind: "ui.backends.settings.list" });
    expect(list.data?.backends.length).toBeGreaterThan(0);

    const created = await mockDaemon.invoke<{ name: string }>({
      kind: "ui.backends.create",
      args: {
        name: "mock-extra",
        kind: "esplora",
        chain: "bitcoin",
        network: "main",
        url: "https://example.invalid/api",
        auth_header: "Bearer demo",
      },
    });
    expect(created.error).toBeUndefined();
    expect(created.data?.name).toBe("mock-extra");

    const updated = await mockDaemon.invoke<{
      has_auth_header?: boolean;
      has_username?: boolean;
    }>({
      kind: "ui.backends.update",
      args: {
        name: "mock-extra",
        config: { username: "demo" },
        clear: ["auth_header"],
      },
    });
    expect(updated.data?.has_auth_header).toBe(false);
    expect(updated.data?.has_username).toBe(true);

    const deleted = await mockDaemon.invoke<{ deleted: boolean }>({
      kind: "ui.backends.delete",
      args: { name: "mock-extra" },
    });
    expect(deleted.data?.deleted).toBe(true);
  });
});

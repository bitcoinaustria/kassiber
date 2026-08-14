import { createMemoryHistory } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";

import { router } from "./routeTree";

describe("imports route", () => {
  it.each(["/imports", "/Imports"])("mounts the imports page for %s", (path) => {
    router.update({ history: createMemoryHistory({ initialEntries: [path] }) });
    expect(router.matchRoutes(path).at(-1)?.fullPath).toBe("/imports");
  });
});

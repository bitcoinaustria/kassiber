import { describe, expect, it } from "vitest";
import { sourceFundsDraftKey, targetQueryArgs } from "./caseScope";

const filters = { query: "", flow: "all", date: "all", status: "all", network: "all", asset: "all", wallet: "all" };
describe("source-funds scope and transaction paging", () => {
  it("separates profiles in one imported database and equal labels in different databases", () => {
    const scope = { workspace_id: "workspace", profile_id: "profile" };
    const key = sourceFundsDraftKey("db:/one.sqlite", scope);
    expect(sourceFundsDraftKey("db:/one.sqlite", { ...scope, profile_id: "second" })).not.toBe(key);
    expect(sourceFundsDraftKey("db:/two.sqlite", scope)).not.toBe(key);
    expect(sourceFundsDraftKey("db:/one.sqlite", { ...scope, workspace_id: "other" })).not.toBe(key);
    expect(sourceFundsDraftKey("db:/one.sqlite", scope)).toBe(key);
  });
  it("pushes searches and all filters to the paginated daemon contract", () => {
    expect(targetQueryArgs({ ...filters, query: " old-tx ", flow: "transfer", status: "review", network: "liquid", asset: "LBTC", wallet: "Cold" })).toEqual({
      limit: 100, sort: "occurred-at", order: "desc", query: "old-tx", flow: "transfer", status: "review", network: "liquid", asset: "LBTC", wallet: "Cold",
    });
    expect(targetQueryArgs(filters)).toEqual({ limit: 100, sort: "occurred-at", order: "desc" });
  });
  it("bounds date filters without filtering away results on later pages", () => {
    const now = new Date("2026-09-05T12:00:00Z");
    const yesterday = targetQueryArgs({ ...filters, date: "yesterday" }, now);
    expect(Date.parse(yesterday.until as string) - Date.parse(yesterday.since as string)).toBe(86400000 - 1);
    expect(targetQueryArgs({ ...filters, date: "older" }, now)).not.toHaveProperty("since");
    expect(targetQueryArgs({ ...filters, date: "older" }, now)).toHaveProperty("until");
  });
});

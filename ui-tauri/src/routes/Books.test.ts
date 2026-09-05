import { describe, expect, it } from "vitest";

import type { Profile } from "@/mocks/profiles";
import {
  poolScopesForProfileEdit,
  profileSettingsUpdatePayload,
} from "@/routes/books-profile-settings";

const profile: Profile = {
  id: "profile-at",
  name: "Austria",
  taxPolicy: "Austria",
  taxCountry: "at",
  gainsAlgorithm: "MOVING_AVERAGE_AT",
  costBasisPoolScope: "global",
  allowedCostBasisPoolScopes: ["global", "wallet"],
  accounts: 1,
  wallets: 2,
  lastOpened: "now",
};

describe("Books cost-basis pool settings", () => {
  it("uses policy-filtered scopes and locks a country switch to global", () => {
    expect(poolScopesForProfileEdit(profile, "at")).toEqual([
      "global",
      "wallet",
    ]);
    expect(poolScopesForProfileEdit(profile, "generic")).toEqual(["global"]);
  });

  it("sends an exact scope update and includes the reset on country switch", () => {
    expect(
      profileSettingsUpdatePayload(
        profile,
        "at",
        "MOVING_AVERAGE_AT",
        "wallet",
      ),
    ).toEqual({
      profile_id: "profile-at",
      cost_basis_pool_scope: "wallet",
    });
    expect(
      profileSettingsUpdatePayload(profile, "generic", "FIFO", "global"),
    ).toEqual({
      profile_id: "profile-at",
      gains_algorithm: "FIFO",
      tax_country: "generic",
      cost_basis_pool_scope: "global",
    });
  });
});

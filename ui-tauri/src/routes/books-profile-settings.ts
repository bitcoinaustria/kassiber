import type { TaxCountry } from "@/components/kb/Onboarding/types";
import type { CostBasisPoolScope, Profile } from "@/mocks/profiles";

interface ProfileSettingsUpdatePayload extends Record<string, unknown> {
  profile_id: string;
  gains_algorithm?: string;
  tax_country?: TaxCountry;
  cost_basis_pool_scope?: CostBasisPoolScope;
}

export const poolScopesForProfileEdit = (
  profile: Profile,
  country: TaxCountry,
): CostBasisPoolScope[] =>
  country === (profile.taxCountry ?? "generic")
    ? (profile.allowedCostBasisPoolScopes ?? ["global"])
    : ["global"];

export const profileSettingsUpdatePayload = (
  profile: Profile,
  country: TaxCountry,
  method: string,
  poolScope: CostBasisPoolScope,
): ProfileSettingsUpdatePayload | null => {
  const originalCountry = profile.taxCountry ?? "generic";
  if (country !== originalCountry) {
    return {
      profile_id: profile.id,
      gains_algorithm: method,
      tax_country: country,
      cost_basis_pool_scope: poolScope,
    };
  }

  const payload: ProfileSettingsUpdatePayload = { profile_id: profile.id };
  if (method !== (profile.gainsAlgorithm ?? "")) {
    payload.gains_algorithm = method;
  }
  if (poolScope !== (profile.costBasisPoolScope ?? "global")) {
    payload.cost_basis_pool_scope = poolScope;
  }
  return Object.keys(payload).length > 1 ? payload : null;
};

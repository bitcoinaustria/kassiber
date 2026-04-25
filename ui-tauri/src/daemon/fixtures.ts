/**
 * Hand-rolled mock fixtures, keyed by daemon `kind`.
 *
 * Each entry is the `data` body of a successful envelope. Kept minimal
 * until Phase 1.2 §2.2 generates real fixtures from the Pydantic schema.
 * Add entries as screens get translated.
 */

export const fixtures: Record<string, unknown> = {
  status: {
    version: "0.0.0-ui-scaffold",
    data_root: "~/.kassiber",
    workspace: null,
    profile: null,
  },
};

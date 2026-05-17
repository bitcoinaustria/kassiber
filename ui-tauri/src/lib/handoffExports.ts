/**
 * Informational taxonomy for Reports UI framing. These labels describe the
 * intended handoff boundaries; runtime package/export gating is tracked in
 * TODO.md until scoped audit-package exports exist.
 */
export type HandoffExportModeId =
  | "tax_advisor_report"
  | "audit_package"
  | "technical_wallet_evidence";

export type HandoffExportAvailability = "available" | "planned" | "restricted";

export type HandoffWalletMaterialPolicy =
  | "excluded"
  | "requires_explicit_consent";

export interface HandoffExportMode {
  id: HandoffExportModeId;
  title: string;
  sensitivity: "External" | "Trusted" | "Restricted";
  availability: HandoffExportAvailability;
  summary: string;
  includes: string[];
  excludes: string[];
  walletMaterialPolicy: HandoffWalletMaterialPolicy;
}

export const SENSITIVE_WALLET_MATERIAL = [
  "wallet descriptors",
  "xpubs",
  "backend credentials",
  "raw wallet config",
] as const;

export const NORMAL_HANDOFF_EXCLUSIONS = [
  ...SENSITIVE_WALLET_MATERIAL,
  "AI settings",
  "logs",
  "unrelated books",
] as const;

export const HANDOFF_EXPORT_MODES: readonly HandoffExportMode[] = [
  {
    id: "tax_advisor_report",
    title: "Tax advisor report",
    sensitivity: "External",
    availability: "available",
    summary: "Default handoff for filing and ordinary tax review.",
    includes: ["PDF, XLSX, and CSV report output", "Tax fields", "Reviewed report rows"],
    excludes: [...NORMAL_HANDOFF_EXCLUSIONS],
    walletMaterialPolicy: "excluded",
  },
  {
    id: "audit_package",
    title: "Audit package",
    sensitivity: "Trusted",
    availability: "planned",
    summary: "Book-scoped evidence package for internal or trusted audit review.",
    includes: [
      "Selected book transactions",
      "Journals and review state",
      "Chosen evidence attachments",
    ],
    excludes: [...NORMAL_HANDOFF_EXCLUSIONS],
    walletMaterialPolicy: "excluded",
  },
  {
    id: "technical_wallet_evidence",
    title: "Technical wallet evidence",
    sensitivity: "Restricted",
    availability: "restricted",
    summary: "Separate custody-verification material for select eyes only.",
    includes: ["Wallet completeness evidence only after explicit approval"],
    excludes: ["Normal tax and audit exports"],
    walletMaterialPolicy: "requires_explicit_consent",
  },
] as const;

export function handoffModeById(
  id: HandoffExportModeId,
): HandoffExportMode | undefined {
  return HANDOFF_EXPORT_MODES.find((mode) => mode.id === id);
}

export function requiresSensitiveWalletMaterialConsent(
  mode: HandoffExportMode,
): boolean {
  return mode.walletMaterialPolicy === "requires_explicit_consent";
}

export function normalHandoffModes(): HandoffExportMode[] {
  return HANDOFF_EXPORT_MODES.filter(
    (mode) => mode.walletMaterialPolicy === "excluded",
  );
}
